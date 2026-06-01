//! PR #20 § 16.1 — terminal output.
//!
//! Phase-2 implementation : compact text status printed to stdout every
//! `1 s` plus per-request progress dots.  Full ratatui TUI is gated
//! behind the `tui` cargo feature (Phase-3 follow-up — architecture
//! § AD19 / G.10).

use std::io::Write;

use super::OutputWriter;
use crate::error::LoadgenResult;
use crate::stats::RequestRecord;

/// Minimalist terminal writer : prints a one-line status every N
/// records and a compact JSON summary at end-of-run.
pub struct TerminalWriter {
    seen: u64,
    last_status_ts_ms: u128,
}

impl TerminalWriter {
    #[must_use]
    pub fn new() -> Self {
        Self { seen: 0, last_status_ts_ms: 0 }
    }
}

impl Default for TerminalWriter {
    fn default() -> Self { Self::new() }
}

impl OutputWriter for TerminalWriter {
    fn write(&mut self, record: &RequestRecord) -> LoadgenResult<()> {
        self.seen += 1;
        let now_ms = std::time::UNIX_EPOCH
            .elapsed()
            .map(|d| d.as_millis())
            .unwrap_or(0);
        // Update no more often than every 1 s.
        if now_ms.saturating_sub(self.last_status_ts_ms) >= 1000 {
            self.last_status_ts_ms = now_ms;
            let mut stdout = std::io::stdout().lock();
            let _ = writeln!(
                stdout,
                "[loadgen] reqs={} last={} {:?} {} (dur={:.3}ms)",
                self.seen,
                record.verb.as_str(),
                record.result,
                record.target,
                record.duration_ns as f64 / 1.0e6
            );
        }
        Ok(())
    }

    fn finalize(&mut self) -> LoadgenResult<()> {
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "[loadgen] finalize : {} requests recorded.", self.seen);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "terminal"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::{RequestResult, Verb};

    #[test]
    fn terminal_writer_counts_records() {
        let mut w = TerminalWriter::new();
        let r = RequestRecord {
            ts_ns: 0,
            verb: Verb::Iterate,
            result: RequestResult::Ok,
            duration_ns: 1000,
            target: "x".into(),
            trace_id: "y".into(),
            payload_bytes: 0,
            error_msg: None,
        };
        w.write(&r).unwrap();
        w.write(&r).unwrap();
        assert_eq!(w.seen, 2);
        w.finalize().unwrap();
    }
}
