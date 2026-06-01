//! PR #20 § 16.4 — parquet output (feature-gated).
//!
//! Phase-2 ships a stub that writes ndjson with a `.parquet` extension
//! so the operator surface (file paths, helm chart, CI tooling) is
//! ready.  The real Apache Arrow / parquet implementation is a
//! Phase-3 deliverable behind the `parquet` cargo feature (architecture
//! § AD12 + G.7).

use std::io::Write;
use std::path::Path;

use super::OutputWriter;
use crate::error::{LoadgenError, LoadgenResult};
use crate::stats::RequestRecord;

type SyncWriter = Box<dyn Write + Send + Sync>;

/// Parquet writer — stubbed in Phase-2 to ndjson with a header marker.
pub struct ParquetWriter {
    inner: SyncWriter,
}

impl ParquetWriter {
    pub fn new(path: &Path) -> LoadgenResult<Self> {
        let file = std::fs::File::create(path).map_err(|source| LoadgenError::Output {
            path: path.to_path_buf(),
            source,
        })?;
        let mut w: SyncWriter = Box::new(std::io::BufWriter::new(file));
        // Marker comment so consumers can detect the stub vs a real parquet.
        let _ = writeln!(
            w,
            "# PR #20 parquet stub — Phase-2 ndjson with .parquet extension.  Phase-3 G.7 swaps to real Apache Parquet via arrow crate."
        );
        Ok(Self { inner: w })
    }
}

impl OutputWriter for ParquetWriter {
    fn write(&mut self, record: &RequestRecord) -> LoadgenResult<()> {
        let line = serde_json::to_string(record)
            .map_err(|e| LoadgenError::Internal(format!("parquet stub serialize : {e}")))?;
        self.inner
            .write_all(line.as_bytes())
            .map_err(|e| LoadgenError::Internal(format!("parquet stub write : {e}")))?;
        self.inner
            .write_all(b"\n")
            .map_err(|e| LoadgenError::Internal(format!("parquet stub newline : {e}")))?;
        Ok(())
    }

    fn finalize(&mut self) -> LoadgenResult<()> {
        self.inner
            .flush()
            .map_err(|e| LoadgenError::Internal(format!("parquet flush : {e}")))?;
        Ok(())
    }

    fn name(&self) -> &'static str {
        "parquet-stub"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn stub_writes_marker_header_and_rows() {
        let tmp = NamedTempFile::new().unwrap();
        let mut w = ParquetWriter::new(tmp.path()).unwrap();
        w.finalize().unwrap();
        let content = std::fs::read_to_string(tmp.path()).unwrap();
        assert!(content.starts_with("# PR #20 parquet stub"));
    }
}
