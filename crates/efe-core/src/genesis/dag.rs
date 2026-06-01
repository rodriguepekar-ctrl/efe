//! Task DAG — output of the meta-analyzer, input of the router.
//!
//! This module defines three related types:
//!
//! - [`TaskClass`]: enum of cognitive primitives the MVP recognizes.
//! - [`TaskMix`]: validated probability distribution over `TaskClass`.
//! - [`TaskDag`]: full decomposition produced by the meta-analyzer.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::errors::{EfeError, Result};

// =====================================================================
// TaskClass
// =====================================================================

/// High-level categorization of a user's intent.
///
/// MVP covers five orthogonal classes. Each class maps to a distinct
/// template distribution via [`crate::genesis::TemplateRouter`].
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, Ord, PartialOrd)]
#[repr(u8)]
#[non_exhaustive]
pub enum TaskClass {
    /// Fact lookup / direct knowledge retrieval.
    FactoidQa = 0,
    /// Arithmetic or algebraic word problem.
    MathWord = 1,
    /// Deductive / rule-based inference.
    LogicalInference = 2,
    /// Classification / label prediction.
    Classification = 3,
    /// Summarization / compression.
    Summarization = 4,
}

impl TaskClass {
    /// All classes in MVP order.
    pub const MVP_SET: &'static [Self] = &[
        Self::FactoidQa,
        Self::MathWord,
        Self::LogicalInference,
        Self::Classification,
        Self::Summarization,
    ];

    /// Size of MVP_SET, useful for array dimensioning.
    pub const COUNT: usize = 5;

    /// Stable lowercase string name for telemetry / logs.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::FactoidQa => "factoid_qa",
            Self::MathWord => "math_word",
            Self::LogicalInference => "logical_inference",
            Self::Classification => "classification",
            Self::Summarization => "summarization",
        }
    }

    /// Index into [`Self::MVP_SET`].
    #[must_use]
    pub const fn index(&self) -> usize {
        *self as usize
    }

    /// Parse from stable string. Returns `None` on unknown input.
    #[must_use]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "factoid_qa" => Some(Self::FactoidQa),
            "math_word" => Some(Self::MathWord),
            "logical_inference" => Some(Self::LogicalInference),
            "classification" => Some(Self::Classification),
            "summarization" => Some(Self::Summarization),
            _ => None,
        }
    }
}

// =====================================================================
// TaskMix
// =====================================================================

/// Epsilon tolerance for simplex sum validation.
const SIMPLEX_EPSILON: f32 = 1e-5;

/// Probability distribution over [`TaskClass`].
///
/// Invariants:
/// - length equals [`TaskClass::COUNT`]
/// - every probability is finite and >= 0
/// - sum equals 1.0 ± 1e-5
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskMix {
    probabilities: Vec<f32>,
}

impl TaskMix {
    /// Construct a validated distribution.
    ///
    /// # Errors
    /// - [`EfeError::InvalidConfig`] if the vector is malformed.
    pub fn new(probabilities: Vec<f32>) -> Result<Self> {
        if probabilities.len() != TaskClass::COUNT {
            return Err(EfeError::InvalidConfig(format!(
                "TaskMix requires {} probabilities, got {}",
                TaskClass::COUNT,
                probabilities.len()
            )));
        }
        let mut sum = 0.0f32;
        for (i, p) in probabilities.iter().enumerate() {
            if !p.is_finite() {
                return Err(EfeError::InvalidConfig(format!(
                    "TaskMix: non-finite probability at index {i}"
                )));
            }
            if *p < 0.0 {
                return Err(EfeError::InvalidConfig(format!(
                    "TaskMix: negative probability {p} at index {i}"
                )));
            }
            sum += p;
        }
        if (sum - 1.0).abs() > SIMPLEX_EPSILON {
            return Err(EfeError::InvalidConfig(format!(
                "TaskMix: sum {sum} differs from 1.0 by more than {SIMPLEX_EPSILON}"
            )));
        }
        Ok(Self { probabilities })
    }

    /// Uniform distribution over all classes.
    #[must_use]
    pub fn uniform() -> Self {
        let p = 1.0 / TaskClass::COUNT as f32;
        Self {
            probabilities: vec![p; TaskClass::COUNT],
        }
    }

    /// One-hot distribution peaked on `class`.
    #[must_use]
    pub fn one_hot(class: TaskClass) -> Self {
        let mut probs = vec![0.0f32; TaskClass::COUNT];
        probs[class.index()] = 1.0;
        Self {
            probabilities: probs,
        }
    }

    /// Probability of a specific class.
    #[must_use]
    pub fn prob(&self, class: TaskClass) -> f32 {
        self.probabilities[class.index()]
    }

    /// Raw slice (length = [`TaskClass::COUNT`]).
    #[must_use]
    pub fn as_slice(&self) -> &[f32] {
        &self.probabilities
    }

    /// Class with the highest probability. Ties resolved by declaration order.
    #[must_use]
    pub fn argmax(&self) -> TaskClass {
        let (best_idx, _) = self.probabilities.iter().enumerate().fold(
            (0usize, f32::NEG_INFINITY),
            |(best_i, best_p), (i, &p)| {
                if p > best_p {
                    (i, p)
                } else {
                    (best_i, best_p)
                }
            },
        );
        TaskClass::MVP_SET[best_idx]
    }

