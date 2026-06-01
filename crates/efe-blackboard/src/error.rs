// SPDX-License-Identifier: UNLICENSED
//! **Sprint 13 — N21** : closed taxonomy of blackboard + eventbus errors.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// **N21** : Blackboard error taxonomy.
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BlackboardError {
    /// CAS-style `write_if` saw a different version than expected (F9).
    #[error("blackboard conflict at {pointer:?} : expected version {expected_version}, got {actual_version}")]
    Conflict {
        pointer: String,
        expected_version: u64,
        actual_version: u64,
    },

    /// JSON pointer doesn't conform to RFC 6901 (must be "" or start
    /// with '/').
    #[error("invalid JSON pointer : {value:?}")]
    InvalidPointer { value: String },

    /// Cumulative byte size of the blackboard exceeds the cap.
    #[error("blackboard size {current} bytes exceeds cap {max}")]
    SizeLimitExceeded { current: usize, max: usize },

    /// Backend storage failure.
    #[error("backend : {message}")]
    Backend { message: String },
}

/// **N21** : EventBus error taxonomy.
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventBusError {
    /// No active subscribers exist for the bus.
    #[error("no subscribers")]
    NoSubscribers,

    /// Backend send failure.
    #[error("backend : {message}")]
    Backend { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blackboard_error_display_non_empty() {
        let e = BlackboardError::Conflict {
            pointer: "/x".into(),
            expected_version: 1,
            actual_version: 2,
        };
        assert!(!format!("{e}").is_empty());
    }

    #[test]
    fn blackboard_error_serde_round_trip() {
        let cases = vec![
            BlackboardError::Conflict {
                pointer: "/x".into(),
                expected_version: 1,
                actual_version: 2,
            },
            BlackboardError::InvalidPointer {
                value: "foo".into(),
            },
            BlackboardError::SizeLimitExceeded {
                current: 1024,
                max: 256,
            },
            BlackboardError::Backend {
                message: "x".into(),
            },
        ];
        for c in cases {
            let json = serde_json::to_string(&c).unwrap();
            let back: BlackboardError = serde_json::from_str(&json).unwrap();
            assert_eq!(c, back);
        }
    }

    #[test]
    fn eventbus_error_display_non_empty() {
        let e = EventBusError::NoSubscribers;
        assert_eq!(format!("{e}"), "no subscribers");
    }

    #[test]
    fn eventbus_error_serde_round_trip() {
        let e = EventBusError::Backend {
            message: "x".into(),
        };
        let json = serde_json::to_string(&e).unwrap();
        let back: EventBusError = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
    }
}
