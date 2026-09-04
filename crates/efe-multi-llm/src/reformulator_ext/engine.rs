// SPDX-License-Identifier: UNLICENSED
//! **Fil B.0** — quel moteur a effectivement produit une reformulation.
//!
//! Le reformulateur (traducteur dialecte-LLM → schéma canonique) peut être
//! rendu par deux moteurs : le déterministe [`super::CanonicalReformulator`]
//! (parsing pur Rust, toujours disponible) ou le vrai petit modèle
//! [`super::SlmReformulator`] (Qwen3-0.6B via llama-server, câblé en Fil B.2).
//!
//! Le point d'honnêteté central de Fil B : on ne peut revendiquer « médié par
//! un petit modèle » **que** pour les hops estampillés [`ReformulatorEngine::Slm`].
//! Une dégradation vers le déterministe (SLM absent / en panne / schéma non
//! supporté) est **toujours** estampillée [`ReformulatorEngine::SlmDegradedToRule`]
//! — jamais présentée en silence comme du SLM.

use efe_nano_app_coordinator::{AuditEntry, AuditStatus};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Le moteur qui a produit une reformulation donnée (porté par hop dans le reçu).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReformulatorEngine {
    /// Reformulateur déterministe (parsing Rust). C'est le défaut et le repli.
    Rule,
    /// Vrai petit modèle (SLM) — la reformulation a réellement transité par le
    /// modèle et sa sortie a été validée.
    Slm,
    /// Le SLM était censé traiter ce hop mais a été indisponible / a échoué /
    /// ne supporte pas le schéma : on a **dégradé** vers le déterministe. La
    /// sortie est fiable, mais ce n'est **pas** du SLM — et on le dit.
    SlmDegradedToRule,
}

impl ReformulatorEngine {
    /// Tag stable pour logs / SSE / reçu de provenance.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Self::Rule => "RULE",
            Self::Slm => "SLM",
            Self::SlmDegradedToRule => "SLM_DEGRADED_TO_RULE",
        }
    }

    /// Vrai **uniquement** si un petit modèle a réellement produit la sortie.
    /// À utiliser pour toute revendication « médié par le SLM ».
    #[must_use]
    pub fn is_slm(self) -> bool {
        matches!(self, Self::Slm)
    }
}

/// **Fil B.3** — les faits d'audit d'une invocation SLM, portés **par hop**
/// dans le reçu re-vérifiable.
///
/// Extraits de l'`AuditEntry` que le client (`LlamaServerClient`) écrit
/// pendant le hop : le reçu committe ainsi ce que le petit modèle a réellement
/// vu (`input_hash`) et produit (`output_hash`) — la revendication
/// « SLM-médié » devient **re-vérifiable**, pas déclarative.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlmHopAudit {
    /// Id de l'`AuditEntry` source — croisable avec un writer durable si le
    /// caller en a fourni un (seam `from_client_with_audit`).
    pub entry_id: Uuid,
    /// Robot invoqué (`"reformulator"`).
    pub robot_id: String,
    /// Version de schéma du robot (`"1.1.0"` en profil multi-llm).
    pub schema_version: String,
    /// Issue de l'invocation, tag stable : `SUCCESS` / `FAILED` / `TIMEOUT` /
    /// `CIRCUIT_OPEN`. Nuance honnête : un hop **dégradé** peut porter
    /// `SUCCESS` — le SLM a répondu mais sa sortie a été rejetée (écho de
    /// schéma, validation typée). Le statut dit ce que le ROBOT a fait ;
    /// l'`engine` du hop dit ce que la mission a **servi**.
    pub status: String,
    /// BLAKE3 (hex) de l'entrée canonique envoyée au SLM.
    pub input_hash: String,
    /// BLAKE3 (hex) de la sortie typée du SLM (vide si échec avant sortie).
    pub output_hash: String,
    /// Tokens évalués côté prompt (rapportés par llama-server).
    pub tokens_prompt: u32,
    /// Tokens générés.
    pub tokens_completion: u32,
    /// Latence bout-en-bout de l'invocation.
    pub latency_ms: u64,
}

impl From<&AuditEntry> for SlmHopAudit {
    fn from(e: &AuditEntry) -> Self {
        Self {
            entry_id: e.entry_id,
            robot_id: e.robot_id.clone(),
            schema_version: e.schema_version.clone(),
            status: match e.status {
                AuditStatus::Success => "SUCCESS",
                AuditStatus::Failed => "FAILED",
                AuditStatus::Timeout => "TIMEOUT",
                AuditStatus::CircuitOpen => "CIRCUIT_OPEN",
            }
            .to_string(),
            input_hash: e.input_hash.clone(),
            output_hash: e.output_hash.clone(),
            tokens_prompt: e.tokens_prompt,
            tokens_completion: e.tokens_completion,
            latency_ms: e.latency_ms,
        }
    }
}

