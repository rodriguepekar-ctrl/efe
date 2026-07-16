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
//! **B.3** : chaque hop capture l'`AuditEntry` de son invocation SLM (tee
//! par-hop) et la porte dans [`Reformulated::slm_audit`] → les missions la
//! mettent dans `reformulation_hops` (réponse) → la gateway la SCELLE dans le
//! reçu re-vérifiable (nœud Mode-A par hop). Le writer partagé du backend
//! reste alimenté à l'identique ([`InMemoryAuditWriter`] local par défaut ;
//! un writer durable s'injecte via [`SlmReformulator::from_client_with_audit`]
//! — les `entry_id` du reçu croisent alors ses entrées). La latence d'un hop SLM est
//! bornée par le timeout du client (30 s en profil multi-llm) mais n'est PAS
//! encore décomptée des deadlines/budgets de mission — B.4 (le circuit B.2
//! borne la perte non budgétée : 3 échecs ⟹ ouvert, puis 1 seule sonde par
//! fenêtre de reset paie le timeout, les autres hops dégradent sans appel).
//! La qualité de
//! contenu du SLM n'est vérifiée en bout en bout que par les tests live
//! `#[ignore]` (llama-server + poids requis) ; hors ligne, la MÉCANIQUE est
//! testée (construction d'input, clamp, écho de schéma — via un faux
//! llama-server —, validation typée, dégradation), pas le contenu.

use std::sync::Arc;

use async_trait::async_trait;
use efe_nano_app_coordinator::types::{LlmDialect, ReformulatorInput, SchemaId};
use efe_nano_app_coordinator::{
    AuditEntry, AuditWriter, CallContext, InMemoryAuditWriter, NanoRobotClient,
    ReformulatorClient, ReformulatorProfile, RobotResult, SlmCircuitBreaker, TenantId,
};
use uuid::Uuid;

/// **Fil B.2** — clé du circuit breaker du reformulateur SLM dans
/// `CoordinatorClients.slm_circuits`. Symbole partagé avec le wiring servi
/// (efe-swarm-gateway) pour interdire toute dérive de chaîne.
pub const SLM_REFORMULATOR_CIRCUIT_KEY: &str = "reformulator";

use crate::canonical::{
    ArgumentStance, CanonicalArgument, CanonicalClaim, CanonicalCode, CanonicalPlan,
    CanonicalSchemaId, CanonicalSynthesis,
};
use crate::error::{MultiLlmError, MultiLlmResult};

use super::engine::{Reformulated, ReformulatorEngine, SlmHopAudit};
use super::{CanonicalReformulator, Reformulator, SourceDialect};

/// **Fil B.3** — capture par-hop de l'`AuditEntry` d'une invocation SLM.
/// Un slot par hop (créé frais dans `to_canonical`/`to_argument_value`) :
/// pas d'état partagé entre hops, donc pas de course entre missions.
pub(crate) type HopCapture = Arc<std::sync::Mutex<Option<AuditEntry>>>;

fn fresh_capture() -> HopCapture {
    Arc::new(std::sync::Mutex::new(None))
}

fn take_capture(c: &HopCapture) -> Option<SlmHopAudit> {
    c.lock().expect("hop capture mutex poisoned").take().map(|e| SlmHopAudit::from(&e))
}

/// Writer en T : capture la DERNIÈRE `AuditEntry` du hop (le contrat
/// `NanoRobotClient` en garantit exactement une par invoke) tout en déléguant
/// au writer partagé du backend — le seam durable
/// (`from_client_with_audit`) reste alimenté à l'identique.
struct TeeAuditWriter {
    shared: Arc<dyn AuditWriter>,
    capture: HopCapture,
}

#[async_trait]
impl AuditWriter for TeeAuditWriter {
    async fn record(&self, entry: AuditEntry) -> RobotResult<()> {
        *self.capture.lock().expect("hop capture mutex poisoned") = Some(entry.clone());
        self.shared.record(entry).await
    }

    async fn entries(&self) -> Vec<AuditEntry> {
        self.shared.entries().await
    }
}

