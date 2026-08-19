//! Audit log sink — writes JSON Lines to a dedicated file separate from
//! the main log stream. Intended for M9 governance events (cone
//! registrations, evolutions, deletions, breach reports).
//!
//! The MVP implementation is intentionally minimal: append-only, no
//! rotation, no signing. Rotation is expected to be done out-of-band by
//! `logrotate` or a sidecar. Signing (tamper-evidence) is P2.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::Value;

use crate::redaction::REDACTED_MARKER;

/// An audit sink abstracts where audit entries are written.
///
/// `flush` is a **blocking** operation meant to be called at shutdown
/// or before a checkpoint. The default no-op implementation is correct
/// for in-memory sinks; `AuditLogger` overrides it to drain its
/// `BufWriter`. See audit finding M11-#10.
pub trait AuditSink: Send + Sync + std::fmt::Debug {
    /// Append a single JSON value. Implementations decide when and
    /// where to flush.
    fn write_entry(&self, entry: Value);

    /// Force any pending bytes to durable storage. Default is a no-op
    /// (for in-memory sinks that don't buffer).
    fn flush(&self) {}
}

/// File-backed JSON Lines audit logger.
#[derive(Debug)]
pub struct AuditLogger {
    path: PathBuf,
    writer: Mutex<BufWriter<File>>,
}

impl AuditLogger {
    /// Open (or create) `path` for append-only writes.
    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            path,
            writer: Mutex::new(BufWriter::new(file)),
        })
    }

    /// Return the backing path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Flush the internal buffer. Called automatically on drop.
    pub fn flush(&self) {
        if let Ok(mut w) = self.writer.lock() {
            let _ = w.flush();
        }
    }
}

impl Drop for AuditLogger {
    fn drop(&mut self) {
        self.flush();
    }
}

impl AuditSink for AuditLogger {
    fn write_entry(&self, mut entry: Value) {
        // Defensive redaction: never commit `authorization` verbatim
        // even if the caller forgot.
        if let Some(obj) = entry.as_object_mut() {
            for (k, v) in obj.iter_mut() {
                if crate::redaction::is_blocked(k) {
                    *v = Value::String(REDACTED_MARKER.to_string());
                }
            }
        }
        if let Ok(mut w) = self.writer.lock() {
            if serde_json::to_writer(&mut *w, &entry).is_ok() {
                let _ = w.write_all(b"\n");
            }
        }
    }

    fn flush(&self) {
        // Delegate to the inherent `flush` method for consistency.
        Self::flush(self);
    }
}

/// In-memory audit sink for tests.
#[derive(Debug, Default)]
pub struct MemoryAuditSink {
    entries: Mutex<Vec<Value>>,
}

impl MemoryAuditSink {
    /// Fresh empty sink.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot of entries emitted so far.
    #[must_use]
    pub fn entries(&self) -> Vec<Value> {
        self.entries.lock().map(|v| v.clone()).unwrap_or_default()
    }

    /// Number of entries emitted.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.lock().map(|v| v.len()).unwrap_or(0)
    }

    /// Convenience: `true` if no entries emitted yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl AuditSink for MemoryAuditSink {
    fn write_entry(&self, mut entry: Value) {
        if let Some(obj) = entry.as_object_mut() {
            for (k, v) in obj.iter_mut() {
                if crate::redaction::is_blocked(k) {
                    *v = Value::String(REDACTED_MARKER.to_string());
                }
            }
        }
        if let Ok(mut entries) = self.entries.lock() {
            entries.push(entry);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Read;

    #[test]
    fn memory_sink_records_entries_in_order() {
        let sink = MemoryAuditSink::new();
        sink.write_entry(json!({"event": "a"}));
        sink.write_entry(json!({"event": "b"}));
        assert_eq!(sink.len(), 2);
        let entries = sink.entries();
        assert_eq!(entries[0]["event"], "a");
        assert_eq!(entries[1]["event"], "b");
    }

    #[test]
    fn memory_sink_redacts_blocked_fields() {
        let sink = MemoryAuditSink::new();
        sink.write_entry(json!({ "raw_prompt": "secret", "event": "x" }));
        let entries = sink.entries();
        assert_eq!(entries[0]["raw_prompt"], "<redacted>");
        assert_eq!(entries[0]["event"], "x");
    }

    #[test]
    fn file_logger_appends_json_lines() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let logger = AuditLogger::open(tmp.path()).unwrap();
        logger.write_entry(json!({"event": "registered", "key": "t/k@v1"}));
        logger.write_entry(json!({"event": "deleted", "key": "t/k@v1"}));
        logger.flush();
        drop(logger); // release file handle
        let mut contents = String::new();
        std::fs::File::open(tmp.path())
            .unwrap()
            .read_to_string(&mut contents)
            .unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("registered"));
        assert!(lines[1].contains("deleted"));
    }

    #[test]
    fn file_logger_redacts_authorization_field() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let logger = AuditLogger::open(tmp.path()).unwrap();
        logger.write_entry(json!({ "authorization": "Bearer xxx", "event": "login" }));
        logger.flush();
        drop(logger);
        let mut contents = String::new();
        std::fs::File::open(tmp.path())
            .unwrap()
            .read_to_string(&mut contents)
            .unwrap();
        assert!(!contents.contains("Bearer"));
        assert!(contents.contains("<redacted>"));
    }

    #[test]
    fn file_logger_creates_parent_dir() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/dir/audit.jsonl");
        let logger = AuditLogger::open(&path).unwrap();
        logger.write_entry(json!({ "ok": true }));
        drop(logger);
        assert!(path.exists());
    }

    // Regression for audit finding M11-#2 and M11-#10:
    // `AuditSink::flush()` must work through the trait object and must
    // not rely on `Drop` being triggered by refcount.
    #[test]
    fn trait_object_flush_writes_bytes_without_dropping() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let logger: std::sync::Arc<dyn AuditSink> =
            std::sync::Arc::new(AuditLogger::open(tmp.path()).unwrap());
        // Clone the Arc to simulate a background task holding a ref.
        let clone = logger.clone();
        logger.write_entry(json!({ "event": "important" }));
        // Without explicit flush, BufWriter may hold < 8 KiB in memory.
        logger.flush();
        let contents = std::fs::read_to_string(tmp.path()).unwrap();
        assert!(
            contents.contains("important"),
            "expected flushed bytes on disk even though a clone is still live"
        );
        // Drop both Arcs to confirm Drop is idempotent with flush.
        drop(logger);
        drop(clone);
    }

    #[test]
    fn memory_sink_flush_is_noop() {
        let sink: std::sync::Arc<dyn AuditSink> = std::sync::Arc::new(MemoryAuditSink::new());
        // Default `flush` for the trait is a no-op — must not panic.
        sink.flush();
    }
}
