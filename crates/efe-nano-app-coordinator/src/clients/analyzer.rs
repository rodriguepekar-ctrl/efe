// SPDX-License-Identifier: UNLICENSED
//! Client du robot **Analyseur** (SmolLM2-360M Q5).
//!
//! Port par défaut : `8090`.

use std::path::Path;

use async_trait::async_trait;

use super::llama_server::{LlamaServerClient, RobotHealth};
use super::{NanoRobotClient, RunMode};
use crate::context::CallContext;
use crate::error::{RobotError, RobotResult};
use crate::types::analyzer::{AnalyzerInput, AnalyzerOutput};

/// Client pour le robot Analyseur.
pub struct AnalyzerClient {
    inner: LlamaServerClient,
}

impl AnalyzerClient {
    /// Construit avec les assets canoniques (grammar + prompt depuis
    /// `assets/grammars/analyzer_v1.gbnf` et `assets/prompts/analyzer.txt`).
    pub fn from_assets(endpoint: impl Into<String>) -> RobotResult<Self> {
        let assets_root = assets_dir();
        let grammar = assets_root.join("grammars/analyzer_v1.gbnf");
        let prompt = assets_root.join("prompts/analyzer.txt");
        Ok(Self {
            inner: LlamaServerClient::from_assets(
                crate::robot_ids::ANALYZER,
                "1.0.0",
                endpoint,
                grammar,
                prompt,
                /* max_tokens */ 1024,
                /* temperature */ 0.1,
            )?,
        })
    }

    /// Construit explicitement (test / customisation).
    #[must_use]
    pub fn from_helper(inner: LlamaServerClient) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl NanoRobotClient for AnalyzerClient {
    type Input = AnalyzerInput;
    type Output = AnalyzerOutput;

    fn id(&self) -> &'static str {
        crate::robot_ids::ANALYZER
    }
    fn schema_version(&self) -> &'static str {
        "1.0.0"
    }
    fn run_mode(&self) -> RunMode {
        RunMode::B
    }

    async fn invoke(
        &self,
        input: Self::Input,
        ctx: &CallContext,
    ) -> RobotResult<Self::Output> {
        use validator::Validate as _;
        // Validation input pré-vol.
        input.validate().map_err(|e| RobotError::SemanticViolation {
            robot: self.id(),
            issues: format!("input invalid: {e}"),
        })?;

        // Appel + parse JSON garanti par grammar.
        let output: AnalyzerOutput = self.inner.complete(&input, &input.text, ctx).await?;

        // Validation post-parse (les ranges que GBNF ne couvre pas).
        output.validate().map_err(|e| RobotError::SemanticViolation {
            robot: self.id(),
            issues: format!("post-parse invalid: {e}"),
        })?;
        output
            .validate_semantics()
            .map_err(|issues| RobotError::SemanticViolation {
                robot: self.id(),
                issues,
            })?;

        Ok(output)
    }

    async fn health(&self) -> RobotResult<RobotHealth> {
        self.inner.health().await
    }
}

/// Localise `assets/` relativement au manifest du crate.
pub(crate) fn assets_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("assets")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assets_dir_exists() {
        let d = assets_dir();
        assert!(
            d.join("grammars").exists() || !d.exists(),
            "assets/grammars/ should exist when assets/ exists"
        );
    }

    #[test]
    fn id_and_version_are_stable() {
        // On instancie inner manuellement pour ne pas dépendre des fichiers.
        let inner = LlamaServerClient {
            id: crate::robot_ids::ANALYZER,
            schema_version: "1.0.0",
            endpoint: "http://127.0.0.1:8090".into(),
            grammar: String::new(),
            system_prompt: String::new(),
            max_tokens: 512,
            temperature: 0.1,
            timeout: std::time::Duration::from_secs(1),
            http: reqwest::Client::new(),
        };
        let client = AnalyzerClient::from_helper(inner);
        assert_eq!(client.id(), "analyzer");
        assert_eq!(client.schema_version(), "1.0.0");
        assert_eq!(client.run_mode(), RunMode::B);
    }
}
