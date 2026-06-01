// SPDX-License-Identifier: UNLICENSED
//! **Sprint 11 — N19** : Assertion engine — § 7.5 + ADR-1.
//!
//! Three first-class strategies (`StrictEquality`,
//! `NumericTolerance`, `SemanticSimilarity`) dispatched by
//! `DefaultAssertionEngine`.  Each strategy lives in its own
//! submodule for clarity ; the trait `AssertionEngine` is the
//! cross-strategy API.

use std::sync::Arc;

use async_trait::async_trait;
use efe_nano_sdk::{
    GoldAssertionStrategy, GoldCase, GoldCaseVerdict, GoldErrorKind, GoldFieldDiff,
};
use thiserror::Error;

use crate::embedding::EmbeddingProvider;

pub mod similarity;
pub mod strict;
pub mod tolerance;

pub use similarity::{semantic_similarity, similarity_target_text};
pub use strict::strict_equality;
pub use tolerance::numeric_tolerance;

/// **N19** : closed taxonomy of assertion-engine errors (engine-level,
/// not case-level).  Case-level errors are encoded in
/// `GoldCaseVerdict::Errored`.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum AssertionError {
    #[error("embedding provider : {message}")]
    Embedding { message: String },

    #[error("schema invalid : {message}")]
    SchemaInvalid { message: String },

    #[error("backend : {message}")]
    Backend { message: String },
}

/// **N19** : the assertion engine — evaluate one case at a time.
#[async_trait]
pub trait AssertionEngine: Send + Sync + 'static {
    async fn evaluate(
        &self,
        case: &GoldCase,
        captured: &serde_json::Value,
        expected: &serde_json::Value,
    ) -> Result<GoldCaseVerdict, AssertionError>;
}

/// **N19** : default engine.  Dispatches by strategy ; the
/// `SemanticSimilarity` path uses the injected `EmbeddingProvider`.
pub struct DefaultAssertionEngine {
    embedding_provider: Arc<dyn EmbeddingProvider>,
}

impl std::fmt::Debug for DefaultAssertionEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DefaultAssertionEngine")
            .finish_non_exhaustive()
    }
}

impl DefaultAssertionEngine {
    pub fn new(embedding_provider: Arc<dyn EmbeddingProvider>) -> Self {
        Self { embedding_provider }
    }

    pub fn embedding_provider(&self) -> &Arc<dyn EmbeddingProvider> {
        &self.embedding_provider
    }
}

#[async_trait]
impl AssertionEngine for DefaultAssertionEngine {
    async fn evaluate(
        &self,
        case: &GoldCase,
        captured: &serde_json::Value,
        expected: &serde_json::Value,
    ) -> Result<GoldCaseVerdict, AssertionError> {
        match &case.assertion {
            GoldAssertionStrategy::StrictEquality => Ok(strict_equality(captured, expected)),
            GoldAssertionStrategy::NumericTolerance { fields } => {
                Ok(numeric_tolerance(captured, expected, fields))
            }
            GoldAssertionStrategy::SemanticSimilarity {
                embedding_model,
                cosine_threshold,
                field_pointer,
            } => {
                let actual_text =
                    similarity_target_text(captured, field_pointer.as_deref());
                let expected_text =
                    similarity_target_text(expected, field_pointer.as_deref());
                let verdict = semantic_similarity(
                    self.embedding_provider.as_ref(),
                    embedding_model,
                    *cosine_threshold,
                    &actual_text,
                    &expected_text,
                )
                .await;
                Ok(verdict)
            }
            GoldAssertionStrategy::SchemaValidationOnly => {
                // ADR-1 : `SchemaValidationOnly` is fixture-only ; on
                // a real GoldSet it would have been rejected at
                // `GoldSet::validate`.  Defensive : treat as a no-op
                // pass.  Sprint 11 tests verify this path never
                // reaches an approved gold set.
                Ok(GoldCaseVerdict::Passed { similarity: None })
            }
        }
    }
}

