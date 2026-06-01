//! **Sprint 8 — N11 days 11-12** : cgroup v2 writer.
//!
//! Creates and tears down a per-VM cgroup v2 directory under
//! `cgroup_root/vm-{id}/`, writes the CPU / memory / IO limits, and
//! moves the Firecracker process PID into it.
//!
//! # Why cgroup v2 (not v1)
//!
//! - **Unified hierarchy** : one mount, one tree.  v1's "per-controller
//!   hierarchy" is deprecated.
//! - **`memory.high`** : soft limit + reclaim.  v1 has no analogue.
//! - **Better OOM semantics** : v2's `memory.events` exposes precise
//!   counters (low, high, max, oom, oom_kill).
//! - **Pressure stall info** : v2 surfaces `cpu.pressure`, `io.pressure`,
//!   `memory.pressure` for advanced telemetry (Phase 4 wiring).
//!
//! Kernel requirement : ≥ 5.10.  Amazon Linux 2023, Ubuntu 22.04+,
//! RHEL 9+, Debian 12+ all ship cgroup v2 by default.
//!
//! # Mount layout
//!
//! ```text
//! /sys/fs/cgroup/                          ← cgroup v2 root
//!   ├── cgroup.subtree_control             ← must contain "+cpu +memory +io"
//!   ├── efe-nano/                          ← parent for our pool
//!   │   ├── cgroup.subtree_control         ← same flags
//!   │   ├── vm-1/                          ← one per VM
//!   │   │   ├── cgroup.procs               ← write Firecracker PID here
//!   │   │   ├── cpu.max                    ← "<quota> <period>"
//!   │   │   ├── memory.max                 ← bytes
//!   │   │   ├── memory.high                ← bytes
//!   │   │   └── io.weight                  ← "default <N>"
//!   │   └── vm-2/
//!   └── ...
//! ```

#![cfg(target_os = "linux")]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

use super::config::KindConfig;
use super::types::VmId;

#[derive(Debug, Error)]
pub enum CgroupError {
    #[error("cgroup root not accessible : {0}")]
    RootInaccessible(String),

    #[error("failed to create cgroup directory {path} : {reason}")]
    MkdirFailed { path: PathBuf, reason: String },

    #[error("failed to write {file} : {reason}")]
    WriteFailed { file: PathBuf, reason: String },

    #[error("subtree_control missing required controllers : {0}")]
    SubtreeControlMissing(String),

    #[error("failed to remove cgroup {path} : {reason}")]
    RemoveFailed { path: PathBuf, reason: String },
}

// ─────────────────────────────────────────────────────────────────────
//  CgroupController
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N11** : per-pool cgroup controller.
///
/// Operates on `<cgroup_root>/efe-nano/` and creates `vm-{id}/`
/// subdirectories for each VM.  Each VM cgroup gets cpu / memory / io
/// limits + the Firecracker PID.
#[derive(Debug)]
pub struct CgroupController {
    /// Absolute path to the parent cgroup (e.g.
    /// `/sys/fs/cgroup/efe-nano`).  Must exist + have `+cpu +memory +io`
    /// in `cgroup.subtree_control` before `new()` returns Ok.
    parent: PathBuf,
}

impl CgroupController {
    /// Construct.  Verifies that the parent cgroup exists and that
    /// the required controllers are delegated to it via
    /// `cgroup.subtree_control`.  Use [`Self::ensure_parent`] to
    /// auto-create the parent (requires CAP_SYS_ADMIN).
    pub fn new(parent: impl Into<PathBuf>) -> Result<Self, CgroupError> {
        let parent = parent.into();
        if !parent.exists() {
            return Err(CgroupError::RootInaccessible(format!(
                "parent cgroup {} does not exist",
                parent.display()
            )));
        }
        let subtree_path = parent.join("cgroup.subtree_control");
        let contents = fs::read_to_string(&subtree_path).map_err(|e| {
            CgroupError::RootInaccessible(format!(
                "cannot read {} : {e}",
                subtree_path.display()
            ))
        })?;
        for required in ["cpu", "memory", "io"] {
            if !contents.contains(required) {
                return Err(CgroupError::SubtreeControlMissing(format!(
                    "{} ({} missing from {})",
                    required,
                    required,
                    subtree_path.display()
                )));
            }
        }
        Ok(Self { parent })
    }