    /// Shannon entropy in nats (natural log).
    ///
    /// Always in `[0, ln(N)]`.
    ///
    /// **Sprint 5 façade** — delegates to
    /// [`crate::compute::ComputeBackend::shannon_entropy`] (with
    /// `eps = 1e-12` floor) instead of the previous private loop.
    /// Falls back to a minimal in-place computation only when the
    /// distribution is empty (which is structurally impossible for
    /// `TaskMix` but kept defensive).
    #[must_use]
    pub fn entropy(&self) -> f32 {
        if self.probabilities.is_empty() {
            return 0.0;
        }
        // Build a tensor view and call the canonical op. Any error from
        // the backend (negative probability, non-finite) is mapped back
        // to `0.0` because `TaskMix::new` already validates the
        // distribution upstream — reaching the error branch would mean
        // a structural bug we'd rather flag in tests than crash here.
        let backend = crate::compute::global_backend();
        let probs = self.probabilities.as_slice();
        match crate::compute::Tensor::<f32>::from_slice(
            probs,
            [probs.len()].into(),
        ) {
            Ok(t) => match backend.shannon_entropy(&t, -1) {
                Ok(out) => out
                    .as_slice()
                    .map(|s| s[0])
                    .unwrap_or(0.0),
                Err(_) => 0.0,
            },
            Err(_) => 0.0,
        }
    }
}

// =====================================================================
// TaskDag
// =====================================================================

/// Full output of the meta-analyzer.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct TaskDag {
    /// Probability distribution over classes.
    pub task_mix: TaskMix,
    /// Cached entropy of `task_mix` (in nats).
    pub entropy_estimate: f32,
    /// Hard label for logs / telemetry (= task_mix.argmax()).
    pub primary_class: TaskClass,
    /// Problem embedding echoed through the pipeline.
    pub problem_embedding: Arc<[f32]>,
}

impl TaskDag {
    /// Build from a task mix + embedding. Entropy and primary class are
    /// derived automatically.
    #[must_use]
    pub fn from_mix(task_mix: TaskMix, problem_embedding: Arc<[f32]>) -> Self {
        let entropy_estimate = task_mix.entropy();
        let primary_class = task_mix.argmax();
        Self {
            task_mix,
            entropy_estimate,
            primary_class,
            problem_embedding,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_class_as_str_stable() {
        assert_eq!(TaskClass::FactoidQa.as_str(), "factoid_qa");
        assert_eq!(TaskClass::MathWord.as_str(), "math_word");
        assert_eq!(TaskClass::LogicalInference.as_str(), "logical_inference");
        assert_eq!(TaskClass::Classification.as_str(), "classification");
        assert_eq!(TaskClass::Summarization.as_str(), "summarization");
    }

    #[test]
    fn task_class_from_str_roundtrip() {
        for class in TaskClass::MVP_SET {
            assert_eq!(TaskClass::from_str(class.as_str()), Some(*class));
        }
        assert_eq!(TaskClass::from_str("unknown"), None);
    }

    #[test]
    fn task_class_index_matches_mvp_set() {
        for (i, class) in TaskClass::MVP_SET.iter().enumerate() {
            assert_eq!(class.index(), i);
        }
    }

    #[test]
    fn task_mix_uniform_valid() {
        let m = TaskMix::uniform();
        let sum: f32 = m.as_slice().iter().sum();
        assert!((sum - 1.0).abs() < SIMPLEX_EPSILON);
    }

    #[test]
    fn task_mix_one_hot_valid() {
        let m = TaskMix::one_hot(TaskClass::MathWord);
        assert!((m.prob(TaskClass::MathWord) - 1.0).abs() < 1e-6);
        assert!(m.prob(TaskClass::FactoidQa) < 1e-6);
    }

    #[test]
    fn task_mix_rejects_wrong_length() {
        assert!(TaskMix::new(vec![0.5; 4]).is_err());
        assert!(TaskMix::new(vec![0.2; 6]).is_err());
    }

    #[test]
    fn task_mix_rejects_non_simplex() {
        assert!(TaskMix::new(vec![0.5, 0.5, 0.5, 0.5, 0.5]).is_err());
    }

    #[test]
    fn task_mix_rejects_nan_inf() {
        assert!(TaskMix::new(vec![f32::NAN, 0.25, 0.25, 0.25, 0.25]).is_err());
        assert!(TaskMix::new(vec![f32::INFINITY, 0.0, 0.0, 0.0, 0.0]).is_err());
    }

    #[test]
    fn task_mix_rejects_negative() {
        assert!(TaskMix::new(vec![-0.1, 0.275, 0.275, 0.275, 0.275]).is_err());
    }

    #[test]
    fn task_mix_argmax_ties_first() {
        let m = TaskMix::new(vec![0.4, 0.4, 0.1, 0.05, 0.05]).unwrap();
        assert_eq!(m.argmax(), TaskClass::FactoidQa);
    }

    #[test]
    fn task_mix_entropy_uniform_max() {
        let m = TaskMix::uniform();
        let h = m.entropy();
        let expected = (TaskClass::COUNT as f32).ln();
        assert!((h - expected).abs() < 1e-4);
    }

    #[test]
    fn task_mix_entropy_one_hot_zero() {
        let m = TaskMix::one_hot(TaskClass::LogicalInference);
        assert!(m.entropy().abs() < 1e-5);
    }

    #[test]
    fn task_dag_from_mix_caches_derivations() {
        let mix = TaskMix::one_hot(TaskClass::Summarization);
        let emb: Arc<[f32]> = Arc::from(vec![0.1f32, 0.2, 0.3]);
        let dag = TaskDag::from_mix(mix, emb);
        assert_eq!(dag.primary_class, TaskClass::Summarization);
        assert!(dag.entropy_estimate.abs() < 1e-5);
    }
}
