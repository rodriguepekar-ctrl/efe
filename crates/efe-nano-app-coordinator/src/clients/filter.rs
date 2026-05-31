// SPDX-License-Identifier: UNLICENSED
//! Client du robot **Filtreur** (bge-m3 via fastembed-rs ou mock).
//!
//! Robot ONNX : pas de génération texte, juste embeddings + cosine.
//!
//! ## Backends
//!
//! - **`MockEmbedder`** — défaut, déterministe, instantané (tests, dev).
//! - **`BgeM3Embedder`** — production, derrière la feature `fastembed-real`.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;

use super::llama_server::RobotHealth;
use super::{NanoRobotClient, RunMode};
use crate::context::{blake3_canonical_json, AuditEntry, CallContext};
use crate::error::{RobotError, RobotResult};
use crate::types::filter::{AgentRef, FilterInput, FilterOutput, RoutingDecision};

/// Trait abstrayant un backend de génération d'embeddings.
///
/// Implémenté par `MockEmbedder` (test/dev) et `BgeM3Embedder` (prod,
/// derrière la feature `fastembed-real`).
#[async_trait]
pub trait EmbeddingBackend: Send + Sync {
    /// Génère un embedding pour un texte (dim attendue 1024 pour bge-m3).
    async fn embed(&self, text: &str) -> RobotResult<Vec<f32>>;

    /// Dimension du modèle (1024 pour bge-m3).
    fn dimension(&self) -> usize;

    /// Nom du modèle (pour audit + provenance).
    fn model_name(&self) -> &'static str;
}

/// Backend mock : embedding déterministe basé sur BLAKE3.
///
/// Pas un vrai embedding sémantique, mais suffisant pour les tests
/// (deux mêmes textes → même vecteur → cosine = 1.0).
#[derive(Debug)]
pub struct MockEmbedder {
    dim: usize,
}

impl MockEmbedder {
    /// Construit avec une dimension donnée.
    #[must_use]
    pub fn new(dim: usize) -> Self {
        Self { dim }
    }
}

impl Default for MockEmbedder {
    fn default() -> Self {
        Self::new(1024)
    }
}

/// Backend bge-m3 réel via `fastembed-rs` (ONNX Runtime).
///
/// Téléchargé automatiquement depuis HF au premier usage et caché dans
/// `~/.cache/huggingface/`.  Disponible uniquement avec la feature
/// `fastembed-real` (par défaut OFF en CI).
#[cfg(feature = "fastembed-real")]
pub struct BgeM3Embedder {
    inner: tokio::sync::Mutex<fastembed::TextEmbedding>,
}

#[cfg(feature = "fastembed-real")]
impl BgeM3Embedder {
    /// Initialise le modèle (télécharge si absent du cache).
    ///
    /// # Errors
    ///
    /// Retourne `RobotError::Config` si l'initialisation ONNX échoue.
    pub fn new() -> RobotResult<Self> {
        use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
        let model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::BGEM3).with_show_download_progress(false),
        )
        .map_err(|e| RobotError::Config {
            robot: "filter",
            reason: format!("fastembed init failed: {e}"),
        })?;
        Ok(Self {
            inner: tokio::sync::Mutex::new(model),
        })
    }
}

#[cfg(feature = "fastembed-real")]
#[async_trait]
impl EmbeddingBackend for BgeM3Embedder {
    async fn embed(&self, text: &str) -> RobotResult<Vec<f32>> {
        let mut model = self.inner.lock().await;
        let mut out = model
            .embed(vec![text], None)
            .map_err(|e| RobotError::Config {
                robot: "filter",
                reason: format!("fastembed embed failed: {e}"),
            })?;
        out.pop().ok_or_else(|| RobotError::Config {
            robot: "filter",
            reason: "fastembed returned empty vec".into(),
        })
    }

    fn dimension(&self) -> usize {
        1024
    }

    fn model_name(&self) -> &'static str {
        "BAAI/bge-m3"
    }
}

#[async_trait]
impl EmbeddingBackend for MockEmbedder {
    async fn embed(&self, text: &str) -> RobotResult<Vec<f32>> {
        // Déterministe : hash le texte, map les bytes en floats.
        let hash = blake3::hash(text.as_bytes());
        let mut v = Vec::with_capacity(self.dim);
        let mut buf = [0u8; 64];
        let mut reader = hash.as_bytes().iter().copied().cycle();
        while v.len() < self.dim {
            for byte in buf.iter_mut() {
                *byte = reader.next().unwrap_or(0);
            }
            for chunk in buf.chunks_exact(2) {
                let raw = u16::from_le_bytes([chunk[0], chunk[1]]);
                // map u16 → [-1, 1]
                let f = (raw as f32 / u16::MAX as f32) * 2.0 - 1.0;
                v.push(f);
                if v.len() >= self.dim {
                    break;
                }
            }
        }
        // Normalisation L2 pour que cosine = dot product
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in v.iter_mut() {
                *x /= norm;
            }
        }
        Ok(v)
    }

    fn dimension(&self) -> usize {
        self.dim
    }

    fn model_name(&self) -> &'static str {
        "mock-deterministic"
    }
}