/// **Fil B.3** — un hop de reformulation estampillé, tel que porté par la
/// réponse de mission (`reformulation_hops`) puis **scellé** dans le reçu
/// re-vérifiable par la gateway (nœud Mode-A par hop + commitment du blob de
/// sortie par le nœud `reduce`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReformulationHop {
    /// Étiquette stable du hop dans sa mission (`"plan"`, `"claim:gpt_x"`,
    /// `"argument:r1:PRO"`).
    pub hop: String,
    /// LLM frontière dont la sortie a été traduite.
    pub from_llm: String,
    /// Forme canonique cible (tag `"canonical/plan_v1"`, …).
    pub target_schema: String,
    /// Le moteur effectif — LE garde-fou d'honnêteté de Fil B (§6).
    pub engine: ReformulatorEngine,
    /// Faits d'audit SLM — présent ssi une invocation SLM a réellement eu
    /// lieu pendant ce hop (y compris une tentative échouée puis dégradée).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slm_audit: Option<SlmHopAudit>,
    /// **Fil B.4** — POURQUOI ce hop a dégradé (tag stable :
    /// `deadline_exhausted` / `circuit_open` / `unsupported_schema` /
    /// `robot_error` / `invalid_output` / `client_side`). Absent pour un hop
    /// `SLM` réussi et pour le moteur règle pur. Rend chaque dégradation
    /// distinguable dans le reçu scellé — une deadline de mission épuisée ne
    /// ressemble plus à un circuit ouvert. (Nuance : pendant qu'une sonde
    /// HalfOpen est en vol, la porte refuse les autres hops avec le tag
    /// `circuit_open` — c'est la porte qui a refusé, même si l'état public
    /// est HalfOpen.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degrade_reason: Option<String>,
}

/// Résultat d'une reformulation : la valeur canonique **plus** le moteur qui
/// l'a produite. Le moteur voyage avec la valeur pour que chaque hop puisse
/// être estampillé honnêtement dans le reçu (jamais un `engine_tag()` global
/// et racé).
#[derive(Debug, Clone)]
pub struct Reformulated {
    /// La forme canonique typée, sérialisée (`serde_json::Value`), identique en
    /// type à la sortie de [`super::CanonicalReformulator`] **et** du SLM.
    pub value: serde_json::Value,
    /// Le moteur effectif (voir [`ReformulatorEngine`]).
    pub engine: ReformulatorEngine,
    /// **Fil B.3** — faits d'audit de l'invocation SLM du hop. `None` pour le
    /// moteur règle et quand aucun appel SLM n'est parti (circuit ouvert,
    /// schéma non supporté, entrée rejetée côté client, deadline épuisée).
    pub slm_audit: Option<SlmHopAudit>,
    /// **Fil B.4** — raison de la dégradation (tag stable), `None` sinon.
    pub degrade_reason: Option<&'static str>,
}

impl Reformulated {
    /// Constructeur pratique (sans audit SLM — le cas règle).
    #[must_use]
    pub fn new(value: serde_json::Value, engine: ReformulatorEngine) -> Self {
        Self { value, engine, slm_audit: None, degrade_reason: None }
    }

    /// **Fil B.3** — attache les faits d'audit SLM capturés pendant le hop.
    #[must_use]
    pub fn with_slm_audit(mut self, audit: Option<SlmHopAudit>) -> Self {
        self.slm_audit = audit;
        self
    }

    /// **Fil B.4** — attache la raison de dégradation (tag stable).
    #[must_use]
    pub fn with_degrade_reason(mut self, reason: &'static str) -> Self {
        self.degrade_reason = Some(reason);
        self
    }

