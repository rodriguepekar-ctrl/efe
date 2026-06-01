// SPDX-License-Identifier: UNLICENSED
//! **Sprint 11 — N19** : `SemanticSimilarity` assertion strategy.
//!
//! ADR-3 : cosine similarity via a pinned embedding model.

use efe_nano_sdk::{GoldCaseVerdict, ModelIdentity};

use crate::assertion::embedding_error_to_verdict;
use crate::embedding::EmbeddingProvider;

/// **N19** : extract the comparable text from a JSON value, applying
/// an optional JSON pointer.
///
/// - If `pointer = None`, the whole value is serialised as compact
///   JSON (no whitespace, deterministic key order — serde_json
///   default).
/// - If `pointer = Some(p)` and the pointer resolves to a string,
///   the string is used verbatim.
/// - If the pointer resolves to a non-string value, the value is
///   serialised as JSON.
/// - If the pointer doesn't resolve, the empty string is returned ;
///   the embedding step will assign it a (degenerate) vector.
pub fn similarity_target_text(
    value: &serde_json::Value,
    pointer: Option<&str>,
) -> String {
    match pointer {
        None => serde_json::to_string(value).unwrap_or_default(),
        Some(p) => match value.pointer(p) {
            None => String::new(),
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(other) => serde_json::to_string(other).unwrap_or_default(),
        },
    }
}

/// **N19** : evaluate semantic similarity by embedding both texts
/// and comparing cosine.
pub async fn semantic_similarity(
    provider: &dyn EmbeddingProvider,
    model: &ModelIdentity,
    cosine_threshold: f64,
    actual_text: &str,
    expected_text: &str,
) -> GoldCaseVerdict {
    let actual_emb = match provider.embed(model, actual_text).await {
        Ok(e) => e,
        Err(e) => return embedding_error_to_verdict(e),
    };
    let expected_emb = match provider.embed(model, expected_text).await {
        Ok(e) => e,
        Err(e) => return embedding_error_to_verdict(e),
    };
    // ADR-3 + F10 : the runner is supposed to pin the model ; if the
    // returned embedding's model identity differs from what we asked
    // for, surface as drift.
    if actual_emb.model.lineage_key() != model.lineage_key()
        || expected_emb.model.lineage_key() != model.lineage_key()
    {
        return GoldCaseVerdict::Errored {
            kind: efe_nano_sdk::GoldErrorKind::EmbeddingProviderError,
            message: "embedding provider returned a different model lineage".into(),
        };
    }
    let Some(similarity) = actual_emb.cosine(&expected_emb) else {
        return GoldCaseVerdict::Errored {
            kind: efe_nano_sdk::GoldErrorKind::EmbeddingProviderError,
            message: "cosine returned None — degenerate vector".into(),
        };
    };
    if similarity >= cosine_threshold {
        GoldCaseVerdict::Passed {
            similarity: Some(similarity),
        }
    } else {
        GoldCaseVerdict::Failed {
            reason: format!(
                "cosine {similarity:.4} below threshold {cosine_threshold:.4}"
            ),
            diff: vec![],
            similarity: Some(similarity),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::StubEmbeddingProvider;

    fn model() -> ModelIdentity {
        ModelIdentity::hosted("openai", "text-embedding-3-small", "1")
    }

    // ─── similarity_target_text ────────────────────────────────────

    #[test]
    fn target_text_no_pointer_uses_whole_value() {
        let v = serde_json::json!({"a": 1, "b": "hi"});
        let t = similarity_target_text(&v, None);
        // Either {"a":1,"b":"hi"} or {"b":"hi","a":1} — order is
        // implementation-defined ; we just check it's a non-empty
        // JSON-looking string.
        assert!(t.starts_with('{') && t.ends_with('}'));
    }

    #[test]
    fn target_text_string_pointer_returns_raw_string() {
        let v = serde_json::json!({"summary": "hello world"});
        let t = similarity_target_text(&v, Some("/summary"));
        assert_eq!(t, "hello world");
    }

    #[test]
    fn target_text_non_string_pointer_serialises() {
        let v = serde_json::json!({"score": 0.7});
        let t = similarity_target_text(&v, Some("/score"));
        assert_eq!(t, "0.7");
    }

    #[test]
    fn target_text_missing_pointer_returns_empty() {
        let v = serde_json::json!({"a": 1});
        let t = similarity_target_text(&v, Some("/missing"));
        assert_eq!(t, "");
    }

    // ─── semantic_similarity ───────────────────────────────────────

    #[tokio::test]
    async fn same_text_high_cosine_passes() {
        let p = StubEmbeddingProvider::new();
        let v = semantic_similarity(&p, &model(), 0.99, "hello", "hello").await;
        match v {
            GoldCaseVerdict::Passed { similarity } => {
                assert!(similarity.unwrap() > 0.99);
            }
            _ => panic!("expected pass"),
        }
    }

    #[tokio::test]
    async fn different_text_fails_high_threshold() {
        let p = StubEmbeddingProvider::new();
        let v = semantic_similarity(&p, &model(), 0.99, "alice", "bob").await;
        assert!(!v.is_passed());
        match v {
            GoldCaseVerdict::Failed { similarity, .. } => {
                let s = similarity.unwrap();
                assert!(s < 0.99);
            }
            _ => panic!("expected fail"),
        }
    }

    #[tokio::test]
    async fn rate_limited_provider_yields_errored_verdict() {
        let p = StubEmbeddingProvider::new();
        p.set_force_rate_limit(true);
        let v = semantic_similarity(&p, &model(), 0.5, "x", "y").await;
        match v {
            GoldCaseVerdict::Errored { kind, .. } => {
                assert_eq!(
                    kind,
                    efe_nano_sdk::GoldErrorKind::EmbeddingProviderError
                );
            }
            _ => panic!("expected errored"),
        }
    }

    #[tokio::test]
    async fn drift_simulated_yields_errored_verdict() {
        let p = StubEmbeddingProvider::new();
        p.set_drift_simulated(true);
        let v = semantic_similarity(&p, &model(), 0.5, "x", "y").await;
        match v {
            GoldCaseVerdict::Errored { kind, .. } => {
                assert_eq!(
                    kind,
                    efe_nano_sdk::GoldErrorKind::EmbeddingProviderError
                );
            }
            _ => panic!("expected errored"),
        }
    }

    #[tokio::test]
    async fn threshold_at_boundary_inclusive() {
        // Exact same text → cosine ≥ 0.999999... ; threshold 0.99 →
        // pass (float arithmetic in cosine doesn't guarantee exactly
        // 1.0 for stub-hashed vectors).
        let p = StubEmbeddingProvider::new();
        let v = semantic_similarity(&p, &model(), 0.99, "z", "z").await;
        assert!(v.is_passed());
    }
}
