//! PR #20 § 16 — output formatters.
//!
//! 4 output channels :
//!   * `Terminal` — live status (non-TTY falls back to ndjson).
//!   * `Csv`      — per-request rows for spreadsheet imports.
//!   * `Ndjson`   — one JSON line per request, streamable.
//!   * `Parquet`  — columnar, default for CI consumption (feature-gated).
//!
//! The engine wraps every result row in a [`crate::stats::RequestRecord`]
//! and hands it to the active [`OutputWriter`].

pub mod csv;
pub mod ndjson;
pub mod parquet;
pub mod terminal;

use std::path::Path;

use crate::cli::OutputFormat;
use crate::error::{LoadgenError, LoadgenResult};
use crate::stats::RequestRecord;

/// Common interface implemented by every output formatter.
pub trait OutputWriter: Send + Sync {
    /// Append one request record.  Called per request (high throughput
    /// path — must be cheap).
    fn write(&mut self, record: &RequestRecord) -> LoadgenResult<()>;

    /// Flush + close the underlying sink.  Called at end-of-run before
    /// SLO evaluation.
    fn finalize(&mut self) -> LoadgenResult<()>;

    /// Friendly name (matches CLI `OutputFormat`).
    fn name(&self) -> &'static str;
}

/// Build a boxed [`OutputWriter`] from a CLI [`OutputFormat`] + optional
/// path.  When `path` is `None`, sinks go to stdout (CSV / ndjson) or
/// the terminal renderer (Terminal mode).
pub fn build(format: OutputFormat, path: Option<&Path>) -> LoadgenResult<Box<dyn OutputWriter>> {
    Ok(match format {
        OutputFormat::Terminal => Box::new(terminal::TerminalWriter::new()),
        OutputFormat::Json => Box::new(ndjson::NdjsonWriter::new_stdout()),
        OutputFormat::Csv => match path {
            Some(p) => Box::new(csv::CsvWriter::new_file(p)?),
            None => Box::new(csv::CsvWriter::new_stdout()),
        },
        OutputFormat::Ndjson => match path {
            Some(p) => Box::new(ndjson::NdjsonWriter::new_file(p)?),
            None => Box::new(ndjson::NdjsonWriter::new_stdout()),
        },
        OutputFormat::Parquet => match path {
            Some(p) => Box::new(parquet::ParquetWriter::new(p)?),
            None => {
                return Err(LoadgenError::InvalidConfig(
                    "parquet output requires --output-file".into(),
                ))
            }
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_terminal_does_not_require_file() {
        let w = build(OutputFormat::Terminal, None).unwrap();
        assert_eq!(w.name(), "terminal");
    }

    #[test]
    fn build_parquet_without_file_rejected() {
        let result = build(OutputFormat::Parquet, None);
        // Can't use unwrap_err — dyn OutputWriter isn't Debug.
        assert!(result.is_err());
        if let Err(err) = result {
            assert!(matches!(err, LoadgenError::InvalidConfig(_)));
        }
    }

    #[test]
    fn build_csv_stdout_when_no_file() {
        let w = build(OutputFormat::Csv, None).unwrap();
        assert_eq!(w.name(), "csv");
    }
}