/// **N19** : utility — produce a per-field diff between two JSON
/// values, recursing into objects and arrays.  Used by strict +
/// tolerance reports.
pub fn diff_json(
    actual: &serde_json::Value,
    expected: &serde_json::Value,
    base_pointer: &str,
) -> Vec<GoldFieldDiff> {
    use serde_json::Value;
    let mut out = Vec::new();
    match (actual, expected) {
        (Value::Null, Value::Null) => {}
        (Value::Bool(a), Value::Bool(b)) if a == b => {}
        (Value::Number(a), Value::Number(b)) if a == b => {}
        (Value::String(a), Value::String(b)) if a == b => {}
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (i, (av, bv)) in a.iter().zip(b.iter()).enumerate() {
                let p = format!("{base_pointer}/{i}");
                out.extend(diff_json(av, bv, &p));
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            let mut keys: std::collections::BTreeSet<&String> =
                a.keys().chain(b.keys()).collect();
            // BTreeSet for deterministic order in the diff.
            let _ = &mut keys;
            for k in keys {
                let p = format!(
                    "{base_pointer}/{}",
                    encode_json_pointer_segment(k)
                );
                let av = a.get(k).cloned().unwrap_or(Value::Null);
                let bv = b.get(k).cloned().unwrap_or(Value::Null);
                if av != bv {
                    out.extend(diff_json(&av, &bv, &p));
                }
            }
        }
        (a, b) => out.push(GoldFieldDiff {
            json_pointer: base_pointer.to_string(),
            expected: b.clone(),
            actual: a.clone(),
            epsilon_used: None,
        }),
    }
    out
}

fn encode_json_pointer_segment(s: &str) -> String {
    // RFC 6901 escaping : '~' → '~0', '/' → '~1'.
    s.replace('~', "~0").replace('/', "~1")
}

