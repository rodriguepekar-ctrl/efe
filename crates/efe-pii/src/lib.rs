//! `efe-pii` — M29 PII detection + auto-redaction.
//!
//! Three layers :
//!
//! 1. **Canonicalize** ([`canonicalize`]) — Unicode NFKC + control char
//!    strip + length cap. First line of defence against adversarial
//!    inputs (homoglyphs, zero-width joiners).
//! 2. **Analyze** ([`PiiAnalyzer`] trait + [`PresidioClient`]) — call
//!    Presidio analyzer HTTP API ; returns detected spans.
//! 3. **Apply policy** ([`policy`]) — given the policy
//!    (Off / Detect / Redact / Strict) decide what to do with the
//!    detections.
//!
//! Failure mode : Presidio sidecar unreachable. The circuit breaker
//! ([`circuit_breaker`]) opens after 3 consecutive fails ; subsequent
//! calls return [`AnalysisOutcome::Unavailable`] immediately. Callers
//! decide : fail-open (Off/Detect/Redact policies) vs reject (Strict).
//!
//! ## Adversarial resilience
//!
//! - **Unicode confusables** : NFKC normalises "А" (Cyrillic) to "A".
//! - **Zero-width joiners** : stripped at canonicalize.
//! - **Excessive length** : capped at 4096 chars to bound Presidio
//!   compute time.
//! - **Base64-encoded PII** : not auto-decoded. Customers needing
//!   this layer use a pre-processor before classify.

#![warn(missing_docs)]

pub mod canonicalize;
pub mod circuit_breaker;
pub mod error;
pub mod metrics;
pub mod policy;
pub mod presidio_client;
pub mod redactor;

pub use canonicalize::canonicalize_for_pii;
pub use error::{PiiError, PiiResult};
pub use policy::{AnalysisOutcome, DetectedSpan, PolicyKind};
pub use presidio_client::{PiiAnalyzer, PresidioClient, PresidioConfig};