    /// Create the parent cgroup if it doesn't exist and write the
    /// required controllers to its `subtree_control`.  Requires
    /// `CAP_SYS_ADMIN` on the EFE Nano process.
    pub fn ensure_parent(
        cgroup_root: impl AsRef<Path>,
        name: &str,
    ) -> Result<PathBuf, CgroupError> {
        let parent = cgroup_root.as_ref().join(name);
        if !parent.exists() {
            fs::create_dir(&parent).map_err(|e| CgroupError::MkdirFailed {
                path: parent.clone(),
                reason: e.to_string(),
            })?;
        }
        // Idempotent write : "+cpu +memory +io".  The kernel accepts
        // multiple writes ; controllers are added incrementally.
        let subtree = parent.join("cgroup.subtree_control");
        write_cgroup_file(&subtree, "+cpu +memory +io")?;
        Ok(parent)
    }

    /// Create a per-VM cgroup at `parent/vm-{id}/` and apply the
    /// limits from `kind_config`.  Returns the cgroup path so the
    /// caller can move the Firecracker PID into it via
    /// [`Self::add_pid`].
    pub fn create_vm_cgroup(
        &self,
        vm_id: VmId,
        kind_config: &KindConfig,
    ) -> Result<PathBuf, CgroupError> {
        let path = self.parent.join(format!("vm-{}", vm_id.raw()));
        if !path.exists() {
            fs::create_dir(&path).map_err(|e| CgroupError::MkdirFailed {
                path: path.clone(),
                reason: e.to_string(),
            })?;
        }

        // cpu.max format : "<quota> <period>".
        // Period is fixed at 100_000 µs (100 ms) ; quota is the
        // configured number.
        let cpu_max = path.join("cpu.max");
        write_cgroup_file(
            &cpu_max,
            &format!("{} 100000", kind_config.cgroup_cpu_quota_us),
        )?;

        // memory.high (soft) and memory.max (hard, OOM kill).
        let memory_high = path.join("memory.high");
        write_cgroup_file(&memory_high, &kind_config.cgroup_memory_high.to_string())?;
        let memory_max = path.join("memory.max");
        write_cgroup_file(&memory_max, &kind_config.cgroup_memory_max.to_string())?;

        // io.weight : "default <N>" where N is 1..1000.
        let io_weight = path.join("io.weight");
        write_cgroup_file(
            &io_weight,
            &format!("default {}", kind_config.cgroup_blkio_weight),
        )?;

        Ok(path)
    }

    /// Move a PID into the cgroup.  Must be called AFTER fork+exec
    /// (Firecracker is now running) and BEFORE the inference daemon
    /// performs any heavy memory allocation — otherwise the OOM cap
    /// won't apply retroactively.
    pub fn add_pid(cgroup_path: &Path, pid: u32) -> Result<(), CgroupError> {
        let procs = cgroup_path.join("cgroup.procs");
        write_cgroup_file(&procs, &pid.to_string())
    }

    /// Read `memory.events` for OOM event counters.  Returns the parsed
    /// (low, high, max, oom, oom_kill) tuple.  Used by the reaper to
    /// classify VM deaths as "OOM" vs "other".
    pub fn read_memory_events(
        cgroup_path: &Path,
    ) -> Result<MemoryEvents, CgroupError> {
        let events_path = cgroup_path.join("memory.events");
        let contents = fs::read_to_string(&events_path).map_err(|e| {
            CgroupError::WriteFailed {
                file: events_path.clone(),
                reason: format!("read failed : {e}"),
            }
        })?;
        let mut events = MemoryEvents::default();
        for line in contents.lines() {
            let mut parts = line.split_whitespace();
            let key = parts.next().unwrap_or("");
            let value: u64 = parts.next().unwrap_or("0").parse().unwrap_or(0);
            match key {
                "low" => events.low = value,
                "high" => events.high = value,
                "max" => events.max = value,
                "oom" => events.oom = value,
                "oom_kill" => events.oom_kill = value,
                _ => {}
            }
        }
        Ok(events)
    }

