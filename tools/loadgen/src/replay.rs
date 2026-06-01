//! PR #20 § 21 — replay mode.
//!
//! Reads a recorded session (CSV or ndjson — parquet stub support
//! pending) and replays it against a new target with one of three
//! timing modes (preserve-spacing / accelerate / drain).

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{LoadgenError, LoadgenResult};

/// Replay recording row.  Schema matches the ndjson output of
/// [`crate::output::ndjson::NdjsonWriter`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedRequest {
    pub ts_ns: u64,
    pub verb: String,
    #[serde(default)]
    pub duration_ns: u64,
    #[serde(default)]
    pub payload_bytes: u32,
}

/// Replay reader — supports ndjson + CSV by file extension.
pub struct ReplayReader {
    pub rows: Vec<RecordedRequest>,
    pub nominal_qps: f64,
}

impl ReplayReader {
    /// Load all rows + compute the nominal QPS.
    pub fn load(path: &Path) -> LoadgenResult<Self> {
        let file = File::open(path).map_err(|source| LoadgenError::Output {
            path: path.to_path_buf(),
            source,
        })?;
        let reader = BufReader::new(file);
        let mut rows = Vec::new();
        let ext = path.extension().and_then(|x| x.to_str()).unwrap_or("");
        match ext {
            "csv" => {
                let mut rdr = ::csv::Reader::from_reader(reader);
                for r in rdr.deserialize::<RecordedRequest>() {
                    let r = r.map_err(|e| {
                        LoadgenError::InvalidConfig(format!("replay CSV parse : {e}"))
                    })?;
                    rows.push(r);
                }
            }
            "json" | "ndjson" | "parquet" => {
                for line in reader.lines() {
                    let line = line.map_err(|source| LoadgenError::Output {
                        path: path.to_path_buf(),
                        source,
                    })?;
                    if line.trim().is_empty() || line.starts_with('#') {
                        continue;
                    }
                    let r: RecordedRequest = serde_json::from_str(&line).map_err(|e| {
                        LoadgenError::InvalidConfig(format!("replay ndjson parse : {e}"))
                    })?;
                    rows.push(r);
                }
            }
            other => {
                return Err(LoadgenError::InvalidConfig(format!(
                    "replay file extension {other:?} not supported (csv | ndjson | parquet)"
                )))
            }
        }
        let nominal_qps = compute_nominal_qps(&rows);
        Ok(Self { rows, nominal_qps })
    }

    /// Iterate over the rows with computed inter-arrival deltas.
    pub fn into_iter_with_spacing(self) -> impl Iterator<Item = (RecordedRequest, Duration)> {
        let mut iter = self.rows.into_iter().peekable();
        let mut last_ts = None::<u64>;
        std::iter::from_fn(move || {
            let r = iter.next()?;
            let dt = if let Some(prev) = last_ts {
                Duration::from_nanos(r.ts_ns.saturating_sub(prev))
            } else {
                Duration::ZERO
            };
            last_ts = Some(r.ts_ns);
            Some((r, dt))
        })
    }
}

/// Compute the nominal QPS = total_rows / total_window.
fn compute_nominal_qps(rows: &[RecordedRequest]) -> f64 {
    if rows.len() < 2 {
        return 0.0;
    }
    let first = rows.first().map(|r| r.ts_ns).unwrap_or(0);
    let last = rows.last().map(|r| r.ts_ns).unwrap_or(0);
    let dt_s = (last - first) as f64 / 1.0e9;
    if dt_s <= 0.0 {
        return 0.0;
    }
    rows.len() as f64 / dt_s
}

/// Returned by [`replay_record`] indicating which row was replayed.
#[derive(Debug, Clone)]
pub struct ReplayedRecord {
    pub row: RecordedRequest,
    pub wall_clock_offset: Duration,
}

/// Helper : compute the wall-clock delay for one record under the given
/// playback mode.
#[must_use]
pub fn pace_delay(dt: Duration, mode: crate::config::ReplayPlaybackMode, speed: f64) -> Duration {
    use crate::config::ReplayPlaybackMode::*;
    match mode {
        PreserveSpacing => dt,
        Accelerate => {
            let scaled = dt.as_secs_f64() / speed.max(0.001);
            Duration::from_secs_f64(scaled)
        }
        Drain => Duration::ZERO,
    }
}

/// Test helper exposing the recorded file path resolution.
#[must_use]
pub fn detect_path(file: &Path) -> &Path {
    file
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_ndjson(path: &Path, rows: &[RecordedRequest]) {
        let mut f = std::fs::File::create(path).unwrap();
        for r in rows {
            writeln!(f, "{}", serde_json::to_string(r).unwrap()).unwrap();
        }
    }

    #[test]
    fn loads_ndjson_rows() {
        let rows = vec![
            RecordedRequest { ts_ns: 0,         verb: "iterate".into(), duration_ns: 100, payload_bytes: 64 },
            RecordedRequest { ts_ns: 1_000_000, verb: "iterate".into(), duration_ns: 110, payload_bytes: 64 },
            RecordedRequest { ts_ns: 2_000_000, verb: "iterate".into(), duration_ns: 120, payload_bytes: 64 },
        ];
        let tmp = NamedTempFile::new().unwrap();
        let tmp_path = tmp.path().with_extension("ndjson");
        write_ndjson(&tmp_path, &rows);
        let reader = ReplayReader::load(&tmp_path).unwrap();
        assert_eq!(reader.rows.len(), 3);
        // 3 rows over 2ms → 1500 QPS approx.
        assert!(reader.nominal_qps > 1000.0);
    }

    #[test]
    fn pace_drain_returns_zero() {
        let d = pace_delay(
            Duration::from_secs(10),
            crate::config::ReplayPlaybackMode::Drain,
            1.0,
        );
        assert_eq!(d, Duration::ZERO);
    }

    #[test]
    fn pace_accelerate_scales_by_speed() {
        let d = pace_delay(
            Duration::from_secs(10),
            crate::config::ReplayPlaybackMode::Accelerate,
            10.0,
        );
        assert!((d.as_secs_f64() - 1.0).abs() < 1e-9);
    }
}
