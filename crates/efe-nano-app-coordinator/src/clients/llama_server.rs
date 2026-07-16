// SPDX-License-Identifier: UNLICENSED
//! Helper `LlamaServerClient` — la brique partagée par les 4 robots LLM.
//!
//! Encapsule la mécanique HTTP /completion vers `llama-server` (b9297+)
//! avec :
//! - Grammar enforcement via le champ `grammar` du payload
//! - Cache prompt (réutilisation KV cache)
//! - Audit chain wiring automatique
//! - Timeout configurable + classification d'erreurs

use std::time::{Duration, Instant};

use reqwest::Client as HttpClient;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::json;
use tracing::{debug, warn};

use crate::context::{blake3_canonical_json, AuditEntry, CallContext};
use crate::error::{RobotError, RobotResult};

/// Statut de santé d'un robot (renvoyé par `health()`).
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RobotHealth {
    Up,
    Degraded,
    Down,
}

/// Client HTTP réutilisable pour tous les robots LLM.
///
/// Construit une fois au boot via `from_assets(...)` puis cloné/wrappé
/// par les 4 clients concrets.
#[derive(Debug, Clone)]
pub struct LlamaServerClient {
    /// Identifiant kebab-case du robot.
    pub id: &'static str,
    /// Version schéma (SemVer).
    pub schema_version: &'static str,
    /// URL de base, ex: `http://127.0.0.1:8090`.
    pub endpoint: String,
    /// Contenu de la grammar GBNF (passé à chaque requête).
    pub grammar: String,
    /// System prompt court (préfixé à chaque user prompt).
    pub system_prompt: String,
    /// Tokens max générés.
    pub max_tokens: u32,
    /// Température (0.0-0.2 pour déterminisme structuré).
    pub temperature: f32,
    /// Timeout end-to-end.
    pub timeout: Duration,
    /// Client HTTP réutilisable (connection pooling).
    pub http: HttpClient,
}

impl LlamaServerClient {
    /// Construit un nouveau client en lisant grammar + prompt depuis le filesystem.
    ///
    /// Convention : `assets/grammars/{robot}_v{schema_major}.gbnf`
    /// et `assets/prompts/{robot}.txt` relatif à `CARGO_MANIFEST_DIR`.
    pub fn from_assets(
        id: &'static str,
        schema_version: &'static str,
        endpoint: impl Into<String>,
        grammar_path: impl AsRef<std::path::Path>,
        prompt_path: impl AsRef<std::path::Path>,
        max_tokens: u32,
        temperature: f32,
    ) -> RobotResult<Self> {
        let grammar = std::fs::read_to_string(grammar_path.as_ref()).map_err(|e| {
            RobotError::Config {
                robot: id,
                reason: format!("cannot read grammar file: {e}"),
            }
        })?;
        let system_prompt = std::fs::read_to_string(prompt_path.as_ref()).map_err(|e| {
            RobotError::Config {
                robot: id,
                reason: format!("cannot read prompt file: {e}"),
            }
        })?;
        let http = HttpClient::builder()
            .timeout(Duration::from_secs(10))
            .pool_max_idle_per_host(8)
            .build()
            .map_err(|e| RobotError::Config {
                robot: id,
                reason: format!("cannot build HTTP client: {e}"),
            })?;
        Ok(Self {
            id,
            schema_version,
            endpoint: endpoint.into(),
            grammar,
            system_prompt,
            max_tokens,
            temperature,
            timeout: Duration::from_secs(5),
            http,
        })
    }