/// Client pour le robot Filtreur.
pub struct FilterClient {
    embedder: Arc<dyn EmbeddingBackend>,
    /// Seuil minimum de similarité pour envoyer sans compression (défaut 0.5).
    pub min_similarity_send: f32,
    /// Seuil minimum de similarité pour envoyer avec compression (défaut 0.3).
    /// En-dessous de ce seuil, le robot Skip l'agent.
    pub min_similarity_compress: f32,
}

impl FilterClient {
    /// Construit avec un backend d'embedding fourni + seuils par défaut.
    #[must_use]
    pub fn new(embedder: Arc<dyn EmbeddingBackend>) -> Self {
        Self {
            embedder,
            min_similarity_send: 0.5,
            min_similarity_compress: 0.3,
        }
    }

    /// Variante avec seuils custom.
    #[must_use]
    pub fn with_thresholds(
        embedder: Arc<dyn EmbeddingBackend>,
        min_send: f32,
        min_compress: f32,
    ) -> Self {
        Self {
            embedder,
            min_similarity_send: min_send,
            min_similarity_compress: min_compress,
        }
    }

    /// Construit avec le backend mock (déterministe, sans dep externe).
    #[must_use]
    pub fn mock() -> Self {
        Self::new(Arc::new(MockEmbedder::default()))
    }
}

#[async_trait]
impl NanoRobotClient for FilterClient {
    type Input = FilterInput;
    type Output = FilterOutput;

    fn id(&self) -> &'static str {
        crate::robot_ids::FILTER
    }
    fn schema_version(&self) -> &'static str {
        "1.0.0"
    }
    fn run_mode(&self) -> RunMode {
        RunMode::A
    }

    async fn invoke(
        &self,
        input: Self::Input,
        ctx: &CallContext,
    ) -> RobotResult<Self::Output> {
        use validator::Validate as _;
        let started = Instant::now();
        input.validate().map_err(|e| RobotError::SemanticViolation {
            robot: self.id(),
            issues: format!("input invalid: {e}"),
        })?;

        let input_hash = blake3_canonical_json(&input);

        // 1. Embedding du message (~3ms réel, ~µs mock).
        let msg_emb = self.embedder.embed(&input.message_summary).await?;

        // 2. Pour chaque agent : cosine similarity, puis décision.
        let mut routing: Vec<(AgentRef, RoutingDecision)> =
            Vec::with_capacity(input.available_agents.len());
        for agent in &input.available_agents {
            let cap = input
                .agent_capabilities
                .get(agent)
                .ok_or_else(|| RobotError::SemanticViolation {
                    robot: self.id(),
                    issues: format!("missing capability vector for agent {agent}"),
                })?;
            if cap.len() != msg_emb.len() {
                return Err(RobotError::SemanticViolation {
                    robot: self.id(),
                    issues: format!(
                        "agent {agent} capability dim {} != embedder dim {}",
                        cap.len(),
                        msg_emb.len()
                    ),
                });
            }
            let sim = cosine(&msg_emb, cap);
            let budget = input.context_budgets.get(agent).copied().unwrap_or(0);
            let estimated_tokens = estimate_tokens(&input.message_summary);

            let decision = if sim < self.min_similarity_compress {
                RoutingDecision::Skip {
                    reason: format!("low similarity ({sim:.2})"),
                }
            } else if budget < estimated_tokens {
                RoutingDecision::SendCompressed {
                    ratio: (estimated_tokens as f32 / budget.max(1) as f32).max(2.0),
                }
            } else if sim < self.min_similarity_send {
                RoutingDecision::SendCompressed {
                    ratio: 2.0,
                }
            } else {
                RoutingDecision::Send {
                    priority: ((sim * 10.0).min(10.0)) as u8,
                    reason: format!("similarity {sim:.2}"),
                }
            };
            routing.push((agent.clone(), decision));
        }

        let output = FilterOutput { routing };
        let output_hash = blake3_canonical_json(&output);
        let latency_ms = started.elapsed().as_millis() as u64;

        let entry = AuditEntry::success(
            ctx,
            self.id(),
            self.schema_version(),
            input_hash,
            output_hash,
            0,
            0,
            latency_ms,
        );
        ctx.audit
            .record(entry)
            .await
            .map_err(|e| RobotError::AuditFailure {
                robot: self.id(),
                reason: e.to_string(),
            })?;

        Ok(output)
    }

    async fn health(&self) -> RobotResult<RobotHealth> {
        // Robot in-process : toujours UP si le binaire tourne.
        Ok(RobotHealth::Up)
    }
}