    /// **Fil B.3** — fabrique le hop de reçu correspondant (l'étiquette et la
    /// source sont fournies par la mission, qui seule les connaît).
    #[must_use]
    pub fn to_hop(&self, hop: &str, from_llm: &str, target_schema: &str) -> ReformulationHop {
        ReformulationHop {
            hop: hop.to_string(),
            from_llm: from_llm.to_string(),
            target_schema: target_schema.to_string(),
            engine: self.engine,
            slm_audit: self.slm_audit.clone(),
            degrade_reason: self.degrade_reason.map(str::to_string),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Importé ICI et non en tête de fichier : le seul appel à `AuditEntry::success`
    // est un montage de test, et une importation de production inutilisée est un
    // avertissement — donc une erreur sous la porte de qualité.
    use efe_nano_app_coordinator::SuccessRecord;

    #[test]
    fn tags_are_stable() {
        assert_eq!(ReformulatorEngine::Rule.tag(), "RULE");
        assert_eq!(ReformulatorEngine::Slm.tag(), "SLM");
        assert_eq!(ReformulatorEngine::SlmDegradedToRule.tag(), "SLM_DEGRADED_TO_RULE");
    }

    #[test]
    fn only_slm_is_slm() {
        assert!(ReformulatorEngine::Slm.is_slm());
        assert!(!ReformulatorEngine::Rule.is_slm());
        assert!(!ReformulatorEngine::SlmDegradedToRule.is_slm());
    }

    #[test]
    fn engine_serializes_screaming_snake() {
        assert_eq!(
            serde_json::to_string(&ReformulatorEngine::SlmDegradedToRule).unwrap(),
            "\"SLM_DEGRADED_TO_RULE\""
        );
    }

    // ─── Fil B.3 : les types de hop portés par le reçu ───────────────

    fn entry_success() -> AuditEntry {
        let ctx = efe_nano_app_coordinator::CallContext::mock();
        AuditEntry::success(
            &ctx,
            SuccessRecord {
                robot_id: "reformulator",
                schema_version: "1.1.0",
                input_hash: "aa11".into(),
                output_hash: "bb22".into(),
                tokens_prompt: 120,
                tokens_completion: 40,
                latency_ms: 350,
            },
        )
    }

    #[test]
    fn slm_hop_audit_lifts_the_committed_facts() {
        let e = entry_success();
        let a = SlmHopAudit::from(&e);
        assert_eq!(a.entry_id, e.entry_id, "croisable avec un writer durable");
        assert_eq!(a.robot_id, "reformulator");
        assert_eq!(a.schema_version, "1.1.0");
        assert_eq!(a.status, "SUCCESS");
        assert_eq!(a.input_hash, "aa11");
        assert_eq!(a.output_hash, "bb22");
        assert_eq!((a.tokens_prompt, a.tokens_completion, a.latency_ms), (120, 40, 350));
    }

    #[test]
    fn hop_wire_format_is_stable_tags() {
        // Le reçu est un contrat : engine en tag SCREAMING, slm_audit omis si None.
        let hop = Reformulated::new(serde_json::json!({"x": 1}), ReformulatorEngine::Rule)
            .to_hop("plan", "claude_x", "canonical/plan_v1");
        let v = serde_json::to_value(&hop).unwrap();
        assert_eq!(v["engine"], "RULE");
        assert_eq!(v["hop"], "plan");
        assert_eq!(v["from_llm"], "claude_x");
        assert_eq!(v["target_schema"], "canonical/plan_v1");
        assert!(v.get("slm_audit").is_none(), "None ⟹ champ absent du fil");
        assert!(v.get("degrade_reason").is_none(), "None ⟹ champ absent du fil (B.4)");
        // Round-trip.
        let back: ReformulationHop = serde_json::from_value(v).unwrap();
        assert_eq!(back.engine, ReformulatorEngine::Rule);
        assert!(back.slm_audit.is_none());
        assert!(back.degrade_reason.is_none());
    }

    /// **Fil B.4** — la raison de dégradation voyage sur le fil (tag stable).
    #[test]
    fn degrade_reason_rides_the_wire() {
        let hop = Reformulated::new(serde_json::json!({}), ReformulatorEngine::SlmDegradedToRule)
            .with_degrade_reason("deadline_exhausted")
            .to_hop("plan", "claude_x", "canonical/plan_v1");
        let v = serde_json::to_value(&hop).unwrap();
        assert_eq!(v["degrade_reason"], "deadline_exhausted");
        let back: ReformulationHop = serde_json::from_value(v).unwrap();
        assert_eq!(back.degrade_reason.as_deref(), Some("deadline_exhausted"));
    }

    #[test]
    fn hop_with_slm_audit_round_trips() {
        let audit = SlmHopAudit::from(&entry_success());
        let hop = Reformulated::new(serde_json::json!({}), ReformulatorEngine::Slm)
            .with_slm_audit(Some(audit.clone()))
            .to_hop("claim:gpt_x", "gpt_x", "canonical/claim_v1");
        let v = serde_json::to_value(&hop).unwrap();
        assert_eq!(v["engine"], "SLM");
        assert_eq!(v["slm_audit"]["robot_id"], "reformulator");
        assert_eq!(v["slm_audit"]["input_hash"], "aa11");
        let back: ReformulationHop = serde_json::from_value(v).unwrap();
        assert_eq!(back.slm_audit, Some(audit));
    }
}
