// SPDX-License-Identifier: UNLICENSED
//! **Fil B** — l'adapter du VRAI petit modèle (SLM) comme reformulateur,
//! avec repli déterministe **estampillé**, plus le sélecteur [`reformulator_for`].
//!
//! **B.0** a posé le seam (trait + dégradation estampillée) ; **B.1** réveille
//! le backend réel : [`NanoRobotBackend`] appelle désormais le
//! `ReformulatorClient` (Qwen3-0.6B, grammaire GBNF multi-llm) quand — et
//! seulement quand — le client a été construit avec le **profil multi-llm**
//! ([`ReformulatorProfile::MultiLlmV1`], cf. `from_assets_multillm`). Un client
//! au profil provenance-v1 (grammaire historique) ne sait PAS émettre les
//! formes multi-llm : `supports()==false` ⟹ dégradation estampillée
//! [`ReformulatorEngine::SlmDegradedToRule`] — la capacité annoncée reflète
//! les assets réellement chargés, jamais plus.
//!
//! Invariant inchangé : tant qu'aucun client SLM n'est câblé dans le
//! `Coordinator` (cas de **tous** les chemins servis actuels, jusqu'à B.2),
//! [`reformulator_for`] renvoie le déterministe — la RÉPONSE des missions est
//! strictement identique ; seuls les évènements portent `engine` (= `RULE`).
//!
//! Honnêteté (B.1) : l'audit des invocations SLM part par défaut dans un
//! [`InMemoryAuditWriter`] local (non chaîné) — le raccord au reçu de
//! provenance re-vérifiable est le jalon B.3 (le seam existe :
//! [`SlmReformulator::from_client_with_audit`]). La latence d'un hop SLM est
//! bornée par le timeout du client (30 s en profil multi-llm) mais n'est PAS
//! encore décomptée des deadlines/budgets de mission — B.2/B.4. La qualité de
//! contenu du SLM n'est vérifiée en bout en bout que par les tests live
//! `#[ignore]` (llama-server + poids requis) ; hors ligne, la MÉCANIQUE est
//! testée (construction d'input, clamp, écho de schéma — via un faux
//! llama-server —, validation typée, dégradation), pas le contenu.

use std::sync::Arc;

use async_trait::async_trait;
use efe_nano_app_coordinator::types::{LlmDialect, ReformulatorInput, SchemaId};
use efe_nano_app_coordinator::{
    AuditWriter, CallContext, InMemoryAuditWriter, NanoRobotClient, ReformulatorClient,
    ReformulatorProfile, TenantId,
};
use uuid::Uuid;

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

/// Plafond de caractères passés au SLM — sous la borne de validation de
/// `ReformulatorInput.raw_text` (max 16 000), avec marge.
const SLM_RAW_TEXT_MAX_CHARS: usize = 15_500;

/// Le backend réel, adossé au [`ReformulatorClient`] (Qwen3-0.6B). **Réveillé
/// en Fil B.1** : il invoque réellement le petit modèle quand le client porte
/// le **profil multi-llm** (assets `reformulator_multillm_v1.gbnf` +
/// `reformulator_multillm.txt`, cf. `ReformulatorClient::from_assets_multillm`).
///
/// La capacité annoncée est honnête par construction : un client au profil
/// provenance-v1 (grammaire historique, vocabulaire citation/fact/…) répond
/// `supports()==false` pour TOUS les schémas multi-llm ⟹ l'adapter dégrade,
/// estampillé.
///
/// Audit : chaque invocation SLM produit une `AuditEntry` via le
/// [`CallContext`] (contrat du `LlamaServerClient`). En B.1 le writer est un
/// [`InMemoryAuditWriter`] local (non chaîné au reçu re-vérifiable — B.3).
pub(crate) struct NanoRobotBackend {
    client: Arc<ReformulatorClient>,
    /// Tenant technique de ce backend (un par construction, UUIDv7).
    tenant_id: TenantId,
    /// Writer d'audit des invocations SLM (in-memory en B.1).
    audit: Arc<dyn AuditWriter>,
}

impl NanoRobotBackend {
    pub(crate) fn new(client: Arc<ReformulatorClient>) -> Self {
        Self::with_audit(client, Arc::new(InMemoryAuditWriter::default()))
    }

