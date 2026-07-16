// SPDX-License-Identifier: UNLICENSED
//! **Fil B.0** — l'adapter du VRAI petit modèle (SLM) comme reformulateur,
//! avec repli déterministe **estampillé**, plus le sélecteur [`reformulator_for`].
//!
//! En B.0 l'adapter est **structurellement complet mais DORMANT** : le backend
//! réel ([`NanoRobotBackend`]) répond `supports()==false` car aucun asset SLM
//! aligné sur le vocabulaire canonique multi-llm (plan/code/claim/argument/
//! synthesis) n'existe encore — c'est Fil B.1. Chaque hop **dégrade** donc vers
//! le déterministe, estampillé [`ReformulatorEngine::SlmDegradedToRule`]. La
//! LOGIQUE de l'adapter (succès SLM / dégradation par capacité, erreur ou
//! sortie invalide) est **entièrement testée** via un backend mock ci-dessous.
//!
//! Invariant B.0 : tant qu'aucun client SLM n'est câblé dans le `Coordinator`
//! (cas de **tous** les chemins servis actuels), [`reformulator_for`] renvoie le
//! déterministe — la RÉPONSE des missions est strictement inchangée ; seuls les
//! évènements de provenance gagnent le champ `engine` (= `RULE`).

use std::sync::Arc;

use async_trait::async_trait;
use efe_nano_app_coordinator::ReformulatorClient;

use crate::canonical::{
    ArgumentStance, CanonicalArgument, CanonicalClaim, CanonicalCode, CanonicalPlan,
    CanonicalSchemaId, CanonicalSynthesis,
};
use crate::error::{MultiLlmError, MultiLlmResult};

use super::engine::{Reformulated, ReformulatorEngine};
use super::{CanonicalReformulator, Reformulator, SourceDialect};

/// Backend d'un reformulateur-modèle : la SEULE partie qui parle réellement au
/// petit modèle. Abstrait pour (a) garder l'adapter testable via un mock, et
/// (b) laisser Fil B.1 remplir [`NanoRobotBackend`] sans toucher à l'adapter.
#[async_trait]
pub(crate) trait ReformulatorBackend: Send + Sync {
    /// Le backend sait-il émettre la forme canonique `target` ? (dépend des
    /// assets GBNF livrés en Fil B.1 ; `false` partout en B.0).
    fn supports(&self, target: CanonicalSchemaId) -> bool;

    /// Traduit `raw` (dialecte `from`) vers la forme canonique `target`.
    /// `stance` n'est renseigné que pour `ArgumentV1`. Renvoie la valeur
    /// canonique brute (à valider par l'appelant), ou une erreur textuelle.
    async fn reformulate(
        &self,
        raw: &str,
        from: SourceDialect,
        target: CanonicalSchemaId,
        stance: Option<ArgumentStance>,
    ) -> Result<serde_json::Value, String>;
}

/// Le backend réel, adossé au [`ReformulatorClient`] (Qwen3-0.6B). **Dormant en
/// B.0** : `supports()==false` et `reformulate()` renvoie une erreur explicite.
///
/// Fil B.1 remplira : (i) `supports()` s'ouvrira schéma par schéma selon les
/// assets SLM livrés (grammaire GBNF + prompt émettant les `CanonicalX`) ; (ii)
/// `reformulate()` construira un `ReformulatorInput`, un `CallContext` et
/// appellera `client.invoke(...)`, puis mappera `output.canonical`.
pub(crate) struct NanoRobotBackend {
    // Conservé pour Fil B.1 ; volontairement inutilisé en B.0 (dormant).
    _client: Arc<ReformulatorClient>,
}

impl NanoRobotBackend {
    pub(crate) fn new(client: Arc<ReformulatorClient>) -> Self {
        Self { _client: client }
    }
}

#[async_trait]
impl ReformulatorBackend for NanoRobotBackend {
    fn supports(&self, _target: CanonicalSchemaId) -> bool {
        // Fil B.1 : couverture réelle des assets SLM. En B.0 aucun schéma n'est
        // supporté ⟹ dégradation systématique, estampillée honnêtement.
        false
    }

    async fn reformulate(
        &self,
        _raw: &str,
        _from: SourceDialect,
        _target: CanonicalSchemaId,
        _stance: Option<ArgumentStance>,
    ) -> Result<serde_json::Value, String> {
        Err("SLM reformulateur non câblé (assets Fil B.1 absents) — dégradation déterministe".into())
    }
}

/// Valide qu'une valeur JSON se désérialise bien dans la forme canonique
/// attendue (défense en profondeur au-delà de la grammaire GBNF : on ne fait
/// jamais confiance à une sortie SLM qui ne parse pas dans son `CanonicalX`).
fn validate_canonical(target: CanonicalSchemaId, v: &serde_json::Value) -> bool {
    match target {
        CanonicalSchemaId::PlanV1 => serde_json::from_value::<CanonicalPlan>(v.clone()).is_ok(),
        CanonicalSchemaId::CodeV1 => serde_json::from_value::<CanonicalCode>(v.clone()).is_ok(),
        CanonicalSchemaId::ClaimV1 => serde_json::from_value::<CanonicalClaim>(v.clone()).is_ok(),
        CanonicalSchemaId::ArgumentV1 => {
            serde_json::from_value::<CanonicalArgument>(v.clone()).is_ok()
        }
        CanonicalSchemaId::SynthesisV1 => {
            serde_json::from_value::<CanonicalSynthesis>(v.clone()).is_ok()
        }
    }
}