    /// Construit un client *sans* grammar pour le mode legacy (uniquement
    /// pour benchs comparatifs A/B).
    pub fn from_assets_no_grammar(
        id: &'static str,
        schema_version: &'static str,
        endpoint: impl Into<String>,
        prompt_path: impl AsRef<std::path::Path>,
        max_tokens: u32,
        temperature: f32,
    ) -> RobotResult<Self> {
        let system_prompt = std::fs::read_to_string(prompt_path.as_ref()).map_err(|e| {
            RobotError::Config {
                robot: id,
                reason: format!("cannot read prompt file: {e}"),
            }
        })?;
        let http = HttpClient::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| RobotError::Config {
                robot: id,
                reason: format!("cannot build HTTP client: {e}"),
            })?;
        Ok(Self {
            id,
            schema_version,
            endpoint: endpoint.into(),
            grammar: String::new(),
            system_prompt,
            max_tokens,
            temperature,
            timeout: Duration::from_secs(5),
            http,
        })
    }

    /// **Fil B.1 (additif)** — fixe le timeout end-to-end de `complete` ET
    /// reconstruit le client HTTP avec une marge (+5 s) pour que la borne
    /// reqwest ne coupe pas avant la borne tokio. Nécessaire dès que
    /// `max_tokens` dépasse ce qu'un petit modèle génère en ~5 s (le défaut
    /// historique) : la borne doit suivre le budget de génération.
    pub fn with_timeouts(mut self, total: Duration) -> RobotResult<Self> {
        self.http = HttpClient::builder()
            .timeout(total + Duration::from_secs(5))
            .pool_max_idle_per_host(8)
            .build()
            .map_err(|e| RobotError::Config {
                robot: self.id,
                reason: format!("cannot rebuild HTTP client: {e}"),
            })?;
        self.timeout = total;
        Ok(self)
    }

    /// Cœur du contrat : envoie un prompt à `llama-server` et parse la
    /// réponse en type fort `O` (qui doit correspondre à la grammar GBNF).
    ///
    /// Écrit l'audit entry quelle que soit l'issue (succès ou échec).
    pub async fn complete<I: Serialize, O: DeserializeOwned + Serialize>(
        &self,
        input: &I,
        user_text: &str,
        ctx: &CallContext,
    ) -> RobotResult<O> {
        self.complete_with_grammar(input, user_text, &self.grammar, ctx)
            .await
    }

    /// Comme [`Self::complete`], mais avec une **grammar fournie** au lieu de
    /// `self.grammar` — permet une grammar DYNAMIQUE par requête (compresseur
    /// R5 corr.3 : cap du `summary` proportionnel à `target_words` pour forcer
    /// la brièveté token par token). `grammar` vide ⇒ pas d'enforcement (comme
    /// `self.grammar` vide).
    pub async fn complete_with_grammar<I: Serialize, O: DeserializeOwned + Serialize>(
        &self,
        input: &I,
        user_text: &str,
        grammar: &str,
        ctx: &CallContext,
    ) -> RobotResult<O> {
        let started = Instant::now();
        let input_hash = blake3_canonical_json(input);

        // Construit le prompt complet (system + user).
        let full_prompt = format!("{}\n\n{}", self.system_prompt, user_text);

        // Construit le body JSON (compatible llama-server b9297).
        let mut body = json!({
            "prompt": full_prompt,
            "n_predict": self.max_tokens,
            "temperature": self.temperature,
            "cache_prompt": true,
            "stop": ["\n\n\n"],
        });
        if !grammar.is_empty() {
            body["grammar"] = json!(grammar);
        }

        // Appel HTTP avec timeout.
        let url = format!("{}/completion", self.endpoint);
        let resp_result = tokio::time::timeout(
            self.timeout,
            self.http.post(&url).json(&body).send(),
        )
        .await;

        let resp = match resp_result {
            Err(_) => {
                let err = RobotError::Timeout {
                    robot: self.id,
                    ms: self.timeout.as_millis() as u64,
                };
                self.record_failure(ctx, &input_hash, started, &err).await;
                return Err(err);
            }
            Ok(Err(e)) => {
                let err = RobotError::Transport {
                    robot: self.id,
                    source: Box::new(e),
                };
                self.record_failure(ctx, &input_hash, started, &err).await;
                return Err(err);
            }
            Ok(Ok(r)) => r,
        };

        // Status check.
        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            let body_truncated = body_text.chars().take(512).collect::<String>();
            let err = RobotError::HttpStatus {
                robot: self.id,
                status: status.as_u16(),
                body: body_truncated,
            };
            self.record_failure(ctx, &input_hash, started, &err).await;
            return Err(err);
        }

        // Parse la réponse llama-server.
        let raw: LlamaCompletionResponse =
            resp.json().await.map_err(|e| RobotError::Transport {
                robot: self.id,
                source: Box::new(e),
            })?;

        debug!(
            robot = self.id,
            tokens_in = raw.tokens_evaluated,
            tokens_out = raw.tokens_predicted,
            content_len = raw.content.len(),
            "llama-server response"
        );

        // Parse le JSON contenu dans `content` en type fort O.
        // Sur échec, on enregistre une entry "Failed" (every invocation produces
        // exactly one audit entry — c'est le contrat).
        let parsed: O = match serde_json::from_str(raw.content.trim()) {
            Ok(p) => p,
            Err(e) => {
                warn!(robot = self.id, content = %raw.content, "schema violation");
                let err = RobotError::SchemaViolation {
                    robot: self.id,
                    error: e.to_string(),
                    raw_output: raw.content.chars().take(1024).collect(),
                };
                self.record_failure(ctx, &input_hash, started, &err).await;
                return Err(err);
            }
        };

        let latency_ms = started.elapsed().as_millis() as u64;
        let output_hash = blake3_canonical_json(&parsed);

        // Audit (succès).
        let entry = AuditEntry::success(
            ctx,
            self.id,
            self.schema_version,
            input_hash,
            output_hash,
            raw.tokens_evaluated,
            raw.tokens_predicted,
            latency_ms,
        );
        ctx.audit
            .record(entry)
            .await
            .map_err(|e| RobotError::AuditFailure {
                robot: self.id,
                reason: e.to_string(),
            })?;

        Ok(parsed)
    }

    /// Health check via le endpoint `/health` de `llama-server`.
    pub async fn health(&self) -> RobotResult<RobotHealth> {
        let url = format!("{}/health", self.endpoint);
        let resp = tokio::time::timeout(
            Duration::from_secs(2),
            self.http.get(&url).send(),
        )
        .await;
        match resp {
            Err(_) => Ok(RobotHealth::Down),
            Ok(Err(_)) => Ok(RobotHealth::Down),
            Ok(Ok(r)) if r.status().is_success() => Ok(RobotHealth::Up),
            Ok(Ok(_)) => Ok(RobotHealth::Degraded),
        }
    }

    /// Enregistre une entry de type "failure" — best-effort (ignore les
    /// erreurs d'audit pour ne pas masquer l'erreur d'inférence).
    async fn record_failure(
        &self,
        ctx: &CallContext,
        input_hash: &str,
        started: Instant,
        err: &RobotError,
    ) {
        let entry = AuditEntry::failure(
            ctx,
            self.id,
            self.schema_version,
            input_hash.to_string(),
            started.elapsed().as_millis() as u64,
            err,
        );
        if let Err(audit_err) = ctx.audit.record(entry).await {
            warn!(
                robot = self.id,
                error = %audit_err,
                "failed to record failure audit entry"
            );
        }
    }
}