    /// **Seam B.3** — même backend, mais l'audit part dans le writer fourni
    /// (chaînable au reçu de provenance).
    pub(crate) fn with_audit(client: Arc<ReformulatorClient>, audit: Arc<dyn AuditWriter>) -> Self {
        Self { client, tenant_id: Uuid::now_v7(), audit }
    }

    fn map_dialect(d: SourceDialect) -> LlmDialect {
        match d {
            SourceDialect::ClaudeXml => LlmDialect::ClaudeXml,
            SourceDialect::Gpt4Json => LlmDialect::Gpt4Json,
            SourceDialect::GeminiMarkdown => LlmDialect::GeminiMarkdown,
            SourceDialect::Unknown => LlmDialect::Unknown,
        }
    }

    fn map_schema(s: CanonicalSchemaId) -> SchemaId {
        match s {
            CanonicalSchemaId::PlanV1 => SchemaId::PlanV1,
            CanonicalSchemaId::CodeV1 => SchemaId::CodeV1,
            CanonicalSchemaId::ClaimV1 => SchemaId::ClaimV1,
            CanonicalSchemaId::ArgumentV1 => SchemaId::ArgumentV1,
            CanonicalSchemaId::SynthesisV1 => SchemaId::SynthesisV1,
        }
    }

    fn stance_tag(s: ArgumentStance) -> &'static str {
        match s {
            ArgumentStance::Pro => "PRO",
            ArgumentStance::Contra => "CONTRA",
            ArgumentStance::Neutral => "NEUTRAL",
            ArgumentStance::Rebuttal => "REBUTTAL",
        }
    }
}

#[async_trait]
impl ReformulatorBackend for NanoRobotBackend {
    fn supports(&self, _target: CanonicalSchemaId) -> bool {
        // La grammaire multi-llm couvre les 5 schémas ; le profil provenance-v1
        // n'en couvre AUCUN (vocabulaire différent). La capacité suit les
        // assets réellement chargés — jamais plus.
        self.client.profile() == ReformulatorProfile::MultiLlmV1
    }

    async fn reformulate(
        &self,
        raw: &str,
        from: SourceDialect,
        target: CanonicalSchemaId,
        stance: Option<ArgumentStance>,
    ) -> Result<serde_json::Value, String> {
        if !self.supports(target) {
            return Err(format!(
                "SLM reformulateur : profil {:?} sans support de {target:?} — dégradation déterministe",
                self.client.profile(),
            ));
        }
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err("SLM reformulateur : raw_text vide".into());
        }
        // Clamp sous la borne de validation (16 000 chars), à la frontière char.
        let raw_text: String = trimmed.chars().take(SLM_RAW_TEXT_MAX_CHARS).collect();

        let expected = Self::map_schema(target);
        let input = ReformulatorInput {
            raw_text,
            source_dialect: Self::map_dialect(from),
            target_schema_id: expected,
            stance_hint: stance.map(|s| Self::stance_tag(s).to_string()),
        };
        // Contexte frais par appel (swarm_id UUIDv7 propre à ce hop).
        let ctx = CallContext::new(self.tenant_id, self.audit.clone());

        let output = self
            .client
            .invoke(input, &ctx)
            .await
            .map_err(|e| format!("SLM reformulateur : invoke a échoué : {e}"))?;

        // Écho de schéma : la grammaire lie schema_id à la forme, mais on
        // re-vérifie (défense en profondeur, et garde contre un mauvais asset).
        if output.schema_id != expected {
            return Err(format!(
                "SLM reformulateur : écho de schéma inattendu ({:?} ≠ {expected:?})",
                output.schema_id,
            ));
        }
        Ok(output.canonical)
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
pub struct SlmReformulator {
    backend: Arc<dyn ReformulatorBackend>,
    fallback: CanonicalReformulator,
}

impl SlmReformulator {
    /// Construit avec un backend (réel ou mock). Le repli est le déterministe.
    pub(crate) fn new(backend: Arc<dyn ReformulatorBackend>) -> Self {
        Self { backend, fallback: CanonicalReformulator::new() }
    }

    /// **Fil B.1** — construit l'adapter directement depuis un
    /// [`ReformulatorClient`] (le vrai Qwen3-0.6B). C'est le constructeur
    /// public utilisé par les tests live et, en B.2, par le wiring servi.
    /// La capacité suit le profil du client (multi-llm ⟹ les 5 schémas ;
    /// provenance-v1 ⟹ dégradation systématique, estampillée).
    #[must_use]
    pub fn from_client(client: Arc<ReformulatorClient>) -> Self {
        Self::new(Arc::new(NanoRobotBackend::new(client)))
    }

