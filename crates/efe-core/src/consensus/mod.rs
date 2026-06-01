//! Consensus Detector (M6).
//!
//! M6 is the decision brain of the fusion loop. After each round, M5
//! emits a [`crate::fusion::FusionStats`]; M6 ingests it and returns a
//! [`ConsensusVerdict`] telling M1 (the controller) what to do next:
//!
//! - [`ConsensusVerdict::Continue`] — keep fusing.
//! - [`ConsensusVerdict::Converged`] — hand off to M8 (output projection).
//! - [`ConsensusVerdict::Diverged`] — rollback via M7.
//! - [`ConsensusVerdict::Stalled`] — plateau, operator / policy decides.
//! - [`ConsensusVerdict::Anomaly`] — NaN/Inf, rollback via M7.
//!
//! # Design pillars
//!
//! 1. **Multi-signal** — six signals (`variance_level`, `entropy_drop`,
//!    `centroid_stability`, `outlier_ratio`, `variance_trend`,
//!    `no_anomaly`) combined via a weighted [`ConfidenceScorer`].
//! 2. **Stateful** — a sliding window of the last `W` rounds (default 6)
//!    is kept so trends (Theil-Sen slope on variance) can be detected.
//! 3. **Hysteresis** — a [`ConsensusVerdict::Converged`] requires
//!    `confidence ≥ threshold` on `K` consecutive rounds (default 2).
//! 4. **Zero I/O** — all computation host-side on scalar/vector stats.
//!    Budget: `< 100 µs` per `ingest`.
//!
//! # Usage
//!
//! ```ignore
//! use efe_core::consensus::{
//!     ConsensusConfig, ConsensusDetector, ConsensusVerdict,
//!     MultiCriteriaDetector,
//! };
//!
//! let mut detector = MultiCriteriaDetector::new(ConsensusConfig::default())?;
//!
//! for round in 0..max_rounds {
//!     let fusion_stats = run_round(round);
//!     match detector.ingest(&fusion_stats)? {
//!         ConsensusVerdict::Continue => continue,
//!         ConsensusVerdict::Converged { confidence: _ } => { break; }
//!         other if other.should_rollback() => { break; }
//!         _ => break,
//!     }
//! }
//! ```

pub mod config;
pub mod detector;
pub mod history;
pub mod multi_criteria;
pub mod scorer;
pub mod signals;
pub mod stub;
pub mod verdict;

pub use config::{ConsensusConfig, SignalWeights};
pub use detector::ConsensusDetector;
pub use history::{ConvergenceHistory, StatsRecord};
pub use multi_criteria::MultiCriteriaDetector;
pub use scorer::ConfidenceScorer;
pub use signals::{
    compute_signals, s1_variance_level, s2_entropy_drop, s3_centroid_stability, s4_outlier_ratio,
    s5_variance_trend, s6_no_anomaly, Signals,
};
pub use stub::StubDetector;
pub use verdict::{AnomalyCode, ConsensusVerdict, DivergenceReason};
