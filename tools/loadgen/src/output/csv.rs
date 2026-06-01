//! PR #20 § 16.2 — CSV per-request output.

use std::io::Write;
use std::path::Path;

use super::OutputWriter;
use crate::error::{LoadgenError, LoadgenResult};
use crate::stats::RequestRecord;

/// Write trait object that's both Send + Sync (required because
/// [`OutputWriter`] is `Send + Sync`).  We never let two threads
/// write concurrently — the engine owns the writer exclusively in a
/// `&mut self` context — but the trait bound requires it.
type SyncWriter = Box<dyn Write + Send + Sync>;

/// CSV writer — one row per request.
pub struct CsvWriter {
    inner: csv::Writer<SyncWriter>,
}

impl CsvWriter {
    /// New writer backed by a file path.  Truncates any existing file.
    pub fn new_file(path: &Path) -> LoadgenResult<Self> {
        let file = std::fs::File::create(path).map_err(|source| LoadgenError::Output {
            path: path.to_path_buf(),
            source,
        })?;
        let boxed: SyncWriter = Box::new(file);
        let mut wtr = csv::WriterBuilder::new().from_writer(boxed);
        wtr.write_record([
            "ts_ns", "verb", "result", "duration_ns", "target", "trace_id", "payload_bytes", "error_msg",
        ])
        .map_err(|e| LoadgenError::Internal(format!("csv header : {e}")))?;
        Ok(Self { inner: wtr })
    }

    /// New writer backed by stdout.
    #[must_use]
    pub fn new_stdout() -> Self {
        let boxed: SyncWriter = Box::new(std::io::stdout());
        let mut wtr = csv::WriterBuilder::new().from_writer(boxed);
        let _ = wtr.write_record([
            "ts_ns", "verb", "result", "duration_ns", "target", "trace_id", "payload_bytes", "error_msg",
        ]);
        Self { inner: wtr }
    }
}

impl OutputWriter for CsvWriter {
    fn write(&mut self, record: &RequestRecord) -> LoadgenResult<()> {
        self.inner
            .write_record(&[
                record.ts_ns.to_string(),
                record.verb.as_str().to_string(),
                record.result.as_str().to_string(),
                record.duration_ns.to_string(),
                record.target.clone(),
                record.trace_id.clone(),
                record.payload_bytes.to_string(),
                record.error_msg.clone().unwrap_or_default(),
            ])
            .map_err(|e| LoadgenError::Internal(format!("csv write : {e}")))?;
        Ok(())
    }

    fn finalize(&mut self) -> LoadgenResult<()> {
        self.inner
            .flush()
            .map_err(|e| LoadgenError::Internal(format!("csv flush : {e}")))?;
        Ok(())
    }

    fn name(&self) -> &'static str {
        "csv"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::{RequestResult, Verb};
    use tempfile::NamedTempFile;

    fn sample_record() -> RequestRecord {
        RequestRecord {
            ts_ns: 100,
            verb: Verb::Iterate,
            result: RequestResult::Ok,
            duration_ns: 12_345_678,
            target: "ctrl-1".into(),
            trace_id: "abc123".into(),
            payload_bytes: 256,
            error_msg: None,
        }
    }

    #[test]
    fn csv_writes_header_and_row() {
        let tmp = NamedTempFile::new().unwrap();
        let mut w = CsvWriter::new_file(tmp.path()).unwrap();
        w.write(&sample_record()).unwrap();
        w.finalize().unwrap();
        let content = std::fs::read_to_string(tmp.path()).unwrap();
        assert!(content.contains("ts_ns,verb"));
        assert!(content.contains("iterate"));
        assert!(content.contains("abc123"));
    }
}