/// Adapter Fil B : essaie le petit modèle, **dégrade** vers le déterministe
/// (`fallback`) en estampillant, sans jamais échouer là où la règle réussirait.
pub(crate) struct SlmReformulator {
    backend: Arc<dyn ReformulatorBackend>,
    fallback: CanonicalReformulator,
}

impl SlmReformulator {
    /// Construit avec un backend (réel ou mock). Le repli est le déterministe.
    pub(crate) fn new(backend: Arc<dyn ReformulatorBackend>) -> Self {
        Self { backend, fallback: CanonicalReformulator::new() }
    }
}

#[async_trait]
impl Reformulator for SlmReformulator {
    async fn to_canonical(
        &self,
        raw: &str,
        from: SourceDialect,
        target: CanonicalSchemaId,
        llm_source: &str,
    ) -> MultiLlmResult<Reformulated> {
        if self.backend.supports(target) {
            if let Ok(v) = self.backend.reformulate(raw, from, target, None).await {
                if validate_canonical(target, &v) {
                    return Ok(Reformulated::new(v, ReformulatorEngine::Slm));
                }
                // Sortie SLM invalide → on ne la sert JAMAIS telle quelle.
            }
            // Absence / erreur / invalidité → dégradation (ci-dessous).
        }
        // Repli déterministe, connu-bon, estampillé.
        let value = self.fallback.route(raw, from, target, llm_source)?;
        Ok(Reformulated::new(value, ReformulatorEngine::SlmDegradedToRule))
    }

    async fn to_argument_value(
        &self,
        raw: &str,
        stance: ArgumentStance,
        from: SourceDialect,
    ) -> MultiLlmResult<Reformulated> {
        let target = CanonicalSchemaId::ArgumentV1;
        if self.backend.supports(target) {
            if let Ok(v) = self.backend.reformulate(raw, from, target, Some(stance)).await {
                if validate_canonical(target, &v) {
                    return Ok(Reformulated::new(v, ReformulatorEngine::Slm));
                }
            }
        }
        let arg = self.fallback.to_argument(raw, stance, from)?;
        let value = serde_json::to_value(arg).map_err(|e| MultiLlmError::ReformulationFailed {
            from_dialect: from.tag().to_string(),
            to_schema: "canonical/argument_v1".to_string(),
            reason: e.to_string(),
        })?;
        Ok(Reformulated::new(value, ReformulatorEngine::SlmDegradedToRule))
    }
}

