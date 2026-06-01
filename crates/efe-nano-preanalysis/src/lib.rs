// SPDX-License-Identifier: UNLICENSED
//! # `efe-nano-preanalysis` — M50 Sprint 13 N21.
//!
//! Six reference pre-analysis agents that implement the `SwarmAgent`
//! trait from `efe-swarm-orchestrator`.  Each agent reads from the
//! shared `Blackboard` and (typically) writes a structured field at
//! a canonical JSON pointer (§ A3 of the N21 architecture doc).
//!
//! | Agent | Mode | Reads | Writes |
//! |---|---|---|---|
//! | [`ner_extractor`] | Mode A | `/input/text` | `/entities` |
//! | [`sentiment_classifier`] | Mode B | `/input/text` | `/sentiment` |
//! | [`language_detector`] | Mode A | `/input/text` | `/language` |
//! | [`pii_redactor`] | Mode A | `/input/text` | `/redactions` + `/redacted_text` |
//! | [`summary_writer`] | Mode C | `/input/text` | `/summary` |
//! | [`schema_validator`] | Mode A | `/input` | `/schema_valid` |

#![deny(unsafe_code)]
#![warn(rust_2018_idioms)]

/// **Sprint 15 — N23 + G12** : synthetic catalogue back-fill so
/// the six pre-analysis agents flow through the N23 adapter
/// (signature gate, gold gate, idempotency cache, envelope).
pub mod catalog;

pub mod language_detector;
pub mod ner_extractor;
pub mod pii_redactor;
pub mod schema_validator;
pub mod sentiment_classifier;
pub mod summary_writer;

pub use catalog::{
    canonical_hash, default_catalogue, language_detector_descriptor,
    language_detector_runner, ner_extractor_descriptor, ner_extractor_runner,
    pii_redactor_descriptor, pii_redactor_runner, schema_validator_descriptor,
    schema_validator_runner, sentiment_classifier_descriptor, sentiment_classifier_runner,
    summary_writer_descriptor, summary_writer_runner, SwarmAgentNanoRunner,
};
pub use language_detector::LanguageDetector;
pub use ner_extractor::NerExtractor;
pub use pii_redactor::PiiRedactor;
pub use schema_validator::SchemaValidator;
pub use sentiment_classifier::SentimentClassifier;
pub use summary_writer::SummaryWriter;

/// **N21** : Crate semver.
pub const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// **N21 — A3** : canonical input pointer expected by all pre-
/// analysis agents.
pub const INPUT_TEXT_POINTER: &str = "/input/text";