    /// **Seam B.3** — comme [`Self::from_client`], mais les `AuditEntry` des
    /// invocations SLM partent dans le writer fourni (au lieu d'un in-memory
    /// local) — c'est par ici que B.3 chaînera l'audit au reçu re-vérifiable.
    #[must_use]
    pub fn from_client_with_audit(
        client: Arc<ReformulatorClient>,
        audit: Arc<dyn AuditWriter>,
    ) -> Self {
        Self::new(Arc::new(NanoRobotBackend::with_audit(client, audit)))
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
            match self.backend.reformulate(raw, from, target, None).await {
                Ok(v) if validate_canonical(target, &v) => {
                    return Ok(Reformulated::new(v, ReformulatorEngine::Slm));
                }
                // Sortie SLM hors-schéma → on ne la sert JAMAIS telle quelle.
                Ok(_) => tracing::warn!(
                    ?target, llm_source,
                    "SLM degrade : sortie hors-schéma (validation typée) — repli déterministe"
                ),
                Err(reason) => tracing::warn!(
                    ?target, llm_source, %reason,
                    "SLM degrade : invoke — repli déterministe"
                ),
            }
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
            match self.backend.reformulate(raw, from, target, Some(stance)).await {
                Ok(v) if validate_canonical(target, &v) => {
                    return Ok(Reformulated::new(v, ReformulatorEngine::Slm));
                }
                Ok(_) => tracing::warn!(
                    ?target, dialect = from.tag(),
                    "SLM degrade : sortie hors-schéma (validation typée) — repli déterministe"
                ),
                Err(reason) => tracing::warn!(
                    ?target, dialect = from.tag(), %reason,
                    "SLM degrade : invoke — repli déterministe"
                ),
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
        Some(client) => Box::new(SlmReformulator::from_client(client.clone())),
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

    /// Sémantique « aucun schéma supporté » (profil provenance-v1) : dégrade
    /// toujours, estampillé, avec une sortie déterministe valide.
    #[tokio::test]
    async fn unsupported_profile_degrades_everywhere() {
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

    // ─── Fil B.1 : le VRAI backend (offline — sans llama-server) ─────

    /// Un client au profil PROVENANCE-V1 (assets historiques) n'annonce AUCUN
    /// schéma multi-llm : l'adapter dégrade, estampillé — la capacité suit les
    /// assets réellement chargés.
    #[tokio::test]
    async fn real_backend_legacy_profile_degrades_stamped() {
        // from_assets lit les fichiers v1 (présents dans le repo) ; aucun
        // appel réseau n'a lieu tant que reformulate() n'invoque pas.
        let client = Arc::new(
            ReformulatorClient::from_assets("http://127.0.0.1:1").expect("assets v1 présents"),
        );
        assert_eq!(client.profile(), ReformulatorProfile::ProvenanceV1);
        let r = SlmReformulator::from_client(client);
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

    /// Un client MULTI-LLM (assets B.1) annonce les 5 schémas — et, face à un
    /// endpoint mort, l'invocation échoue proprement ⟹ dégradation estampillée
    /// (jamais une erreur de mission là où la règle réussit).
    #[tokio::test]
    async fn real_backend_multillm_profile_supports_and_degrades_on_dead_endpoint() {
        let client = Arc::new(
            ReformulatorClient::from_assets_multillm("http://127.0.0.1:1")
                .expect("assets multillm présents (Fil B.1)"),
        );
        assert_eq!(client.profile(), ReformulatorProfile::MultiLlmV1);
        let backend = NanoRobotBackend::new(client.clone());
        for target in [
            CanonicalSchemaId::PlanV1,
            CanonicalSchemaId::CodeV1,
            CanonicalSchemaId::ClaimV1,
            CanonicalSchemaId::ArgumentV1,
            CanonicalSchemaId::SynthesisV1,
        ] {
            assert!(backend.supports(target), "multillm doit annoncer {target:?}");
        }
        // Endpoint mort (connexion refusée) ⟹ invoke Err ⟹ dégradation.
        let r = SlmReformulator::from_client(client);
        let out = r
            .to_canonical(
                "The Fed held rates steady.",
                SourceDialect::GeminiMarkdown,
                CanonicalSchemaId::ClaimV1,
                "gemini_x",
            )
            .await
            .unwrap();
        assert_eq!(out.engine, ReformulatorEngine::SlmDegradedToRule);
        assert!(validate_canonical(CanonicalSchemaId::ClaimV1, &out.value));
    }

    /// Le backend refuse un raw_text vide AVANT tout appel réseau (la borne de
    /// validation de `ReformulatorInput` exige ≥ 1 caractère).
    #[tokio::test]
    async fn real_backend_rejects_empty_raw_early() {
        let client = Arc::new(
            ReformulatorClient::from_assets_multillm("http://127.0.0.1:1").expect("assets"),
        );
        let backend = NanoRobotBackend::new(client);
        let err = backend
            .reformulate("   ", SourceDialect::Unknown, CanonicalSchemaId::ClaimV1, None)
            .await
            .unwrap_err();
        assert!(err.contains("vide"), "{err}");
    }

    // ─── Faux llama-server : écho de schéma + clamp, HORS LIGNE ──────

    /// Démarre un faux llama-server qui répond à toute requête POST par une
    /// réponse /completion dont `content` est l'enveloppe fournie. Retourne
    /// l'URL de base.
    async fn spawn_fake_llama(envelope: serde_json::Value) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { break };
                let env = envelope.clone();
                tokio::spawn(async move {
                    // Lit la requête (best-effort ; on ne la parse pas).
                    let mut buf = vec![0u8; 128 * 1024];
                    let _ = sock.read(&mut buf).await;
                    let body = serde_json::json!({
                        "content": env.to_string(),
                        "tokens_evaluated": 10,
                        "tokens_predicted": 20,
                    })
                    .to_string();
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        format!("http://{addr}")
    }

    /// **Écho de schéma** : le faux serveur renvoie une enveloppe `code_v1`
    /// alors qu'on demandait `claim_v1` → le backend refuse (écho ≠ attendu)
    /// et l'adapter dégrade, estampillé. C'est la garde contre un mauvais
    /// asset/modèle qui émettrait le mauvais schéma.
    #[tokio::test]
    async fn real_backend_schema_echo_mismatch_degrades() {
        let code_env = serde_json::json!({
            "canonical": {"language": "rust", "source": "fn a() {}"},
            "schema_id": "code_v1",
            "extraction_warnings": []
        });
        let url = spawn_fake_llama(code_env).await;
        let client = Arc::new(ReformulatorClient::from_assets_multillm(url).expect("assets"));
        let backend = NanoRobotBackend::new(client.clone());
        let err = backend
            .reformulate("The Fed held rates.", SourceDialect::Unknown, CanonicalSchemaId::ClaimV1, None)
            .await
            .unwrap_err();
        assert!(err.contains("écho"), "{err}");
        // Et au niveau adapter : dégradation estampillée, sortie déterministe valide.
        let r = SlmReformulator::from_client(client);
        let out = r
            .to_canonical("The Fed held rates.", SourceDialect::Unknown, CanonicalSchemaId::ClaimV1, "x")
            .await
            .unwrap();
        assert_eq!(out.engine, ReformulatorEngine::SlmDegradedToRule);
    }

    /// **Clamp** : un raw_text de 20 000 chars dépasse la borne de validation
    /// (16 000) — le clamp à 15 500 doit le faire passer, et le hop revient
    /// estampillé `Slm` (le faux serveur renvoie une enveloppe claim valide).
    #[tokio::test]
    async fn real_backend_clamps_oversized_raw_and_succeeds() {
        let claim_env = serde_json::json!({
            "canonical": {
                "claim": "The Fed held rates steady.",
                "evidence": [],
                "confidence": 0.8,
                "domain": "finance"
            },
            "schema_id": "claim_v1",
            "extraction_warnings": []
        });
        let url = spawn_fake_llama(claim_env).await;
        let client = Arc::new(ReformulatorClient::from_assets_multillm(url).expect("assets"));
        let r = SlmReformulator::from_client(client);
        let huge = "x".repeat(20_000); // > 16 000 : sans clamp, invoke échouerait en validation
        let out = r
            .to_canonical(&huge, SourceDialect::Unknown, CanonicalSchemaId::ClaimV1, "x")
            .await
            .unwrap();
        assert_eq!(out.engine, ReformulatorEngine::Slm, "le clamp doit permettre le hop SLM");
        assert!(validate_canonical(CanonicalSchemaId::ClaimV1, &out.value));
    }
}
