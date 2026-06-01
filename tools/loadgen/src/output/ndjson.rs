//! PR #20 § 16.3 — newline-delimited JSON output.

use std::io::Write;
use std::path::Path;

use super::OutputWriter;
use crate::error::{LoadgenError, LoadgenResult};
use crate::stats::RequestRecord;

type SyncWriter = Box<dyn Write + Send + Sync>;

/// One JSON object per line — streamable.  Default for non-TTY runs.
pub struct NdjsonWriter {
    inner: SyncWriter,
}

impl NdjsonWriter {
    pub fn new_file(path: &Path) -> LoadgenResult<Self> {
        let file = std::fs::File::create(path).map_err(|source| LoadgenError::Output {
            path: path.to_path_buf(),
            source,
        })?;
        let boxed: SyncWriter = Box::new(std::io::BufWriter::new(file));
        Ok(Self { inner: boxed })
    }

    #[must_use]
    pub fn new_stdout() -> Self {
        let boxed: SyncWriter = Box::new(std::io::BufWriter::new(std::io::stdout()));
        Self { inner: boxed }
    }
}

impl OutputWriter for NdjsonWriter {
    fn write(&mut self, record: &RequestRecord) -> LoadgenResult<()> {
        let line = serde_json::to_string(record)
            .map_err(|e| LoadgenError::Internal(format!("ndjson serialize : {e}")))?;
        self.inner
            .write_all(line.as_bytes())
            .map_err(|e| LoadgenError::Internal(format!("ndjson write : {e}")))?;
        self.inner
            .write_all(b"\n")
            .map_err(|e| LoadgenError::Internal(format!("ndjson newline : {e}")))?;
        Ok(())
    }

    fn finalize(&mut self) -> LoadgenResult<()> {
        self.inner
            .flush()
            .map_err(|e| LoadgenError::Internal(format!("ndjson flush : {e}")))?;
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ndjson"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::{RequestResult, Verb};
    use tempfile::NamedTempFile;

    #[test]
    fn ndjson_writes_one_line_per_record() {
        let tmp = NamedTempFile::new().unwrap();
        let mut w = NdjsonWriter::new_file(tmp.path()).unwrap();
        let rec = RequestRecord {
            ts_ns: 1,
            verb: Verb::Iterate,
            result: RequestResult::Ok,
            duration_ns: 100,
            target: "t".into(),
            trace_id: "abc".into(),
            payload_bytes: 8,
            error_msg: None,
        };
        w.write(&rec).unwrap();
        w.write(&rec).unwrap();
        w.finalize().unwrap();
        let content = std::fs::read_to_string(tmp.path()).unwrap();
        assert_eq!(content.lines().count(), 2);
        let parsed: serde_json::Value = serde_json::from_str(content.lines().next().unwrap()).unwrap();
        assert_eq!(parsed["verb"], "iterate");
    }
}
