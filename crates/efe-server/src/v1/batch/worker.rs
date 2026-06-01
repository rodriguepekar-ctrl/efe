//! Pure pipeline logic for the batch worker.
//!
//! Hosted by the `efe-batch-worker` binary (in `crates/efe-server/src/bin/`).
//! Kept pure (no async IO, no DB) so unit tests can exercise the
//! row-batching + cost-capping math.

use super::job::BatchJob;

/// Per-row classification result before serialization to the output
/// Parquet/CSV file.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassifiedRow {
    pub source_row_index: u64,
    pub text: String,
    pub class_id: String,
    pub confidence: f32,
    pub cost_cents: u64,
}

/// Aggregate stats accumulated as the worker streams batches.
#[derive(Debug, Clone, Copy, Default)]
pub struct BatchProgress {
    pub rows_processed: u64,
    pub cost_cents: u64,
}

impl BatchProgress {
    /// Apply a freshly-classified row, returning a [`BatchTerminate`]
    /// indicator if the cost cap is now exceeded.
    pub fn record(&mut self, row: &ClassifiedRow, max_cost_cents: Option<u64>) -> BatchAction {
        self.rows_processed += 1;
        self.cost_cents += row.cost_cents;
        if let Some(cap) = max_cost_cents {
            if self.cost_cents > cap {
                return BatchAction::TerminateCostCap;
            }
        }
        BatchAction::Continue
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchAction {
    Continue,
    TerminateCostCap,
}

/// Compute the cents cap from the job's `max_cost_usd` field, if any.
pub fn cost_cap_cents(job: &BatchJob) -> Option<u64> {
    job.max_cost_usd.map(|usd| (usd * 100.0).round() as u64)
}

/// Determine the batch size for `classify_n` based on environment +
/// caller hint. Default = 1000 ; clamp to 100..=10_000.
pub fn batch_size() -> usize {
    let raw = std::env::var("EFE_BATCH_CHUNK_SIZE")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(1000);
    raw.clamp(100, 10_000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1::batch::job::{BatchStatus, InputFormat};
    use chrono::Utc;
    use uuid::Uuid;

    fn dummy_job(max_cost_usd: Option<f64>) -> BatchJob {
        BatchJob {
            id: Uuid::new_v4(),
            tenant_id: "acme".into(),
            catalog_id: "demo".into(),
            input_url: "s3://acme-data/input.parquet".into(),
            input_format: InputFormat::Parquet,
            input_column: "text".into(),
            output_url: "s3://acme-data/output.parquet".into(),
            output_format: InputFormat::Parquet,
            callback_webhook_id: None,
            max_cost_usd,
            mode: 1,
            status: BatchStatus::Running,
            rows_total: Some(100),
            rows_processed: 0,
            cost_cents: 0,
            error_message: None,
            submitted_by: "user".into(),
            submitted_at: Utc::now(),
            started_at: None,
            completed_at: None,
        }
    }

    fn dummy_row(cost_cents: u64) -> ClassifiedRow {
        ClassifiedRow {
            source_row_index: 0,
            text: "x".into(),
            class_id: "c".into(),
            confidence: 0.9,
            cost_cents,
        }
    }

    #[test]
    fn progress_accumulates() {
        let mut p = BatchProgress::default();
        p.record(&dummy_row(10), None);
        p.record(&dummy_row(20), None);
        assert_eq!(p.rows_processed, 2);
        assert_eq!(p.cost_cents, 30);
    }

    #[test]
    fn cost_cap_terminates() {
        let mut p = BatchProgress::default();
        assert_eq!(p.record(&dummy_row(50), Some(100)), BatchAction::Continue);
        assert_eq!(
            p.record(&dummy_row(60), Some(100)),
            BatchAction::TerminateCostCap
        );
    }

    #[test]
    fn cost_cap_cents_from_usd() {
        let j = dummy_job(Some(5.0));
        assert_eq!(cost_cap_cents(&j), Some(500));
        let j2 = dummy_job(None);
        assert_eq!(cost_cap_cents(&j2), None);
    }

    // Φ-D-OPS audit fix : these two tests mutate the global env
    // var `EFE_BATCH_CHUNK_SIZE` and race when run in parallel.
    // Serialise them via a module-local mutex so the writer-reader
    // pattern is observed correctly.
    static BATCH_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn batch_size_default() {
        let _g = BATCH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("EFE_BATCH_CHUNK_SIZE");
        assert_eq!(batch_size(), 1000);
    }

    #[test]
    fn batch_size_clamps() {
        let _g = BATCH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("EFE_BATCH_CHUNK_SIZE", "50");
        assert_eq!(batch_size(), 100);
        std::env::set_var("EFE_BATCH_CHUNK_SIZE", "999999");
        assert_eq!(batch_size(), 10_000);
        std::env::remove_var("EFE_BATCH_CHUNK_SIZE");
    }
}