/// Schéma exact de la réponse `llama-server /completion` (b9297).
///
/// On ne déserialise QUE les champs utilisés ; le reste est ignoré.
#[derive(Debug, Deserialize)]
pub struct LlamaCompletionResponse {
    /// Texte généré (sera parsé en type fort par les clients).
    pub content: String,
    /// Tokens du prompt (utilisé par billing + audit).
    #[serde(default)]
    pub tokens_evaluated: u32,
    /// Tokens générés (utilisé par billing + audit).
    #[serde(default)]
    pub tokens_predicted: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_assets_missing_files_returns_config_error() {
        let result = LlamaServerClient::from_assets(
            "analyzer",
            "1.0.0",
            "http://127.0.0.1:8090",
            "/nonexistent/grammar.gbnf",
            "/nonexistent/prompt.txt",
            512,
            0.1,
        );
        assert!(matches!(result, Err(RobotError::Config { .. })));
    }

    #[test]
    fn llama_response_deserializes_extra_fields() {
        let json = r#"{
            "content": "{}",
            "tokens_evaluated": 14,
            "tokens_predicted": 50,
            "extra_field_ignored": "yes"
        }"#;
        let r: LlamaCompletionResponse = serde_json::from_str(json).unwrap();
        assert_eq!(r.tokens_evaluated, 14);
        assert_eq!(r.tokens_predicted, 50);
    }
}