    /// Remove a VM cgroup.  Must be called AFTER the Firecracker
    /// process has exited (otherwise rmdir returns EBUSY).
    pub fn remove_vm_cgroup(&self, vm_id: VmId) -> Result<(), CgroupError> {
        let path = self.parent.join(format!("vm-{}", vm_id.raw()));
        if !path.exists() {
            return Ok(());
        }
        fs::remove_dir(&path).map_err(|e| CgroupError::RemoveFailed {
            path,
            reason: e.to_string(),
        })?;
        Ok(())
    }

    pub fn parent_path(&self) -> &Path {
        &self.parent
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MemoryEvents {
    pub low: u64,
    pub high: u64,
    pub max: u64,
    pub oom: u64,
    pub oom_kill: u64,
}

impl MemoryEvents {
    /// True if this cgroup has experienced an OOM kill.  Reaper uses
    /// this to classify VM deaths.
    pub fn experienced_oom(&self) -> bool {
        self.oom > 0 || self.oom_kill > 0
    }
}

/// Write a value to a cgroup pseudo-file.  Most cgroup writes are
/// "write the whole file" semantics ; the kernel parses each write
/// atomically.
fn write_cgroup_file(path: &Path, value: &str) -> Result<(), CgroupError> {
    let mut f = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(|e| CgroupError::WriteFailed {
            file: path.to_path_buf(),
            reason: format!("open failed : {e}"),
        })?;
    f.write_all(value.as_bytes())
        .map_err(|e| CgroupError::WriteFailed {
            file: path.to_path_buf(),
            reason: format!("write failed : {e}"),
        })?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────
//  Tests — only the parsing logic is testable without root
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn memory_events_default_is_zero() {
        let m = MemoryEvents::default();
        assert!(!m.experienced_oom());
        assert_eq!(m.oom, 0);
    }

    #[test]
    fn memory_events_experienced_oom() {
        let m = MemoryEvents {
            oom: 1,
            ..Default::default()
        };
        assert!(m.experienced_oom());

        let m = MemoryEvents {
            oom_kill: 3,
            ..Default::default()
        };
        assert!(m.experienced_oom());
    }

    /// Parse `memory.events` format from a tempfile.  This exercises
    /// the file-parsing path without needing real cgroup access.
    #[test]
    fn read_memory_events_parses_correctly() {
        let tmpdir = std::env::temp_dir().join(format!(
            "efe-nano-test-cgroup-{}",
            std::process::id()
        ));
        let _ = fs::create_dir(&tmpdir);
        let events = tmpdir.join("memory.events");
        let mut f = fs::File::create(&events).unwrap();
        f.write_all(b"low 0\nhigh 5\nmax 0\noom 1\noom_kill 1\n").unwrap();
        let parsed = CgroupController::read_memory_events(&tmpdir).unwrap();
        assert_eq!(parsed.high, 5);
        assert_eq!(parsed.oom, 1);
        assert_eq!(parsed.oom_kill, 1);
        assert!(parsed.experienced_oom());
        let _ = fs::remove_dir_all(&tmpdir);
    }

    /// Construction fails fast when the parent doesn't exist.  This is
    /// the operator-feedback path : "you forgot to mount cgroup v2".
    #[test]
    fn new_fails_when_parent_missing() {
        let bogus = PathBuf::from("/dev/null/not-a-cgroup");
        let err = CgroupController::new(bogus).unwrap_err();
        assert!(matches!(err, CgroupError::RootInaccessible(_)));
    }
}