/// Cosine similarity entre deux vecteurs (déjà normalisés idéalement).
#[must_use]
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

/// Estimation grossière du nombre de tokens (≈ 4 chars/token EN/FR).
#[must_use]
pub fn estimate_tokens(text: &str) -> u32 {
    ((text.chars().count() as f32 / 4.0).ceil()) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::AuditWriter as _;
    use std::collections::HashMap;

    #[tokio::test]
    async fn mock_embedder_is_deterministic() {
        let e = MockEmbedder::default();
        let v1 = e.embed("hello world").await.unwrap();
        let v2 = e.embed("hello world").await.unwrap();
        assert_eq!(v1, v2);
        assert_eq!(v1.len(), 1024);
    }

    #[tokio::test]
    async fn mock_embedder_different_for_different_texts() {
        let e = MockEmbedder::default();
        let v1 = e.embed("hello").await.unwrap();
        let v2 = e.embed("world").await.unwrap();
        let s = cosine(&v1, &v2);
        assert!(s.abs() < 0.5, "expected low similarity, got {s}");
    }

    #[test]
    fn cosine_self_is_one() {
        let v = vec![0.3, 0.5, -0.2, 0.1];
        let s = cosine(&v, &v);
        assert!((s - 1.0).abs() < 1e-5);
    }

    #[test]
    fn estimate_tokens_roughly() {
        assert_eq!(estimate_tokens(""), 0);
        let s = "a".repeat(100);
        assert_eq!(estimate_tokens(&s), 25);
    }

    #[tokio::test]
    async fn filter_routes_high_similarity_to_send() {
        let client = FilterClient::mock();
        let ctx = CallContext::mock();

        // Capability vector = embedding du même texte → similarity = 1.0
        let cap = MockEmbedder::default()
            .embed("Fed rate decision")
            .await
            .unwrap();
        let mut caps = HashMap::new();
        caps.insert("claude".to_string(), cap);
        let mut budgets = HashMap::new();
        budgets.insert("claude".to_string(), 10_000);

        let input = FilterInput {
            message_summary: "Fed rate decision".into(),
            available_agents: vec!["claude".into()],
            agent_capabilities: caps,
            context_budgets: budgets,
        };

        let out = client.invoke(input, &ctx).await.unwrap();
        assert_eq!(out.routing.len(), 1);
        assert!(matches!(out.routing[0].1, RoutingDecision::Send { .. }));
    }

    #[tokio::test]
    async fn filter_skips_low_similarity() {
        let client = FilterClient::mock();
        let ctx = CallContext::mock();
        let cap = MockEmbedder::default()
            .embed("completely unrelated topic")
            .await
            .unwrap();
        let mut caps = HashMap::new();
        caps.insert("gemini".to_string(), cap);
        let mut budgets = HashMap::new();
        budgets.insert("gemini".to_string(), 10_000);

        let input = FilterInput {
            message_summary: "Fed rate decision".into(),
            available_agents: vec!["gemini".into()],
            agent_capabilities: caps,
            context_budgets: budgets,
        };
        let out = client.invoke(input, &ctx).await.unwrap();
        assert_eq!(out.routing.len(), 1);
        // Avec mock embedder, deux textes différents → similarity faible.
        // On accepte Skip OU SendCompressed.
        assert!(!matches!(out.routing[0].1, RoutingDecision::Send { .. }));
    }

    #[tokio::test]
    async fn audit_entry_recorded_on_success() {
        let client = FilterClient::mock();
        let writer = Arc::new(crate::context::InMemoryAuditWriter::default());
        let ctx = CallContext::new(uuid::Uuid::now_v7(), writer.clone());

        let cap = MockEmbedder::default().embed("x").await.unwrap();
        let mut caps = HashMap::new();
        caps.insert("claude".to_string(), cap);
        let mut budgets = HashMap::new();
        budgets.insert("claude".to_string(), 1000);

        let input = FilterInput {
            message_summary: "x".into(),
            available_agents: vec!["claude".into()],
            agent_capabilities: caps,
            context_budgets: budgets,
        };
        client.invoke(input, &ctx).await.unwrap();
        let entries = writer.entries().await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].robot_id, "filter");
        assert_eq!(entries[0].status, crate::context::AuditStatus::Success);
    }
}