/// **Le sélecteur de Fil B.** Choisit le reformulateur d'une mission : le vrai
/// SLM si `coord` porte un client `reformulator` (câblé en Fil B.2), sinon le
/// déterministe.
///
/// **Invariant B.0 :** sans client câblé (tous les chemins servis actuels),
/// renvoie [`CanonicalReformulator`] — comportement strictement inchangé.
#[must_use]
pub fn reformulator_for(coord: &efe_coordinator::Coordinator) -> Box<dyn Reformulator> {
    match coord.clients.reformulator.as_ref() {
        Some(client) => {
            Box::new(SlmReformulator::new(Arc::new(NanoRobotBackend::new(client.clone()))))
        }
        None => Box::new(CanonicalReformulator::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Backend mock configurable : quels schémas il supporte, et quelle sortie
    /// `reformulate` renvoie (`None` ⟹ erreur).
    struct MockBackend {
        supported: Vec<CanonicalSchemaId>,
        output: Option<serde_json::Value>,
    }

    #[async_trait]
    impl ReformulatorBackend for MockBackend {
        fn supports(&self, target: CanonicalSchemaId) -> bool {
            self.supported.contains(&target)
        }
        async fn reformulate(
            &self,
            _raw: &str,
            _from: SourceDialect,
            _target: CanonicalSchemaId,
            _stance: Option<ArgumentStance>,
        ) -> Result<serde_json::Value, String> {
            self.output.clone().ok_or_else(|| "mock backend error".to_string())
        }
    }

    // Fabrique une valeur canonique VALIDE en passant par le déterministe —
    // évite de deviner la forme serde exacte.
    fn valid_claim_json() -> serde_json::Value {
        let c = CanonicalReformulator::new()
            .to_claim("The Fed held rates steady.", SourceDialect::Unknown)
            .unwrap();
        serde_json::to_value(c).unwrap()
    }
    fn valid_argument_json() -> serde_json::Value {
        let a = CanonicalReformulator::new()
            .to_argument(
                "<thesis>Rates should stay</thesis>\n- reason one\n- reason two",
                ArgumentStance::Pro,
                SourceDialect::ClaudeXml,
            )
            .unwrap();
        serde_json::to_value(a).unwrap()
    }

    #[tokio::test]
    async fn slm_success_stamps_slm() {
        let backend = Arc::new(MockBackend {
            supported: vec![CanonicalSchemaId::ClaimV1],
            output: Some(valid_claim_json()),
        });
        let r = SlmReformulator::new(backend);
        let out = r
            .to_canonical(
                "<claim>Fed held</claim>",
                SourceDialect::ClaudeXml,
                CanonicalSchemaId::ClaimV1,
                "claude_x",
            )
            .await
            .unwrap();
        assert_eq!(out.engine, ReformulatorEngine::Slm);
        assert!(out.engine.is_slm());
        assert!(validate_canonical(CanonicalSchemaId::ClaimV1, &out.value));
    }

    #[tokio::test]
    async fn unsupported_schema_degrades_stamped() {
        let backend = Arc::new(MockBackend {
            supported: vec![], // ne supporte rien → dégrade
            output: Some(valid_claim_json()),
        });
        let r = SlmReformulator::new(backend);
        let out = r
            .to_canonical(
                "The Fed held rates steady.",
                SourceDialect::Unknown,
                CanonicalSchemaId::ClaimV1,
                "x",
            )
            .await
            .unwrap();
        assert_eq!(out.engine, ReformulatorEngine::SlmDegradedToRule);
        assert!(validate_canonical(CanonicalSchemaId::ClaimV1, &out.value));
    }

    #[tokio::test]
    async fn slm_error_degrades_stamped() {
        let backend = Arc::new(MockBackend {
            supported: vec![CanonicalSchemaId::ClaimV1],
            output: None, // reformulate → Err
        });
        let r = SlmReformulator::new(backend);
        let out = r
            .to_canonical(
                "The Fed held rates steady.",
                SourceDialect::Unknown,
                CanonicalSchemaId::ClaimV1,
                "x",
            )
            .await
            .unwrap();
        assert_eq!(out.engine, ReformulatorEngine::SlmDegradedToRule);
    }

    #[tokio::test]
    async fn slm_invalid_output_degrades_stamped() {
        // Supporte le schéma MAIS renvoie une valeur hors-schéma → validation
        // échoue → on ne la sert pas, on dégrade.
        let backend = Arc::new(MockBackend {
            supported: vec![CanonicalSchemaId::ClaimV1],
            output: Some(serde_json::json!({"garbage": true})),
        });
        let r = SlmReformulator::new(backend);
        let out = r
            .to_canonical(
                "The Fed held rates steady.",
                SourceDialect::Unknown,
                CanonicalSchemaId::ClaimV1,
                "x",
            )
            .await
            .unwrap();
        assert_eq!(out.engine, ReformulatorEngine::SlmDegradedToRule);
        assert!(validate_canonical(CanonicalSchemaId::ClaimV1, &out.value));
    }

    #[tokio::test]
    async fn argument_slm_success() {
        let backend = Arc::new(MockBackend {
            supported: vec![CanonicalSchemaId::ArgumentV1],
            output: Some(valid_argument_json()),
        });
        let r = SlmReformulator::new(backend);
        let out = r
            .to_argument_value("whatever", ArgumentStance::Pro, SourceDialect::ClaudeXml)
            .await
            .unwrap();
        assert_eq!(out.engine, ReformulatorEngine::Slm);
    }

    #[tokio::test]
    async fn argument_degrades_via_fallback_stamped() {
        let backend = Arc::new(MockBackend { supported: vec![], output: None });
        let r = SlmReformulator::new(backend);
        let out = r
            .to_argument_value(
                "<thesis>Rates should stay</thesis>\n- reason one",
                ArgumentStance::Pro,
                SourceDialect::ClaudeXml,
            )
            .await
            .unwrap();
        assert_eq!(out.engine, ReformulatorEngine::SlmDegradedToRule);
        assert!(validate_canonical(CanonicalSchemaId::ArgumentV1, &out.value));
    }

    /// Le backend RÉEL (dormant en B.0) dégrade toujours — même « supporté »
    /// serait faux, mais on vérifie surtout que `reformulate` ne panique pas et
    /// que l'adapter produit du déterministe estampillé.
    #[tokio::test]
    async fn real_backend_is_dormant_and_degrades() {
        // On ne construit pas un vrai ReformulatorClient (nécessiterait des
        // assets) ; on vérifie la sémantique via un mock reproduisant B.0 :
        // supports=false partout.
        let backend = Arc::new(MockBackend { supported: vec![], output: None });
        let r = SlmReformulator::new(backend);
        for target in [
            CanonicalSchemaId::PlanV1,
            CanonicalSchemaId::CodeV1,
            CanonicalSchemaId::ClaimV1,
            CanonicalSchemaId::SynthesisV1,
        ] {
            let out = r
                .to_canonical("plan:\n1. do a thing\n2. do another", SourceDialect::Unknown, target, "x")
                .await
                .unwrap();
            assert_eq!(out.engine, ReformulatorEngine::SlmDegradedToRule);
            assert!(validate_canonical(target, &out.value), "degraded output must be valid {target:?}");
        }
    }
}
