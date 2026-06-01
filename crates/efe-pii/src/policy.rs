//! Per-tenant PII policy + analysis outcome.
//!
//! The policy comes from `tenant_feature.pii_policy` (column added by
//! migration 0023). Four levels :
//!
//! - `Off` : skip Presidio entirely. Cheapest.
//! - `Detect` : run Presidio, emit audit event with detections, return
//!   input unchanged.
//! - `Redact` : detect AND replace spans with `[REDACTED:TYPE]` before
//!   downstream classify.
//! - `Strict` : detect ; if any high-confidence span found, REJECT the
//!   request with 422 Unprocessable Entity.

use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Per-tenant policy kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolicyKind {
    /// Skip Presidio entirely. Default for new tenants.
    Off,
    /// Detect + audit, but don't modify input.
    Detect,
    /// Detect + redact spans before classify.
    Redact,
    /// Detect + reject the request if any high-confidence span found.
    Strict,
}

impl PolicyKind {
    /// Returns true if this policy requires running the analyzer at all.
    pub fn requires_analysis(&self) -> bool {
        !matches!(self, PolicyKind::Off)
    }

    /// Wire string used in SQL / API JSON.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Detect => "detect",
            Self::Redact => "redact",
            Self::Strict => "strict",
        }
    }
}

impl Default for PolicyKind {
    fn default() -> Self {
        Self::Off
    }
}

impl FromStr for PolicyKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "off" => Ok(Self::Off),
            "detect" => Ok(Self::Detect),
            "redact" => Ok(Self::Redact),
            "strict" => Ok(Self::Strict),
            other => Err(format!("unknown pii_policy : {}", other)),
        }
    }
}

/// A single detected PII span in the input.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DetectedSpan {
    /// Byte offset (UTF-8) into the canonicalized input.
    pub start: usize,
    /// Byte offset (exclusive end).
    pub end: usize,
    /// Entity type label (e.g. `PERSON`, `EMAIL_ADDRESS`).
    pub entity: String,
    /// Confidence 0.0..1.0.
    pub score: f32,
}

impl DetectedSpan {
    /// Returns true if this span exceeds the high-confidence threshold
    /// used by the `Strict` policy.
    pub fn is_high_confidence(&self) -> bool {
        self.score >= 0.85
    }
}

/// Final outcome of an analyze-then-apply pipeline.
#[derive(Debug, Clone, PartialEq)]
pub enum AnalysisOutcome {
    /// No PII detected ; input unchanged.
    Clean(String),
    /// PII detected. If policy was Redact, the redacted form is here ;
    /// otherwise the original.
    Detected {
        /// Either the original text (Detect policy) or the redacted
        /// form (Redact policy).
        text: String,
        /// Detected PII spans with offsets into `text`.
        spans: Vec<DetectedSpan>,
    },
    /// Policy = Strict and at least one high-confidence span found.
    /// Caller should reject the request.
    Rejected {
        /// Spans that caused the rejection. Logged but never emitted
        /// to the caller's response body.
        spans: Vec<DetectedSpan>,
    },
    /// Backend unavailable (circuit open, timeout, etc.). Caller
    /// decides : strict policy → reject ; other policies → fail-open
    /// (use original input).
    Unavailable {
        /// Diagnostic reason for the unavailability (e.g.
        /// "circuit_open", "timeout"). Surfaces in audit log.
        reason: String,
    },
}

impl AnalysisOutcome {
    /// Returns the text the downstream classifier should use, or
    /// `None` if the request should be rejected.
    pub fn classify_input(&self) -> Option<&str> {
        match self {
            Self::Clean(t) | Self::Detected { text: t, .. } => Some(t.as_str()),
            Self::Rejected { .. } => None,
            Self::Unavailable { .. } => None, // caller decides
        }
    }

    /// Number of high-confidence spans (for telemetry).
    pub fn high_confidence_count(&self) -> usize {
        match self {
            Self::Detected { spans, .. } | Self::Rejected { spans } => {
                spans.iter().filter(|s| s.is_high_confidence()).count()
            }
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_kind_roundtrip() {
        for p in &[
            PolicyKind::Off,
            PolicyKind::Detect,
            PolicyKind::Redact,
            PolicyKind::Strict,
        ] {
            let s = p.as_str();
            let back: PolicyKind = s.parse().unwrap();
            assert_eq!(*p, back);
        }
        assert!("bogus".parse::<PolicyKind>().is_err());
    }

    #[test]
    fn requires_analysis() {
        assert!(!PolicyKind::Off.requires_analysis());
        assert!(PolicyKind::Detect.requires_analysis());
        assert!(PolicyKind::Redact.requires_analysis());
        assert!(PolicyKind::Strict.requires_analysis());
    }

    #[test]
    fn high_confidence_threshold() {
        let low = DetectedSpan {
            start: 0,
            end: 5,
            entity: "PERSON".into(),
            score: 0.5,
        };
        let high = DetectedSpan {
            score: 0.95,
            ..low.clone()
        };
        assert!(!low.is_high_confidence());
        assert!(high.is_high_confidence());
    }

    #[test]
    fn classify_input_outcomes() {
        let clean = AnalysisOutcome::Clean("hello".into());
        assert_eq!(clean.classify_input(), Some("hello"));

        let detected = AnalysisOutcome::Detected {
            text: "[REDACTED:EMAIL] hi".into(),
            spans: vec![],
        };
        assert_eq!(detected.classify_input(), Some("[REDACTED:EMAIL] hi"));

        let rejected = AnalysisOutcome::Rejected { spans: vec![] };
        assert_eq!(rejected.classify_input(), None);

        let unav = AnalysisOutcome::Unavailable {
            reason: "circuit open".into(),
        };
        assert_eq!(unav.classify_input(), None);
    }

    #[test]
    fn high_confidence_count() {
        let spans = vec![
            DetectedSpan {
                start: 0,
                end: 5,
                entity: "PERSON".into(),
                score: 0.5,
            },
            DetectedSpan {
                start: 6,
                end: 20,
                entity: "EMAIL_ADDRESS".into(),
                score: 0.95,
            },
            DetectedSpan {
                start: 21,
                end: 25,
                entity: "PHONE".into(),
                score: 0.9,
            },
        ];
        let outcome = AnalysisOutcome::Detected {
            text: "x".into(),
            spans,
        };
        assert_eq!(outcome.high_confidence_count(), 2);
    }
}
