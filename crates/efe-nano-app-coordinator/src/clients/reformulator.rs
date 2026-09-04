// SPDX-License-Identifier: UNLICENSED
//! Client du robot **Reformulateur** (Qwen3-0.6B Q5).
//!
//! Port par défaut : `8091`.

use async_trait::async_trait;

use super::llama_server::{LlamaServerClient, RobotAssets, RobotHealth};
use super::{NanoRobotClient, RunMode};
use crate::clients::analyzer::assets_dir;
use crate::context::CallContext;
use crate::error::{RobotError, RobotResult};
use crate::types::reformulator::{ReformulatorInput, ReformulatorOutput};

/// **Profil d'assets** du Reformulateur — quel vocabulaire de schémas la
/// grammaire chargée sait émettre. Décide de la *capacité* annoncée aux
/// appelants (cf. `efe-multi-llm::reformulator_ext::slm`) : un client
/// provenance-v1 ne peut PAS produire les formes multi-llm, et inversement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReformulatorProfile {
    /// Assets historiques (`reformulator_v1.gbnf`) — schémas de provenance
    /// (`citation/fact/tool_call/claim/source_ref`), forme `claim_text`.
    ProvenanceV1,
    /// **Fil B.1** — assets multi-llm (`reformulator_multillm_v1.gbnf`) :
    /// les 5 formes canoniques d'`efe-multi-llm`
    /// (`plan/code/claim/argument/synthesis`), liées à `schema_id` par la
    /// grammaire elle-même.
    MultiLlmV1,
}

/// Client pour le robot Reformulateur.
pub struct ReformulatorClient {
    inner: LlamaServerClient,
    profile: ReformulatorProfile,
}

impl ReformulatorClient {
    /// Construit avec les assets canoniques (profil provenance v1).
    pub fn from_assets(endpoint: impl Into<String>) -> RobotResult<Self> {
        let assets = assets_dir();
        Ok(Self {
            inner: LlamaServerClient::from_assets(
                RobotAssets {
                    id: crate::robot_ids::REFORMULATOR,
                    schema_version: "1.0.0",
                    grammar_path: &assets.join("grammars/reformulator_v1.gbnf"),
                    prompt_path: &assets.join("prompts/reformulator.txt"),
                },
                endpoint,
                /* max_tokens */ 1024,
                /* temperature */ 0.1,
            )?,
            profile: ReformulatorProfile::ProvenanceV1,
        })
    }

    /// **Fil B.1** — construit avec les assets MULTI-LLM : la grammaire-union
    /// qui émet les 5 formes canoniques d'`efe-multi-llm` (plan/code/claim/
    /// argument/synthesis), chaque forme LIÉE à son `schema_id` par la
    /// grammaire (mismatch impossible par construction). `n_predict` monte à
    /// 1536 (les plans/synthèses sont plus longs que les citations v1) — et le
    /// timeout suit : 30 s end-to-end (prefill jusqu'à ~15,5 k chars + ~1536
    /// tokens de génération sur Qwen3-0.6B ; les 5 s historiques rendraient ce
    /// budget de tokens inatteignable et forceraient une dégradation
    /// systématique sur toute entrée réelle).
    pub fn from_assets_multillm(endpoint: impl Into<String>) -> RobotResult<Self> {
        let assets = assets_dir();
        let inner = LlamaServerClient::from_assets(
            RobotAssets {
                id: crate::robot_ids::REFORMULATOR,
                schema_version: "1.1.0",
                grammar_path: &assets.join("grammars/reformulator_multillm_v1.gbnf"),
                prompt_path: &assets.join("prompts/reformulator_multillm.txt"),
            },
            endpoint,
            /* max_tokens */ 1536,
            /* temperature */ 0.1,
        )?
        .with_timeouts(std::time::Duration::from_secs(30))?;
        Ok(Self { inner, profile: ReformulatorProfile::MultiLlmV1 })
    }

    /// Construit explicitement (profil provenance v1, historique).
    #[must_use]
    pub fn from_helper(inner: LlamaServerClient) -> Self {
        Self { inner, profile: ReformulatorProfile::ProvenanceV1 }
    }

    /// Construit explicitement avec un profil donné.
    #[must_use]
    pub fn from_helper_with_profile(inner: LlamaServerClient, profile: ReformulatorProfile) -> Self {
        Self { inner, profile }
    }

    /// Le profil d'assets chargé (la capacité honnête de ce client).
    #[must_use]
    pub fn profile(&self) -> ReformulatorProfile {
        self.profile
    }
}

#[async_trait]
impl NanoRobotClient for ReformulatorClient {
    type Input = ReformulatorInput;
    type Output = ReformulatorOutput;

    fn id(&self) -> &'static str {
        crate::robot_ids::REFORMULATOR
    }
    fn schema_version(&self) -> &'static str {
        // Délégué à l'inner : c'est LUI qui porte la version stampée dans les
        // AuditEntry ("1.0.0" provenance / "1.1.0" multi-llm) — une seule
        // source de vérité, aucune dérive possible entre trait et audit.
        self.inner.schema_version
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

        // Construit le user prompt en sérialisant l'input structuré.
        let user_text = serde_json::to_string(&input).map_err(|e| RobotError::Config {
            robot: self.id(),
            reason: format!("input serialization failed: {e}"),
        })?;

        let output: ReformulatorOutput =
            self.inner.complete(&input, &user_text, ctx).await?;

        output.validate().map_err(|e| RobotError::SemanticViolation {
            robot: self.id(),
            issues: format!("post-parse invalid: {e}"),
        })?;

        Ok(output)
    }

    async fn health(&self) -> RobotResult<RobotHealth> {
        self.inner.health().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provenance_profile_reports_v1() {
        let c = ReformulatorClient::from_assets("http://127.0.0.1:1").expect("assets v1");
        assert_eq!(c.profile(), ReformulatorProfile::ProvenanceV1);
        assert_eq!(c.schema_version(), "1.0.0");
    }

    #[test]
    fn multillm_profile_reports_v1_1_and_long_timeout() {
        let c = ReformulatorClient::from_assets_multillm("http://127.0.0.1:1")
            .expect("assets multillm (Fil B.1)");
        assert_eq!(c.profile(), ReformulatorProfile::MultiLlmV1);
        // La version du trait suit l'inner (source unique) — pas de dérive.
        assert_eq!(c.schema_version(), "1.1.0");
        // Le timeout suit le budget n_predict=1536 (30 s, pas les 5 s legacy).
        assert_eq!(c.inner.timeout, std::time::Duration::from_secs(30));
    }
}