/// Erreur d'un backend de reformulation — la distinction est **porteuse** :
/// seul un échec du ROBOT (réseau, modèle, écho de schéma) doit compter pour
/// le circuit breaker ; un rejet côté client (entrée inutilisable) ne dit
/// RIEN de la santé du robot et ne doit jamais contribuer à ouvrir le circuit
/// (sinon trois entrées vides bloqueraient des hops sains pendant le reset).
#[derive(Debug)]
pub(crate) enum BackendError {
    /// Entrée inutilisable côté client — dégradation SANS impact circuit.
    ClientSide(String),
    /// Défaillance du robot (invoke/écho) — compte pour le circuit.
    Robot(String),
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClientSide(m) => write!(f, "client-side: {m}"),
            Self::Robot(m) => write!(f, "robot: {m}"),
        }
    }
}

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
    /// canonique brute (à valider par l'appelant), ou une [`BackendError`]
    /// typée (client-side vs robot — décide de l'impact circuit).
    ///
    /// **B.3** : `capture` reçoit l'`AuditEntry` de l'invocation quand une
    /// invocation part réellement — y compris pour un invoke qui ÉCHOUE
    /// (l'entrée `failure` est capturée : le reçu peut committer la
    /// tentative). Les backends sans invocation réelle (mocks) l'ignorent.
    async fn reformulate(
        &self,
        raw: &str,
        from: SourceDialect,
        target: CanonicalSchemaId,
        stance: Option<ArgumentStance>,
        capture: &HopCapture,
    ) -> Result<serde_json::Value, BackendError>;
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
        capture: &HopCapture,
    ) -> Result<serde_json::Value, BackendError> {
        if !self.supports(target) {
            return Err(BackendError::ClientSide(format!(
                "SLM reformulateur : profil {:?} sans support de {target:?} — dégradation déterministe",
                self.client.profile(),
            )));
        }
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(BackendError::ClientSide("SLM reformulateur : raw_text vide".into()));
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
        // Contexte frais par appel (swarm_id UUIDv7 propre à ce hop). B.3 :
        // le writer en T capture l'AuditEntry du hop (succès OU échec) tout
        // en alimentant le writer partagé — le reçu committe la tentative.
        let ctx = CallContext::new(
            self.tenant_id,
            Arc::new(TeeAuditWriter { shared: self.audit.clone(), capture: capture.clone() }),
        );

        let output = self
            .client
            .invoke(input, &ctx)
            .await
            .map_err(|e| BackendError::Robot(format!("SLM reformulateur : invoke a échoué : {e}")))?;

        // Écho de schéma : la grammaire lie schema_id à la forme, mais on
        // re-vérifie (défense en profondeur, et garde contre un mauvais asset).
        if output.schema_id != expected {
            return Err(BackendError::Robot(format!(
                "SLM reformulateur : écho de schéma inattendu ({:?} ≠ {expected:?})",
                output.schema_id,
            )));
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
///
/// **B.2** — un [`SlmCircuitBreaker`] optionnel protège le hop, via la porte
/// à acquisition `try_call` : circuit ouvert ⟹ dégradation SANS appel
/// réseau ; en HalfOpen, **une seule sonde à la fois** paie l'appel réseau
/// (les hops concurrents dégradent sans appel — pas de troupeau vers un
/// llama-server malade) ; un succès validé referme le circuit ; toute
/// défaillance du modèle (invoke, sortie hors-schéma, écho) l'incrémente.
/// Le breaker est `Clone`-partagé (`Arc` interne) : l'état est GLOBAL au
/// coordinator — des échecs répétés à travers les missions ouvrent le
/// circuit pour toutes, exactement l'intention.
pub struct SlmReformulator {
    backend: Arc<dyn ReformulatorBackend>,
    fallback: CanonicalReformulator,
    circuit: Option<SlmCircuitBreaker>,
}

impl SlmReformulator {
    /// Construit avec un backend (réel ou mock). Le repli est le déterministe.
    pub(crate) fn new(backend: Arc<dyn ReformulatorBackend>) -> Self {
        Self { backend, fallback: CanonicalReformulator::new(), circuit: None }
    }

    /// **Fil B.2** — attache un circuit breaker (clone partagé : l'état vit
    /// dans le `CoordinatorClients` d'origine, commun à toutes les missions).
    #[must_use]
    pub fn with_circuit(mut self, circuit: SlmCircuitBreaker) -> Self {
        self.circuit = Some(circuit);
        self
    }

    /// Le circuit autorise-t-il un appel ? (`None` = pas de circuit = oui.)
    ///
    /// Porte à acquisition ([`SlmCircuitBreaker::try_call`]) : en HalfOpen,
    /// un seul hop obtient le slot de sonde ; les autres reçoivent `false`
    /// et dégradent sans appel réseau. Nuance honnête : le bras `ClientSide`
    /// ne rend aucun verdict au circuit — une sonde consommée par une entrée
    /// inutilisable retient le slot jusqu'à la borne de reprise
    /// (`reset_timeout`), jamais plus.
    fn circuit_allows(&self) -> bool {
        self.circuit.as_ref().map_or(true, SlmCircuitBreaker::try_call)
    }

    fn circuit_success(&self) {
        if let Some(c) = &self.circuit {
            c.record_success();
        }
    }

    fn circuit_failure(&self) {
        if let Some(c) = &self.circuit {
            c.record_failure();
        }
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
        // B.3 : capture par-hop de l'AuditEntry — attachée à la sortie sur
        // TOUS les chemins (succès SLM comme dégradation-avec-tentative).
        let capture = fresh_capture();
        if self.backend.supports(target) {
            if !self.circuit_allows() {
                // Circuit ouvert : dégradation SANS appel réseau (B.2).
                tracing::warn!(
                    ?target, llm_source,
                    "SLM degrade : circuit ouvert — repli déterministe sans appel"
                );
            } else {
                match self.backend.reformulate(raw, from, target, None, &capture).await {
                    Ok(v) if validate_canonical(target, &v) => {
                        self.circuit_success();
                        return Ok(Reformulated::new(v, ReformulatorEngine::Slm)
                            .with_slm_audit(take_capture(&capture)));
                    }
                    // Sortie SLM hors-schéma → on ne la sert JAMAIS telle
                    // quelle — et c'est une défaillance du ROBOT (circuit).
                    Ok(_) => {
                        self.circuit_failure();
                        tracing::warn!(
                            ?target, llm_source,
                            "SLM degrade : sortie hors-schéma (validation typée) — repli déterministe"
                        );
                    }
                    Err(BackendError::Robot(reason)) => {
                        self.circuit_failure();
                        tracing::warn!(
                            ?target, llm_source, %reason,
                            "SLM degrade : invoke — repli déterministe"
                        );
                    }
                    // Entrée inutilisable : dégradation SANS impact circuit
                    // (le circuit mesure la santé du robot, pas des entrées).
                    Err(BackendError::ClientSide(reason)) => {
                        tracing::warn!(
                            ?target, llm_source, %reason,
                            "SLM degrade : entrée côté client — repli déterministe (circuit non impacté)"
                        );
                    }
                }
            }
        }
        // Repli déterministe, connu-bon, estampillé — avec, le cas échéant,
        // l'audit de la TENTATIVE SLM (le reçu committe ce qui a été essayé).
        let value = match self.fallback.route(raw, from, target, llm_source) {
            Ok(v) => v,
            Err(e) => {
                // RÉSIDU documenté (revue B.3) : double défaillance — SLM ET
                // repli. L'erreur remonte sans hop, la tentative capturée ne
                // rejoint pas le reçu. Elle reste observable ici, et RETENUE
                // par le writer partagé du backend (croisable par entry_id
                // via `from_client_with_audit`).
                if let Some(a) = take_capture(&capture) {
                    tracing::warn!(
                        entry_id = %a.entry_id, status = %a.status, ?target, llm_source,
                        "double défaillance (SLM + repli) : tentative SLM hors reçu — retenue par le writer partagé"
                    );
                }
                return Err(e);
            }
        };
        Ok(Reformulated::new(value, ReformulatorEngine::SlmDegradedToRule)
            .with_slm_audit(take_capture(&capture)))
    }

    async fn to_argument_value(
        &self,
        raw: &str,
        stance: ArgumentStance,
        from: SourceDialect,
    ) -> MultiLlmResult<Reformulated> {
        let target = CanonicalSchemaId::ArgumentV1;
        let capture = fresh_capture();
        if self.backend.supports(target) {
            if !self.circuit_allows() {
                tracing::warn!(
                    ?target, dialect = from.tag(),
                    "SLM degrade : circuit ouvert — repli déterministe sans appel"
                );
            } else {
                match self.backend.reformulate(raw, from, target, Some(stance), &capture).await {
                    Ok(v) if validate_canonical(target, &v) => {
                        self.circuit_success();
                        return Ok(Reformulated::new(v, ReformulatorEngine::Slm)
                            .with_slm_audit(take_capture(&capture)));
                    }
                    Ok(_) => {
                        self.circuit_failure();
                        tracing::warn!(
                            ?target, dialect = from.tag(),
                            "SLM degrade : sortie hors-schéma (validation typée) — repli déterministe"
                        );
                    }
                    Err(BackendError::Robot(reason)) => {
                        self.circuit_failure();
                        tracing::warn!(
                            ?target, dialect = from.tag(), %reason,
                            "SLM degrade : invoke — repli déterministe"
                        );
                    }
                    Err(BackendError::ClientSide(reason)) => {
                        tracing::warn!(
                            ?target, dialect = from.tag(), %reason,
                            "SLM degrade : entrée côté client — repli déterministe (circuit non impacté)"
                        );
                    }
                }
            }
        }
        let arg = match self.fallback.to_argument(raw, stance, from) {
            Ok(a) => a,
            Err(e) => {
                // Même résidu documenté que `to_canonical` (double défaillance).
                if let Some(a) = take_capture(&capture) {
                    tracing::warn!(
                        entry_id = %a.entry_id, status = %a.status, dialect = from.tag(),
                        "double défaillance (SLM + repli) : tentative SLM hors reçu — retenue par le writer partagé"
                    );
                }
                return Err(e);
            }
        };
        let value = serde_json::to_value(arg).map_err(|e| MultiLlmError::ReformulationFailed {
            from_dialect: from.tag().to_string(),
            to_schema: "canonical/argument_v1".to_string(),
            reason: e.to_string(),
        })?;
        Ok(Reformulated::new(value, ReformulatorEngine::SlmDegradedToRule)
            .with_slm_audit(take_capture(&capture)))
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
            let mut slm = SlmReformulator::from_client(client.clone());
            // B.2 : si le wiring a enregistré un circuit pour le reformulateur,
            // on l'attache (clone partagé — l'état reste global au coordinator).
            if let Some(circuit) = coord.clients.slm_circuit(SLM_REFORMULATOR_CIRCUIT_KEY) {
                slm = slm.with_circuit(circuit.clone());
            }
            Box::new(slm)
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
            _capture: &HopCapture,
        ) -> Result<serde_json::Value, BackendError> {
            self.output.clone().ok_or_else(|| BackendError::Robot("mock backend error".into()))
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
        let capture = fresh_capture();
        let err = backend
            .reformulate("   ", SourceDialect::Unknown, CanonicalSchemaId::ClaimV1, None, &capture)
            .await
            .unwrap_err();
        match err {
            BackendError::ClientSide(m) => assert!(m.contains("vide"), "{m}"),
            other => panic!("attendu ClientSide, obtenu {other}"),
        }
        // B.3 : rejet AVANT invocation ⟹ aucune AuditEntry capturée.
        assert!(take_capture(&capture).is_none(), "pas d'invocation, pas d'audit");
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
        let capture = fresh_capture();
        let err = backend
            .reformulate("The Fed held rates.", SourceDialect::Unknown, CanonicalSchemaId::ClaimV1, None, &capture)
            .await
            .unwrap_err();
        match err {
            BackendError::Robot(m) => assert!(m.contains("écho"), "{m}"),
            other => panic!("attendu Robot (compte pour le circuit), obtenu {other}"),
        }
        // B.3 : une invocation RÉELLE est partie (le faux serveur a répondu) —
        // l'AuditEntry est capturée même si l'écho a été refusé ensuite.
        let attempt = take_capture(&capture).expect("la tentative est capturée");
        assert_eq!(attempt.robot_id, "reformulator");
        assert_eq!(attempt.status, "SUCCESS", "le CLIENT a vu un 200 ; c'est l'écho qui a été refusé");
        // Et au niveau adapter : dégradation estampillée, sortie déterministe
        // valide, ET la tentative SLM portée dans le hop (B.3).
        let r = SlmReformulator::from_client(client);
        let out = r
            .to_canonical("The Fed held rates.", SourceDialect::Unknown, CanonicalSchemaId::ClaimV1, "x")
            .await
            .unwrap();
        assert_eq!(out.engine, ReformulatorEngine::SlmDegradedToRule);
        let audit = out.slm_audit.expect("dégradé-avec-tentative ⟹ audit porté");
        assert_eq!(audit.robot_id, "reformulator");
        assert!(!audit.input_hash.is_empty(), "le reçu committe ce que le SLM a vu");
    }

    // ─── Fil B.2 : le circuit breaker, consommé pour de vrai ─────────

    /// Backend qui COMPTE ses appels et échoue (ou réussit) à la demande —
    /// pour prouver que le circuit ouvert court-circuite l'appel réseau.
    struct CountingBackend {
        calls: std::sync::atomic::AtomicUsize,
        output: Option<serde_json::Value>, // None ⟹ Err
    }

    #[async_trait]
    impl ReformulatorBackend for CountingBackend {
        fn supports(&self, _t: CanonicalSchemaId) -> bool {
            true
        }
        async fn reformulate(
            &self,
            _raw: &str,
            _from: SourceDialect,
            _target: CanonicalSchemaId,
            _stance: Option<ArgumentStance>,
            _capture: &HopCapture,
        ) -> Result<serde_json::Value, BackendError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.output.clone().ok_or_else(|| BackendError::Robot("boom".into()))
        }
    }

    /// Seuil 2 : deux échecs ouvrent le circuit ; le 3ᵉ hop dégrade SANS
    /// appeler le backend (le compteur reste à 2). Tous estampillés dégradés.
    #[tokio::test]
    async fn circuit_opens_after_threshold_and_skips_backend() {
        use efe_nano_app_coordinator::SlmCircuitBreaker;
        let backend = Arc::new(CountingBackend {
            calls: std::sync::atomic::AtomicUsize::new(0),
            output: None,
        });
        let circuit = SlmCircuitBreaker::new(2, std::time::Duration::from_secs(60));
        let r = SlmReformulator::new(backend.clone()).with_circuit(circuit.clone());
        for _ in 0..3 {
            let out = r
                .to_canonical("The Fed held rates.", SourceDialect::Unknown, CanonicalSchemaId::ClaimV1, "x")
                .await
                .unwrap();
            assert_eq!(out.engine, ReformulatorEngine::SlmDegradedToRule);
        }
        assert_eq!(
            backend.calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "le 3ᵉ hop doit être court-circuité (circuit ouvert)"
        );
        assert!(circuit.is_open());
    }

    /// Une sortie HORS-SCHÉMA compte comme échec de circuit (seuil 1 ⟹ le
    /// hop suivant est court-circuité).
    #[tokio::test]
    async fn invalid_output_counts_as_circuit_failure() {
        use efe_nano_app_coordinator::SlmCircuitBreaker;
        let backend = Arc::new(CountingBackend {
            calls: std::sync::atomic::AtomicUsize::new(0),
            output: Some(serde_json::json!({"garbage": true})),
        });
        let circuit = SlmCircuitBreaker::new(1, std::time::Duration::from_secs(60));
        let r = SlmReformulator::new(backend.clone()).with_circuit(circuit);
        for _ in 0..2 {
            let out = r
                .to_canonical("x y z", SourceDialect::Unknown, CanonicalSchemaId::ClaimV1, "x")
                .await
                .unwrap();
            assert_eq!(out.engine, ReformulatorEngine::SlmDegradedToRule);
        }
        assert_eq!(backend.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    /// Après `reset_timeout`, le circuit passe HalfOpen : UN appel de sonde
    /// repart vers le backend ; s'il réussit, le circuit se referme (Closed).
    #[tokio::test]
    async fn circuit_half_opens_then_closes_on_success() {
        use efe_nano_app_coordinator::{SlmCircuitBreaker, SlmCircuitState};
        // Échec d'abord (seuil 1, reset immédiat), puis on bascule le backend
        // en succès via un second adapter partageant le MÊME circuit.
        let failing = Arc::new(CountingBackend {
            calls: std::sync::atomic::AtomicUsize::new(0),
            output: None,
        });
        let circuit = SlmCircuitBreaker::new(1, std::time::Duration::ZERO);
        let r_fail = SlmReformulator::new(failing).with_circuit(circuit.clone());
        let _ = r_fail
            .to_canonical("a b", SourceDialect::Unknown, CanonicalSchemaId::ClaimV1, "x")
            .await
            .unwrap();
        // reset ZERO ⟹ state() bascule aussitôt en HalfOpen (sonde autorisée).
        assert_eq!(circuit.state(), SlmCircuitState::HalfOpen);

        let succeeding = Arc::new(CountingBackend {
            calls: std::sync::atomic::AtomicUsize::new(0),
            output: Some(valid_claim_json()),
        });
        let r_ok = SlmReformulator::new(succeeding.clone()).with_circuit(circuit.clone());
        let out = r_ok
            .to_canonical("a b", SourceDialect::Unknown, CanonicalSchemaId::ClaimV1, "x")
            .await
            .unwrap();
        assert_eq!(out.engine, ReformulatorEngine::Slm, "la sonde HalfOpen passe");
        assert_eq!(succeeding.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(circuit.state(), SlmCircuitState::Closed, "succès ⟹ refermé");
    }

    /// **Le correctif B.2** : une entrée inutilisable (ClientSide) n'impacte
    /// PAS le circuit — trois entrées vides (qui échouent partout, y compris
    /// au repli : on ne fabrique pas de claim depuis du vide) ne doivent pas
    /// bloquer des hops sains ; seule une défaillance ROBOT ouvre le circuit.
    #[tokio::test]
    async fn client_side_errors_do_not_open_the_circuit() {
        use efe_nano_app_coordinator::{SlmCircuitBreaker, SlmCircuitState};
        let client = Arc::new(
            ReformulatorClient::from_assets_multillm("http://127.0.0.1:1").expect("assets"),
        );
        let circuit = SlmCircuitBreaker::new(1, std::time::Duration::from_secs(60));
        let r = SlmReformulator::from_client(client).with_circuit(circuit.clone());
        // 3 entrées vides : ClientSide au backend, et le repli déterministe
        // refuse AUSSI le vide (« empty claim ») — l'erreur remonte à
        // l'appelant (on ne fabrique pas de claim depuis du vide), mais le
        // point épinglé est ailleurs : le circuit reste FERMÉ, car il mesure
        // la santé du ROBOT, pas la qualité des entrées.
        for _ in 0..3 {
            let err = r
                .to_canonical("   ", SourceDialect::Unknown, CanonicalSchemaId::ClaimV1, "x")
                .await
                .expect_err("vide partout : l'erreur doit remonter, pas être maquillée");
            assert!(err.to_string().contains("empty claim"), "{err}");
        }
        assert_eq!(circuit.state(), SlmCircuitState::Closed, "ClientSide ⇏ circuit");
        // Une vraie défaillance robot (endpoint mort) : le circuit s'ouvre.
        let out = r
            .to_canonical("real text", SourceDialect::Unknown, CanonicalSchemaId::ClaimV1, "x")
            .await
            .unwrap();
        assert_eq!(out.engine, ReformulatorEngine::SlmDegradedToRule);
        assert!(circuit.is_open(), "échec ROBOT ⟹ circuit ouvert (seuil 1)");
    }

    /// Sans circuit attaché (`from_client` nu, tests live), rien ne change :
    /// les hops passent, aucun court-circuit.
    #[tokio::test]
    async fn no_circuit_means_no_gating() {
        let backend = Arc::new(CountingBackend {
            calls: std::sync::atomic::AtomicUsize::new(0),
            output: None,
        });
        let r = SlmReformulator::new(backend.clone());
        for _ in 0..4 {
            let out = r
                .to_canonical("a b", SourceDialect::Unknown, CanonicalSchemaId::ClaimV1, "x")
                .await
                .unwrap();
            assert_eq!(out.engine, ReformulatorEngine::SlmDegradedToRule);
        }
        assert_eq!(backend.calls.load(std::sync::atomic::Ordering::SeqCst), 4);
    }

    /// **Revue B.2** — la sonde qui ÉCHOUE ré-ouvre le circuit avec une
    /// fenêtre FRAÎCHE (sans le refresh du timestamp, l'ancienne fenêtre
    /// expirée ferait rebasculer aussitôt en HalfOpen), et exactement UNE
    /// sonde paie l'appel réseau par fenêtre.
    #[tokio::test]
    async fn probe_failure_reopens_with_fresh_window_single_probe() {
        use efe_nano_app_coordinator::{SlmCircuitBreaker, SlmCircuitState};
        let backend = Arc::new(CountingBackend {
            calls: std::sync::atomic::AtomicUsize::new(0),
            output: None,
        });
        let circuit = SlmCircuitBreaker::new(1, std::time::Duration::from_millis(40));
        let r = SlmReformulator::new(backend.clone()).with_circuit(circuit.clone());
        let hop = || r.to_canonical("a b", SourceDialect::Unknown, CanonicalSchemaId::ClaimV1, "x");
        // Hop 1 : échec robot ⟹ Open (fenêtre fraîche).
        assert_eq!(hop().await.unwrap().engine, ReformulatorEngine::SlmDegradedToRule);
        assert_eq!(backend.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(circuit.is_open());
        // Hop 2 immédiat : Open non expiré ⟹ dégradé SANS appel réseau.
        assert_eq!(hop().await.unwrap().engine, ReformulatorEngine::SlmDegradedToRule);
        assert_eq!(backend.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        // Fenêtre écoulée : LA sonde repart (1 appel), échoue ⟹ RE-Open.
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        assert_eq!(hop().await.unwrap().engine, ReformulatorEngine::SlmDegradedToRule);
        assert_eq!(
            backend.calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "exactement UNE sonde par fenêtre"
        );
        // Fenêtre FRAÎCHE : l'ancien timestamp (60 ms > 40 ms) rebasculerait
        // aussitôt en HalfOpen — le refresh doit donner Open.
        assert_eq!(
            circuit.state(),
            SlmCircuitState::Open,
            "réouverture avec fenêtre fraîche, pas l'ancienne expirée"
        );
        // Hop suivant immédiat : court-circuité (le compteur ne bouge pas).
        assert_eq!(hop().await.unwrap().engine, ReformulatorEngine::SlmDegradedToRule);
        assert_eq!(backend.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    /// **Revue B.2** — les bras circuit de `to_argument_value` (jusqu'ici
    /// seuls ceux de `to_canonical` étaient épinglés) : succès SLM referme,
    /// échec robot ouvre, circuit ouvert court-circuite l'appel réseau.
    #[tokio::test]
    async fn to_argument_value_circuit_arms() {
        use efe_nano_app_coordinator::{SlmCircuitBreaker, SlmCircuitState};
        let raw = "<thesis>Rates should stay</thesis>\n- reason one\n- reason two";
        let circuit = SlmCircuitBreaker::new(1, std::time::Duration::from_secs(60));
        // Succès SLM : estampillé Slm, circuit refermé/fermé.
        let ok_backend = Arc::new(CountingBackend {
            calls: std::sync::atomic::AtomicUsize::new(0),
            output: Some(valid_argument_json()),
        });
        let r_ok = SlmReformulator::new(ok_backend.clone()).with_circuit(circuit.clone());
        let out = r_ok
            .to_argument_value(raw, ArgumentStance::Pro, SourceDialect::ClaudeXml)
            .await
            .unwrap();
        assert_eq!(out.engine, ReformulatorEngine::Slm);
        assert_eq!(circuit.state(), SlmCircuitState::Closed);
        // Échec robot (seuil 1) : dégradé estampillé + circuit OUVERT.
        let failing = Arc::new(CountingBackend {
            calls: std::sync::atomic::AtomicUsize::new(0),
            output: None,
        });
        let r_fail = SlmReformulator::new(failing.clone()).with_circuit(circuit.clone());
        let out = r_fail
            .to_argument_value(raw, ArgumentStance::Pro, SourceDialect::ClaudeXml)
            .await
            .unwrap();
        assert_eq!(out.engine, ReformulatorEngine::SlmDegradedToRule);
        assert!(circuit.is_open(), "échec robot via to_argument_value ⟹ ouvert");
        // Circuit ouvert : le hop suivant dégrade SANS appel réseau.
        let out = r_fail
            .to_argument_value(raw, ArgumentStance::Contra, SourceDialect::ClaudeXml)
            .await
            .unwrap();
        assert_eq!(out.engine, ReformulatorEngine::SlmDegradedToRule);
        assert_eq!(
            failing.calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "circuit ouvert ⟹ pas de 2ᵉ appel réseau"
        );
    }

    /// **Revue B.2** — le seam servi de bout en bout : `reformulator_for`
    /// doit ATTACHER le circuit enregistré sous [`SLM_REFORMULATOR_CIRCUIT_KEY`]
    /// dans `coord.clients` (jusqu'ici, seuls des adapters construits à la
    /// main via `.with_circuit()` étaient testés — le lookup + attach du
    /// sélecteur restait non prouvé).
    #[tokio::test]
    async fn reformulator_for_attaches_the_registered_circuit() {
        use efe_nano_app_coordinator::SlmCircuitBreaker;
        let client = Arc::new(
            ReformulatorClient::from_assets_multillm("http://127.0.0.1:1").expect("assets"),
        );
        let circuit = SlmCircuitBreaker::new(1, std::time::Duration::from_secs(60));
        let mut clients = efe_coordinator::CoordinatorClients::default();
        clients.reformulator = Some(client);
        clients
            .slm_circuits
            .insert(SLM_REFORMULATOR_CIRCUIT_KEY.to_string(), circuit.clone());
        let coord = efe_coordinator::Coordinator::builder()
            .registry(efe_coordinator::ProviderRegistry::new())
            .clients(clients)
            .build()
            .expect("coordinator");
        let r = reformulator_for(&coord);
        // Endpoint mort ⟹ échec ROBOT sur le hop : si le sélecteur a bien
        // attaché le clone partagé, c'est le circuit du COORDINATOR qui
        // s'ouvre (seuil 1) — pas un état local perdu.
        let out = r
            .to_canonical(
                "The Fed held rates.",
                SourceDialect::Unknown,
                CanonicalSchemaId::ClaimV1,
                "claude",
            )
            .await
            .unwrap();
        assert_eq!(out.engine, ReformulatorEngine::SlmDegradedToRule);
        assert!(
            circuit.is_open(),
            "reformulator_for doit attacher le circuit enregistré (clé partagée)"
        );
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
        // B.3 : un hop SLM réussi porte ses faits d'audit — le reçu committe
        // input_hash/output_hash/tokens de l'invocation réelle.
        let audit = out.slm_audit.expect("hop SLM ⟹ audit capturé");
        assert_eq!(audit.robot_id, "reformulator");
        assert_eq!(audit.schema_version, "1.1.0", "profil multi-llm");
        assert_eq!(audit.status, "SUCCESS");
        assert!(!audit.input_hash.is_empty() && !audit.output_hash.is_empty());
        assert_eq!((audit.tokens_prompt, audit.tokens_completion), (10, 20), "tokens du faux serveur");
    }

    // ─── Fil B.3 : l'audit SLM porté par hop, chaîné au reçu ─────────

    /// **Revue B.3** — un 200 au corps INDÉCODABLE (serveur crashé en pleine
    /// écriture, proxy qui tronque) était le SEUL chemin terminal sans entrée
    /// d'audit : une invocation réelle devenait invisible du reçu. Épinglé :
    /// la tentative est désormais capturée (statut FAILED).
    #[tokio::test]
    async fn garbled_200_body_attempt_is_captured_in_hop_audit() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { break };
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 64 * 1024];
                    let _ = sock.read(&mut buf).await;
                    let body = "pas du json du tout";
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
        let client = Arc::new(
            ReformulatorClient::from_assets_multillm(format!("http://{addr}")).expect("assets"),
        );
        let r = SlmReformulator::from_client(client);
        let out = r
            .to_canonical("The Fed held rates.", SourceDialect::Unknown, CanonicalSchemaId::ClaimV1, "x")
            .await
            .unwrap();
        assert_eq!(out.engine, ReformulatorEngine::SlmDegradedToRule);
        let audit = out.slm_audit.expect("corps indécodable : la tentative RÉELLE est capturée");
        assert_eq!(audit.status, "FAILED");
        assert!(!audit.input_hash.is_empty(), "ce qui a été envoyé au SLM est committé");
    }

    /// Un invoke qui ÉCHOUE réellement (endpoint mort) capture l'AuditEntry
    /// `FAILED` — le reçu committe la tentative, pas seulement les succès.
    #[tokio::test]
    async fn failed_invoke_attempt_is_captured_in_hop_audit() {
        let client = Arc::new(
            ReformulatorClient::from_assets_multillm("http://127.0.0.1:1").expect("assets"),
        );
        let r = SlmReformulator::from_client(client);
        let out = r
            .to_canonical("The Fed held rates.", SourceDialect::Unknown, CanonicalSchemaId::ClaimV1, "x")
            .await
            .unwrap();
        assert_eq!(out.engine, ReformulatorEngine::SlmDegradedToRule);
        let audit = out.slm_audit.expect("l'invoke est parti (et a échoué) ⟹ tentative capturée");
        assert_eq!(audit.status, "FAILED");
        assert!(!audit.input_hash.is_empty(), "l'entrée tentée est committée");
        assert!(audit.output_hash.is_empty(), "pas de sortie sur un échec transport");
    }

    /// Circuit OUVERT ⟹ aucun appel réseau ⟹ AUCUN audit sur le hop dégradé
    /// (on ne committe pas une tentative qui n'a pas eu lieu).
    #[tokio::test]
    async fn open_circuit_hop_carries_no_slm_audit() {
        use efe_nano_app_coordinator::SlmCircuitBreaker;
        let backend = Arc::new(CountingBackend {
            calls: std::sync::atomic::AtomicUsize::new(0),
            output: None,
        });
        let circuit = SlmCircuitBreaker::new(1, std::time::Duration::from_secs(60));
        let r = SlmReformulator::new(backend).with_circuit(circuit);
        // Hop 1 : échec robot (mock, sans AuditEntry) → ouvre le circuit.
        let _ = r
            .to_canonical("a b", SourceDialect::Unknown, CanonicalSchemaId::ClaimV1, "x")
            .await
            .unwrap();
        // Hop 2 : circuit ouvert → dégradé sans appel → pas d'audit.
        let out = r
            .to_canonical("a b", SourceDialect::Unknown, CanonicalSchemaId::ClaimV1, "x")
            .await
            .unwrap();
        assert_eq!(out.engine, ReformulatorEngine::SlmDegradedToRule);
        assert!(out.slm_audit.is_none(), "pas d'appel ⟹ pas d'audit committé");
    }

    /// Le tee alimente AUSSI le writer partagé (seam `from_client_with_audit`) :
    /// les `entry_id` portés par les hops croisent les entrées du writer durable.
    #[tokio::test]
    async fn shared_audit_writer_still_receives_entries() {
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
        let shared = Arc::new(InMemoryAuditWriter::default());
        let r = SlmReformulator::from_client_with_audit(client, shared.clone());
        let out = r
            .to_canonical("Rates held.", SourceDialect::Unknown, CanonicalSchemaId::ClaimV1, "x")
            .await
            .unwrap();
        assert_eq!(out.engine, ReformulatorEngine::Slm);
        let hop_audit = out.slm_audit.expect("audit du hop");
        let entries = shared.entries().await;
        assert_eq!(entries.len(), 1, "le writer partagé reçoit l'entrée à l'identique");
        assert_eq!(entries[0].entry_id, hop_audit.entry_id, "croisable par entry_id");
        assert_eq!(entries[0].input_hash, hop_audit.input_hash);
    }

    /// **DoD Fil B (§12)** — `reformulator_slm_stamped_when_url_set` : une
    /// mission SERVIE via `reformulator_for` (le sélecteur du chemin servi)
    /// contre un llama-server (faux) : chaque hop de la réponse est estampillé
    /// `SLM` et porte ses faits d'audit. C'est la revendication « relais médié
    /// par le petit modèle », re-vérifiable depuis le reçu.
    #[tokio::test]
    async fn reformulator_slm_stamped_when_url_set() {
        use efe_coordinator::{Coordinator, MockLlmProvider, ProviderRegistry};
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
        let reg = ProviderRegistry::new();
        for id in ["claude_x", "gpt_x", "gemini_x"] {
            reg.register(Arc::new(MockLlmProvider::new(id).with_response("The Fed held rates.")));
        }
        let mut clients = efe_coordinator::CoordinatorClients::default();
        clients.reformulator =
            Some(Arc::new(ReformulatorClient::from_assets_multillm(url).expect("assets")));
        let coord = Coordinator::builder().registry(reg).clients(clients).build().expect("coord");

        let req = crate::request::Vote3LlmRequest {
            user_text: "Did the Fed hold rates?".into(),
            task_class: efe_nano_app_coordinator::types::router::TaskClass::FactualQa,
            llms: vec!["claude_x".into(), "gpt_x".into(), "gemini_x".into()],
            mode: crate::request::VoteMode::Plurality,
            quorum: None,
            max_budget_cents: 100,
            max_total_latency: std::time::Duration::from_secs(60),
        };
        let resp = crate::workflows::run_vote_3llm(&coord, req).await.expect("vote");
        assert_eq!(resp.reformulation_hops.len(), 3, "un hop par LLM répondant");
        for hop in &resp.reformulation_hops {
            assert_eq!(hop.engine, ReformulatorEngine::Slm, "hop {} : URL câblée ⟹ SLM", hop.hop);
            assert_eq!(hop.target_schema, "canonical/claim_v1");
            let audit = hop.slm_audit.as_ref().expect("hop SLM ⟹ faits d'audit");
            assert_eq!(audit.status, "SUCCESS");
            assert!(!audit.input_hash.is_empty());
        }
    }
}
