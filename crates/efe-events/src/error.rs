//! Error type for all event operations.

use thiserror::Error;

/// All `efe-events` operations return this error.
#[derive(Debug, Error)]
pub enum EventError {
    /// NATS connection / publish / subscribe error.
    #[error("nats error : {0}")]
    Nats(String),

    /// JetStream stream / consumer create / fetch error.
    #[error("jetstream error : {0}")]
    JetStream(String),

    /// JSON encode of payload failed.
    #[error("encode failed : {0}")]
    Encode(#[from] serde_json::Error),

    /// Bus unreachable ; caller MUST degrade gracefully (don't block).
    #[error("event bus unavailable : {0}")]
    Unavailable(String),

    /// Misconfiguration (bad URL, missing creds, etc.).
    #[error("event bus misconfigured : {0}")]
    Misconfig(String),

    /// In-memory publisher buffer overflowed ; oldest event dropped.
    /// Returned as Err but the new event WAS published (best-effort).
    #[error("publisher buffer overflow ; {dropped} oldest events dropped")]
    BufferOverflow {
        /// Number of events dropped to make room.
        dropped: usize,
    },
}

/// Convenience alias.
pub type EventResult<T> = Result<T, EventError>;
