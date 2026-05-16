//! PR #24 § 10 + AD14-AD19 — 5 frozen JSON API endpoints.
//!
//! STABILITY_CONTRACT v1.6 § 24.3 :
//!
//!   `/api/current.json`     latest bench-full result
//!   `/api/history.json`     last N daily runs (tamper-evidence via sha256)
//!   `/api/scenarios.json`   10 scenarios metadata + latest verdicts
//!   `/api/baselines.json`   published baselines per chart version
//!   `/api/version.json`     harness + chart + git_sha + build_date

pub mod baselines;
pub mod current;
pub mod history;
pub mod scenarios;
pub mod version;

use std::path::Path;

use crate::error::{BenchError, BenchResult};

/// Schema version stamped on every JSON file (AD15, AD16 — forward-compat).
pub const API_SCHEMA_VERSION: u32 = 1;

/// STABILITY_CONTRACT v1.6 § 24.3 — the 5 frozen API endpoint filenames.
///
/// **Single source of truth (SOT)** — STABILITY v1.7 § 25.2.  Consumers
/// MUST reference this const ; do NOT duplicate the list.  Adding a
/// 6th entry requires a STABILITY major bump.
///
/// PR #25 day 4 hoist (D-AS final cleanup) :
///   * was `REQUIRED_API` const local to `publish/deploy/ratchet.rs`
///   * now lives at the most general module that DEFINES the concept :
///     `crate::publish::api::API_ENDPOINT_FILENAMES`.
///
/// Consumers :
///   * `publish::deploy::ratchet::run` — iterates to count files.
///   * `tests/stability_contract_v16.rs` v1_6_section_24_3_*  — locks
///     the count + content at v1.6 frozen surface.
///   * `tests/stability_contract_v17.rs` v1_7_section_25_2_*  — asserts
///     the canonical location at v1.7.
///   * `tests/process_invariants.rs` i_2_*  — asserts no parallel
///     list elsewhere in the workspace.
pub const API_ENDPOINT_FILENAMES: &[&str] = &[
    "current.json",
    "history.json",
    "scenarios.json",
    "baselines.json",
    "version.json",
];

/// Write a JSON value to disk in canonical form (stable struct-field
/// order + 2-space indent + BTreeMap canonical sort — AD18).  Atomic
/// via temp + rename(2).
///
/// **D-Z fix** — the temp filename now carries a per-call UUID-v4
/// suffix so two publisher processes writing to the same `output_dir`
/// cannot clobber each other's `.tmp` file.  Previously every call
/// used the fixed name `{stem}.json.tmp`, so concurrent invocations
/// could interleave : Process B's truncate would erase Process A's
/// bytes mid-rename, leaving a torn / mixed file on disk.
pub fn write_canonical<T: serde::Serialize>(path: &Path, value: &T) -> BenchResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| BenchError::Result {
            path: path.to_path_buf(),
            source,
        })?;
    }
    let body = serde_json::to_vec_pretty(value)?;
    // Unique per-call suffix : UUID v4 → collision-free across
    // concurrent processes / threads on the same `output_dir`.
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let tmp = path.with_extension(format!("json.{suffix}.tmp"));
    std::fs::write(&tmp, body).map_err(|source| BenchError::Result {
        path: tmp.clone(),
        source,
    })?;
    std::fs::rename(&tmp, path).map_err(|source| BenchError::Result {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_version_frozen_1() {
        // STABILITY_CONTRACT v1.6 § 24.3.
        assert_eq!(API_SCHEMA_VERSION, 1);
    }

    #[test]
    fn write_canonical_atomic_no_partial() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.json");
        let v = serde_json::json!({"hello": "world"});
        write_canonical(&path, &v).unwrap();
        // No .tmp* file should linger in the output dir (D-Z : the
        // suffix is per-call, so check the WHOLE directory is clean).
        let leftover_tmps: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .contains(".tmp")
            })
            .collect();
        assert!(
            leftover_tmps.is_empty(),
            "no .tmp* file must linger after write_canonical : {leftover_tmps:?}"
        );
        // Final path exists.
        assert!(path.exists());
    }

    #[test]
    fn d_z_write_canonical_unique_temp_per_call() {
        // **D-Z** — successive calls must use DIFFERENT temp filenames.
        // The previous implementation hardcoded `{stem}.json.tmp`,
        // which made two concurrent writers race on the same `.tmp`.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.json");

        // We inspect the temp filename via std::fs::rename hook is hard.
        // Easier : run write_canonical twice in quick succession and
        // assert both succeed (a fixed-temp collision in a single
        // thread succeeds, so this isn't dispositive — see the threaded
        // test below for the real race scenario).
        for v in [
            serde_json::json!({"v": 1}),
            serde_json::json!({"v": 2}),
        ] {
            write_canonical(&path, &v).unwrap();
        }
        let final_body = std::fs::read_to_string(&path).unwrap();
        assert!(
            final_body.contains("\"v\""),
            "last write must have landed cleanly"
        );
    }

    #[test]
    fn d_z_write_canonical_concurrent_threads_dont_corrupt() {
        // **D-Z** — N concurrent writers to the SAME output path must
        // never produce a corrupted file.  The final state must be
        // one of the inputs, byte-for-byte (last-writer-wins via
        // rename(2) atomicity).
        use std::sync::Arc;
        use std::thread;
        let tmp = Arc::new(tempfile::tempdir().unwrap());
        let path = tmp.path().join("concurrent.json");
        let handles: Vec<_> = (0..16)
            .map(|i| {
                let p = path.clone();
                thread::spawn(move || {
                    let v = serde_json::json!({"writer": i, "padding": "x".repeat(1024)});
                    write_canonical(&p, &v).unwrap();
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        // File MUST exist and parse as valid JSON (no torn writes).
        let body = std::fs::read_to_string(&path).expect("final file must exist");
        let parsed: serde_json::Value =
            serde_json::from_str(&body).expect("D-Z : torn write → invalid JSON");
        assert!(parsed.get("writer").is_some(), "must carry one writer's payload");
        // No leftover .tmp files in the directory.
        let leftover_tmps: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(
            leftover_tmps.is_empty(),
            "D-Z : concurrent writers left orphan .tmp files : {leftover_tmps:?}"
        );
    }

    #[test]
    fn write_canonical_pretty_printed() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("p.json");
        let v = serde_json::json!({"key": "value"});
        write_canonical(&path, &v).unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        // serde_json::to_vec_pretty uses 2-space indent by default.
        assert!(body.contains("  \"key\""));
    }
}
