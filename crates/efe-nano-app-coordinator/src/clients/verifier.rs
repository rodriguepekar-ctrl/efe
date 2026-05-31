// SPDX-License-Identifier: UNLICENSED
//! Client du robot **Vérificateur** (Phi-4-mini Q4).
//!
//! Port par défaut : `8093`.

use async_trait::async_trait;

use super::llama_server::{LlamaServerClient, RobotHealth};
use super::{NanoRobotClient, RunMode};
use crate::clients::analyzer::assets_dir;
use crate::context::CallContext;
use crate::error::{RobotError, RobotResult};
use crate::types::verifier::{VerifierInput, VerifierOutput};

/// Client pour le robot Vérificateur.
pub struct VerifierClient {
    inner: LlamaServerClient,
}

impl VerifierClient {
    /// Construit avec les assets canoniques.
    pub fn from_assets(endpoint: impl Into<String>) -> RobotResult<Self> {
        let assets = assets_dir();
        Ok(Self {
            inner: LlamaServerClient::from_assets(
                crate::robot_ids::VERIFIER,
                "1.0.0",
                endpoint,
                assets.join("grammars/verifier_v1.gbnf"),
                assets.join("prompts/verifier.txt"),
                /* max_tokens */ 768,
                /* temperature */ 0.1,
            )?,
        })
    }

    /// Construit explicitement.
    #[must_use]
    pub fn from_helper(inner: LlamaServerClient) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl NanoRobotClient for VerifierClient {
    type Input = VerifierInput;
    type Output = VerifierOutput;

    fn id(&self) -> &'static str {
        crate::robot_ids::VERIFIER
    }
    fn schema_version(&self) -> &'static str {
        "1.0.0"
    }
    fn run_mode(&self) -> RunMode {
        RunMode::C
    }

    async fn invoke(
        &self,
        input: Self::Input,
        ctx: &CallContext,
    ) -> RobotResult<Self::Output> {
        use validator::Validate as _;
        input.validate().map_err(|e| RobotError::SemanticViolation {
            robot: self.id(),
            issues: format!("input invalid: {e}"),
        })?;

        let user_text = serde_json::to_string(&input).map_err(|e| RobotError::Config {
            robot: self.id(),
            reason: format!("input serialization failed: {e}"),
        })?;

        let output: VerifierOutput =
            self.inner.complete(&input, &user_text, ctx).await?;

        output.validate().map_err(|e| RobotError::SemanticViolation {
            robot: self.id(),
            issues: format!("post-parse invalid: {e}"),
        })?;

        output
            .validate_consistency()
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