/// **N19** : try to wrap an `EmbeddingProviderError` into a verdict
/// or an engine error.  Embedding-level rate-limit / drift errors
/// become per-case `Errored` verdicts ; other errors propagate.
pub(crate) fn embedding_error_to_verdict(
    err: crate::embedding::EmbeddingProviderError,
) -> GoldCaseVerdict {
    use crate::embedding::EmbeddingProviderError;
    match err {
        EmbeddingProviderError::RateLimited => GoldCaseVerdict::Errored {
            kind: GoldErrorKind::EmbeddingProviderError,
            message: "rate limited".into(),
        },
        EmbeddingProviderError::ModelDrift { expected, got } => GoldCaseVerdict::Errored {
            kind: GoldErrorKind::EmbeddingProviderError,
            message: format!("model drift : expected {expected}, got {got}"),
        },
        EmbeddingProviderError::Backend { message } => GoldCaseVerdict::Errored {
            kind: GoldErrorKind::EmbeddingProviderError,
            message,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::StubEmbeddingProvider;
    use efe_nano_sdk::{GoldCaseId, GoldExpectedOutput, ModelIdentity};
    use std::collections::BTreeMap;

    fn engine() -> DefaultAssertionEngine {
        DefaultAssertionEngine::new(Arc::new(StubEmbeddingProvider::new()))
    }

    fn case_strict() -> GoldCase {
        GoldCase {
            gold_case_id: GoldCaseId::new(),
            name: "x".into(),
            input: serde_json::json!({"a": 1}),
            expected: GoldExpectedOutput::Inline {
                value: serde_json::json!({"b": 2}),
            },
            assertion: GoldAssertionStrategy::StrictEquality,
            timeout_seconds: 30,
            tags: vec![],
        }
    }

    fn case_tol(eps: f64) -> GoldCase {
        let mut fields = BTreeMap::new();
        fields.insert("/score".into(), eps);
        GoldCase {
            gold_case_id: GoldCaseId::new(),
            name: "x".into(),
            input: serde_json::json!({}),
            expected: GoldExpectedOutput::Inline {
                value: serde_json::json!({"score": 0.5}),
            },
            assertion: GoldAssertionStrategy::NumericTolerance { fields },
            timeout_seconds: 30,
            tags: vec![],
        }
    }

    fn case_sim(threshold: f64) -> GoldCase {
        GoldCase {
            gold_case_id: GoldCaseId::new(),
            name: "x".into(),
            input: serde_json::json!({}),
            expected: GoldExpectedOutput::Inline {
                value: serde_json::json!({"summary": "hello world"}),
            },
            assertion: GoldAssertionStrategy::SemanticSimilarity {
                embedding_model: ModelIdentity::hosted("openai", "text-embedding-3-small", "1"),
                cosine_threshold: threshold,
                field_pointer: Some("/summary".into()),
            },
            timeout_seconds: 30,
            tags: vec![],
        }
    }

    // ─── Engine dispatch ───────────────────────────────────────────

    #[tokio::test]
    async fn engine_dispatches_strict_equality_pass() {
        let e = engine();
        let case = case_strict();
        let actual = serde_json::json!({"b": 2});
        let expected = serde_json::json!({"b": 2});
        let v = e.evaluate(&case, &actual, &expected).await.unwrap();
        assert!(v.is_passed());
    }

    #[tokio::test]
    async fn engine_dispatches_strict_equality_fail() {
        let e = engine();
        let case = case_strict();
        let actual = serde_json::json!({"b": 3});
        let expected = serde_json::json!({"b": 2});
        let v = e.evaluate(&case, &actual, &expected).await.unwrap();
        assert!(!v.is_passed());
    }

    #[tokio::test]
    async fn engine_dispatches_numeric_tolerance_within_epsilon() {
        let e = engine();
        let case = case_tol(0.05);
        let actual = serde_json::json!({"score": 0.51});
        let expected = serde_json::json!({"score": 0.50});
        let v = e.evaluate(&case, &actual, &expected).await.unwrap();
        assert!(v.is_passed());
    }

    #[tokio::test]
    async fn engine_dispatches_numeric_tolerance_outside_epsilon() {
        let e = engine();
        let case = case_tol(0.01);
        let actual = serde_json::json!({"score": 0.7});
        let expected = serde_json::json!({"score": 0.5});
        let v = e.evaluate(&case, &actual, &expected).await.unwrap();
        assert!(!v.is_passed());
    }

    #[tokio::test]
    async fn engine_dispatches_semantic_similarity_self_match_passes() {
        let e = engine();
        let case = case_sim(0.99);
        let actual = serde_json::json!({"summary": "hello world"});
        let expected = serde_json::json!({"summary": "hello world"});
        let v = e.evaluate(&case, &actual, &expected).await.unwrap();
        assert!(v.is_passed());
    }

    #[tokio::test]
    async fn engine_dispatches_semantic_similarity_different_text_fails_high_threshold() {
        let e = engine();
        let case = case_sim(0.99);
        let actual = serde_json::json!({"summary": "completely different text"});
        let expected = serde_json::json!({"summary": "hello world"});
        let v = e.evaluate(&case, &actual, &expected).await.unwrap();
        assert!(!v.is_passed());
    }

    #[tokio::test]
    async fn engine_schema_validation_only_returns_pass_no_op() {
        let mut case = case_strict();
        case.assertion = GoldAssertionStrategy::SchemaValidationOnly;
        let e = engine();
        let actual = serde_json::json!({});
        let expected = serde_json::json!({"b": 2});
        let v = e.evaluate(&case, &actual, &expected).await.unwrap();
        assert!(v.is_passed());
    }

    // ─── diff_json ────────────────────────────────────────────────

    #[test]
    fn diff_json_identical_objects_no_diff() {
        let a = serde_json::json!({"x": 1, "y": "hi"});
        let b = serde_json::json!({"x": 1, "y": "hi"});
        let d = diff_json(&a, &b, "");
        assert!(d.is_empty());
    }

    #[test]
    fn diff_json_different_field_one_diff() {
        let a = serde_json::json!({"x": 1});
        let b = serde_json::json!({"x": 2});
        let d = diff_json(&a, &b, "");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].json_pointer, "/x");
        assert_eq!(d[0].actual, serde_json::json!(1));
        assert_eq!(d[0].expected, serde_json::json!(2));
    }

    #[test]
    fn diff_json_nested_object() {
        let a = serde_json::json!({"outer": {"inner": 1}});
        let b = serde_json::json!({"outer": {"inner": 2}});
        let d = diff_json(&a, &b, "");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].json_pointer, "/outer/inner");
    }

    #[test]
    fn diff_json_array_index_pointer() {
        let a = serde_json::json!([1, 2, 3]);
        let b = serde_json::json!([1, 4, 3]);
        let d = diff_json(&a, &b, "");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].json_pointer, "/1");
    }

    #[test]
    fn diff_json_encodes_pointer_segment() {
        let a = serde_json::json!({"a/b": 1});
        let b = serde_json::json!({"a/b": 2});
        let d = diff_json(&a, &b, "");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].json_pointer, "/a~1b");
    }

    #[test]
    fn debug_format_no_panic() {
        let e = engine();
        let _ = format!("{e:?}");
    }
}
