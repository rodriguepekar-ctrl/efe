//! Batch job lifecycle types + state transitions.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BatchStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl BatchStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    /// Validate a transition. State machine :
    ///   queued → (running | cancelled)
    ///   running → (completed | failed | cancelled)
    ///   completed | failed | cancelled = sink
    pub fn can_transition_to(self, next: BatchStatus) -> bool {
        use BatchStatus::*;
        matches!(
            (self, next),
            (Queued, Running)
                | (Queued, Cancelled)
                | (Running, Completed)
                | (Running, Failed)
                | (Running, Cancelled)
        )
    }
}

impl FromStr for BatchStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(format!("unknown batch status : {}", other)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputFormat {
    Parquet,
    Csv,
}

impl InputFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Parquet => "parquet",
            Self::Csv => "csv",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchJob {
    pub id: Uuid,
    pub tenant_id: String,
    pub catalog_id: String,
    pub input_url: String,
    pub input_format: InputFormat,
    pub input_column: String,
    pub output_url: String,
    pub output_format: InputFormat,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback_webhook_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<f64>,
    pub mode: i16,
    pub status: BatchStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows_total: Option<i64>,
    pub rows_processed: i64,
    pub cost_cents: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub submitted_by: String,
    pub submitted_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_transitions() {
        assert!(BatchStatus::Queued.can_transition_to(BatchStatus::Running));
        assert!(BatchStatus::Queued.can_transition_to(BatchStatus::Cancelled));
        assert!(BatchStatus::Running.can_transition_to(BatchStatus::Completed));
        assert!(BatchStatus::Running.can_transition_to(BatchStatus::Failed));
        assert!(BatchStatus::Running.can_transition_to(BatchStatus::Cancelled));
        // Forbidden :
        assert!(!BatchStatus::Queued.can_transition_to(BatchStatus::Completed));
        assert!(!BatchStatus::Completed.can_transition_to(BatchStatus::Running));
        assert!(!BatchStatus::Failed.can_transition_to(BatchStatus::Running));
    }

    #[test]
    fn from_str_roundtrip() {
        for s in &["queued", "running", "completed", "failed", "cancelled"] {
            let bs: BatchStatus = s.parse().unwrap();
            assert_eq!(bs.as_str(), *s);
        }
        assert!(BatchStatus::from_str("bogus").is_err());
    }

    #[test]
    fn terminal_set() {
        assert!(BatchStatus::Completed.is_terminal());
        assert!(BatchStatus::Failed.is_terminal());
        assert!(BatchStatus::Cancelled.is_terminal());
        assert!(!BatchStatus::Queued.is_terminal());
        assert!(!BatchStatus::Running.is_terminal());
    }
}
