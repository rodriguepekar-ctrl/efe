//! Gold label queue management — durable Postgres queue with FOR
//! UPDATE SKIP LOCKED for safe concurrent worker drain.
//!
//! See § 8 of EFE_M24_ACTIVE_LEARNING_ARCHITECTURE.md.

use crate::{ActiveLearningError, ActiveLearningResult, TenantId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "gold_label_queue_status", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum QueueStatus {
    Enqueued,
    Labeling,
    LlmLabeled,
    AwaitingReview,
    Confirmed,
    Rejected,
    Failed,
    Tombstoned,
}

impl QueueStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Enqueued => "ENQUEUED",
            Self::Labeling => "LABELING",
            Self::LlmLabeled => "LLM_LABELED",
            Self::AwaitingReview => "AWAITING_REVIEW",
            Self::Confirmed => "CONFIRMED",
            Self::Rejected => "REJECTED",
            Self::Failed => "FAILED",
            Self::Tombstoned => "TOMBSTONED",
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct QueueRow {
    pub id: Uuid,
    pub tenant_id: String,
    pub catalog_id: String,
    pub content_hash: String,
    pub sentence_redacted: String,
    pub proposed_class_id: Option<String>,
    pub observed_confidence: Option<f64>,
    pub status: QueueStatus,
    pub source_llm_call_id: Option<Uuid>,
    pub enqueued_at: DateTime<Utc>,
    pub labeled_at: Option<DateTime<Utc>>,
    pub confirmed_at: Option<DateTime<Utc>>,
    pub failure_count: i32,
    pub last_error: Option<String>,
}

/// Dequeue a batch of ENQUEUED rows for a tenant, atomically marking
/// them LABELING. Uses FOR UPDATE SKIP LOCKED so multiple workers can
/// drain concurrently without double-processing.
pub async fn dequeue_batch(
    tx: &mut Transaction<'_, Postgres>,
    tenant: &TenantId,
    batch_size: u32,
) -> ActiveLearningResult<Vec<QueueRow>> {
    let rows: Vec<QueueRow> = sqlx::query_as(
        r#"
        SELECT id, tenant_id, catalog_id, content_hash, sentence_redacted,
               proposed_class_id, observed_confidence, status,
               source_llm_call_id, enqueued_at, labeled_at, confirmed_at,
               failure_count, last_error
        FROM gold_label_queue
        WHERE tenant_id = $1
          AND status = 'ENQUEUED'::gold_label_queue_status
        ORDER BY enqueued_at ASC
        LIMIT $2
        FOR UPDATE SKIP LOCKED
        "#,
    )
    .bind(&tenant.0).bind(batch_size as i64)
    .fetch_all(&mut **tx).await?;

    if rows.is_empty() { return Ok(rows); }

    let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    sqlx::query(
        r#"UPDATE gold_label_queue
           SET status = 'LABELING'::gold_label_queue_status
           WHERE id = ANY($1)"#,
    )
    .bind(&ids)
    .execute(&mut **tx).await?;

    Ok(rows)
}

/// Mark a queue row as failed (or retry by resetting to ENQUEUED if
/// failure_count < 3). Called when the LLM worker hits an error.
pub async fn mark_failed(
    pg: &PgPool, id: Uuid, error: &str,
) -> ActiveLearningResult<()> {
    sqlx::query(
        r#"UPDATE gold_label_queue
           SET status = CASE
                          WHEN failure_count + 1 >= 3 THEN 'FAILED'::gold_label_queue_status
                          ELSE 'ENQUEUED'::gold_label_queue_status
                        END,
               failure_count = failure_count + 1,
               last_error = $2
           WHERE id = $1"#,
    )
    .bind(id).bind(error)
    .execute(pg).await?;
    Ok(())
}

/// Transition a queue row to a new status (validated by DB trigger).
pub async fn transition(
    pg: &PgPool, id: Uuid, new_status: QueueStatus,
) -> ActiveLearningResult<()> {
    sqlx::query(
        r#"UPDATE gold_label_queue
           SET status = $1::gold_label_queue_status,
               labeled_at = CASE WHEN $1 = 'LLM_LABELED' THEN NOW() ELSE labeled_at END,
               confirmed_at = CASE WHEN $1 = 'CONFIRMED' THEN NOW() ELSE confirmed_at END
           WHERE id = $2"#,
    )
    .bind(new_status.as_str()).bind(id)
    .execute(pg).await?;
    Ok(())
}

/// Tombstone a row for GDPR Art. 17.
pub async fn tombstone(
    pg: &PgPool, id: Uuid, reason: &str,
) -> ActiveLearningResult<()> {
    sqlx::query(
        r#"UPDATE gold_label_queue
           SET status = 'TOMBSTONED'::gold_label_queue_status,
               sentence_redacted = '[REDACTED-Art17]',
               last_error = $2
           WHERE id = $1"#,
    )
    .bind(id).bind(reason)
    .execute(pg).await?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueStats {
    pub enqueued: u64,
    pub labeling: u64,
    pub llm_labeled: u64,
    pub awaiting_review: u64,
    pub failed: u64,
    pub total_in_flight: u64,
    pub oldest_age_seconds: Option<i64>,
}

pub async fn get_stats(
    pg: &PgPool, tenant: &TenantId,
) -> ActiveLearningResult<QueueStats> {
    let row: (i64, i64, i64, i64, i64, Option<DateTime<Utc>>) = sqlx::query_as(
        r#"
        SELECT
            COUNT(*) FILTER (WHERE status = 'ENQUEUED')        AS enqueued,
            COUNT(*) FILTER (WHERE status = 'LABELING')        AS labeling,
            COUNT(*) FILTER (WHERE status = 'LLM_LABELED')     AS llm_labeled,
            COUNT(*) FILTER (WHERE status = 'AWAITING_REVIEW') AS awaiting_review,
            COUNT(*) FILTER (WHERE status = 'FAILED')          AS failed,
            MIN(enqueued_at) FILTER (WHERE status IN ('ENQUEUED', 'LABELING', 'LLM_LABELED'))
                                                                AS oldest_open
        FROM gold_label_queue
        WHERE tenant_id = $1
        "#,
    )
    .bind(&tenant.0)
    .fetch_one(pg).await?;

    let total = (row.0 + row.1 + row.2 + row.3) as u64;
    let oldest_age = row.5.map(|t| (Utc::now() - t).num_seconds());

    Ok(QueueStats {
        enqueued: row.0 as u64,
        labeling: row.1 as u64,
        llm_labeled: row.2 as u64,
        awaiting_review: row.3 as u64,
        failed: row.4 as u64,
        total_in_flight: total,
        oldest_age_seconds: oldest_age,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_serialization_screaming_snake() {
        assert_eq!(QueueStatus::Enqueued.as_str(), "ENQUEUED");
        assert_eq!(QueueStatus::LlmLabeled.as_str(), "LLM_LABELED");
        assert_eq!(QueueStatus::AwaitingReview.as_str(), "AWAITING_REVIEW");
    }
}
