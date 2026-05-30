// SPDX-License-Identifier: UNLICENSED
//! `StreamEvent` — protocole SSE exposé à l'UI.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use efe_nano_app_coordinator::types::router::TaskClass;

use crate::metrics::ChewingMetrics;

/// Version SemVer du protocole SSE (`StreamEvent`).
///
/// Bump MAJOR si un variant est supprimé ou son `tag` change.
/// Bump MINOR si on ajoute un nouveau variant ou un nouveau champ optionnel.
pub const EVENT_SCHEMA_VERSION: &str = "1.0.0";

/// **P4.S2** — Liste canonique des tags des **19 variants** de [`StreamEvent`].
///
/// Source de vérité exposée par le bridge SSE (`GET /v1/schema`) pour le
/// garde-fou de non-dérive côté UI (le front compare cette liste + la version à
/// ses propres types au boot).
///
/// **Maintenance** : tout nouveau variant doit (1) être géré par
/// [`StreamEvent::tag`] (le compilateur force l'exhaustivité du `match`) ET
/// (2) être ajouté ici (le test `all_event_tags_count_and_unique` rappelle de
/// bumper la longueur).
pub const ALL_EVENT_TAGS: &[&str] = &[
    "SWARM_STARTED",
    "ROBOT_STARTED",
    "ROBOT_FINISHED",
    "PHEROMONE_DEPOSITED",
    "LLM_TOKEN_STREAMED",
    "LLM_FINISHED",
    "VERDICT_READY",
    "VOTE_RESOLVED",
    "DRAFT_TOKEN",
    "SPECULATION_RESOLVED",
    "PRE_CHEWING_METRICS",
    "SWARM_FINISHED",
    "SWARM_FAILED",
    "HEARTBEAT",
    "LLM_FALLBACK_TRIGGERED",
    "LLM_FAILED",
    "SLM_CIRCUIT_OPENED",
    "DRAFTER_BYPASSED",
    "DRAFT_REJECTED_AS_NOISE",
];

/// Mission qui a produit l'évènement (pour multiplexing UI).
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MissionTag {
    MissionA,
    MissionB,
    Speculative,
}

/// Évènement typé exposé au client SSE.
///
/// Toutes les variantes embarquent `swarm_id` (corrélation cross-évènements)
/// et `ts` (estampille UTC).
///
/// ## Format SSE
///
/// Chaque évènement est sérialisé en `data: <json>\n\n` sur la wire.
/// Le `tag` dans `#[serde(tag = "type")]` permet à l'UI de switcher
/// sans roundtrip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StreamEvent {
    /// Le swarm démarre.  Précédé d'aucun autre évènement pour ce `swarm_id`.
    SwarmStarted {
        /// UUID v7 du swarm (= request_id pour le tracing).
        swarm_id: Uuid,
        /// Mission concernée.
        mission: MissionTag,
        /// Texte input verbatim (peut être tronqué côté client si trop long).
        user_text: String,
        /// Catégorie devinée par l'UI/gateway (peut différer du final).
        task_class: TaskClass,
        /// Estampille de démarrage.
        ts: DateTime<Utc>,
    },

    /// Un nano-robot a commencé son inférence.
    RobotStarted {
        /// Swarm.
        swarm_id: Uuid,
        /// Robot (`analyzer`, ...).
        robot_id: String,
        /// Estampille.
        ts: DateTime<Utc>,
    },

    /// Un nano-robot a fini, succès ou échec.
    RobotFinished {
        /// Swarm.
        swarm_id: Uuid,
        /// Robot.
        robot_id: String,
        /// Succès vs échec.
        success: bool,
        /// Latence ms.
        latency_ms: u64,
        /// Détail erreur si échec (None sinon).
        error: Option<String>,
        /// Estampille.
        ts: DateTime<Utc>,
    },

    /// Une phéromone a été déposée dans la blackboard.
    PheromoneDeposited {
        /// Swarm.
        swarm_id: Uuid,
        /// Auteur (`"analyzer"`, `"claude_opus_4"`, …).
        author: String,
        /// Catégorie (`"ENTITY"`, `"VERDICT"`, …).
        kind: String,
        /// ID de l'entry (pour les filtres UI).
        entry_id: Uuid,
        /// Estampille.
        ts: DateTime<Utc>,
    },

    /// Un LLM gros a streamé un token (concat côté UI).
    LlmTokenStreamed {
        /// Swarm.
        swarm_id: Uuid,
        /// Provider (`"claude_opus_4"`, `"gpt4_turbo"`, …).
        provider: String,
        /// Token (string courte, parfois plusieurs caractères).
        token: String,
        /// Index séquentiel dans le stream pour ce provider.
        index: u64,
        /// Estampille.
        ts: DateTime<Utc>,
    },

    /// Un LLM gros a fini son output complet.
    LlmFinished {
        /// Swarm.
        swarm_id: Uuid,
        /// Provider.
        provider: String,
        /// Tokens générés au total.
        tokens_generated: u32,
        /// Coût observé en cents.
        cost_cents: u32,
        /// Latence end-to-end.
        latency_ms: u64,
        /// Estampille.
        ts: DateTime<Utc>,
    },

    /// Un verdict de fact-check vient d'être produit (overlay UI).
    VerdictReady {
        /// Swarm.
        swarm_id: Uuid,
        /// Claim verbatim.
        claim: String,
        /// `SUPPORTED` | `UNSUPPORTED` | `CONTRADICTED` | `INSUFFICIENT_EVIDENCE`.
        verdict: String,
        /// `INFO` | `WARN` | `ERROR`.
        severity: String,
        /// ID source la mieux matchée.
        best_source_id: Option<String>,
        /// Similarité 0..1.
        similarity: f32,
        /// Estampille.
        ts: DateTime<Utc>,
    },

    /// Résultat d'un vote (Mission B).
    VoteResolved {
        /// Swarm.
        swarm_id: Uuid,
        /// Provider gagnant.
        winner: String,
        /// Score du gagnant.
        winner_score: f32,
        /// Scores per-LLM.
        per_llm_scores: HashMap<String, f32>,
        /// Estampille.
        ts: DateTime<Utc>,
    },

    /// Le drafter speculative a produit un token candidat.
    DraftToken {
        /// Swarm.
        swarm_id: Uuid,
        /// Token spéculatif.
        token: String,
        /// Position dans le draft.
        index: u32,
        /// Estampille.
        ts: DateTime<Utc>,
    },

    /// Le verifier speculative a tranché (accepté / regen partiel / regen total).
    SpeculationResolved {
        /// Swarm.
        swarm_id: Uuid,
        /// Tokens acceptés tels quels.
        tokens_accepted: u32,
        /// Tokens regénérés.
        tokens_regenerated: u32,
        /// Latence gagnée (ms négatif si on a perdu).
        latency_saved_ms: i64,
        /// **Sprint S7 R2** — Le verifier API a-t-il été appelé ?
        ///
        /// - `true` (HOT PATH) : drafter seul, verifier jamais touché → vrai
        ///   savings économique (coût + latence du verifier non payés).
        /// - `false` (SLOW PATH) : verifier appelé, savings 0 sur ce point.
        ///
        /// Sémantique distincte de `draft_accepted` : le draft peut être
        /// "accepté" après comparaison Jaccard avec verify (slow path), mais
        /// `verifier_skipped` = `false` car on a quand même payé le verifier.
        ///
        /// `#[serde(default)]` pour backward-compat avec les events S5-S6
        /// sérialisés sans ce champ.
        #[serde(default)]
        verifier_skipped: bool,
        /// Estampille.
        ts: DateTime<Utc>,
    },

    /// Métriques de pré-mâchage agrégées en fin de mission (Sprint S4 R2).
    ///
    /// Émis juste avant `SwarmFinished` (donc après `LlmFinished` + éventuel
    /// `VerdictReady`). Contient les tokens d'entrée bruts vs compressés, les
    /// latences par phase SLM, le LLM choisi par le router, et les ratios de
    /// savings.
    ///
    /// Permet aux UIs / tests / outils d'audit de quantifier objectivement le
    /// gain économique du pipeline EFE vs un appel naïf au gros LLM.
    PreChewingMetrics {
        /// Swarm.
        swarm_id: Uuid,
        /// Métriques agrégées (cf. [`crate::metrics::ChewingMetrics`]).
        metrics: ChewingMetrics,
        /// Estampille d'émission.
        ts: DateTime<Utc>,
    },

    /// Le swarm est terminé, succès complet.
    SwarmFinished {
        /// Swarm.
        swarm_id: Uuid,
        /// Mission.
        mission: MissionTag,
        /// Latence end-to-end ms.
        latency_ms: u64,
        /// Hash de la chaîne d'audit complète (BLAKE3 hex 64 chars).
        audit_chain_hash: String,
        /// Estampille.
        ts: DateTime<Utc>,
    },

    /// Le swarm a échoué irrécupérablement.
    SwarmFailed {
        /// Swarm.
        swarm_id: Uuid,
        /// Mission.
        mission: MissionTag,
        /// Stade où l'erreur s'est produite (`"analyze"`, `"vote"`, …).
        stage: String,
        /// Message d'erreur humain.
        error: String,
        /// Estampille.
        ts: DateTime<Utc>,
    },

    /// Heartbeat — envoyé toutes les ~15 s pour garder la connexion SSE
    /// chaude (proxies HTTP coupent souvent à 60 s sans activité).
    Heartbeat {
        /// Estampille.
        ts: DateTime<Utc>,
    },

    // ─── Sprint S8 R2 — Failure modes / graceful degradation ───────────

    /// **S8** — Le LLM `selected` par le router a échoué après retry épuisé.
    /// Le bridge a basculé sur `fallback_chain[N]`.
    LlmFallbackTriggered {
        /// Swarm.
        swarm_id: Uuid,
        /// LLM initial (choix router).
        from: String,
        /// LLM cible du fallback effectivement utilisé.
        to: String,
        /// Reason de l'échec original (e.g. `"HTTP 429"`, `"circuit open"`).
        reason: String,
        /// Estampille.
        ts: DateTime<Utc>,
    },

    /// **S8** — Un LLM (Mission B) a fail pendant le fan-out parallèle.
    /// Émis par LLM raté, distinct de `RobotFinished` (qui concerne les
    /// nano-robots SLM).
    LlmFailed {
        /// Swarm.
        swarm_id: Uuid,
        /// LLM ayant échoué.
        provider: String,
        /// Reason humain (e.g. `"HTTP 429"`, `"circuit open"`).
        reason: String,
        /// Estampille.
        ts: DateTime<Utc>,
    },

    /// **S8** — Le circuit breaker d'un SLM (analyzer/compressor/verifier)
    /// vient de s'ouvrir après seuil d'échecs atteint. Les missions
    /// suivantes skipperont ce robot jusqu'au `reset_timeout`.
    SlmCircuitOpened {
        /// Swarm où le circuit s'est ouvert (= dernier échec).
        swarm_id: Uuid,
        /// Robot SLM concerné (`"analyzer"`, `"compressor"`, `"verifier"`).
        robot: String,
        /// Nombre d'échecs consécutifs comptés.
        failures: u32,
        /// Estampille.
        ts: DateTime<Utc>,
    },

    /// **S8** — Mission Speculative : le drafter a fail, on a sauté
    /// directement au verifier (savings perdu mais qualité préservée).
    DrafterBypassed {
        /// Swarm.
        swarm_id: Uuid,
        /// Drafter qui a fail.
        drafter: String,
        /// Verifier appelé en fallback.
        fallback_to_verifier: String,
        /// Reason humain du fail drafter.
        reason: String,
        /// Estampille.
        ts: DateTime<Utc>,
    },

    /// **R3.S3 (fix L4/D6)** — Mission Speculative : le draft du drafter a
    /// été détecté comme **bruit** (mot-de-contenu nul, refus, répétition)
    /// et REJETÉ du hot path → forcé sur le slow path (verifier). La
    /// mission produit donc la réponse correcte du verifier au lieu de
    /// livrer le bruit. Tracé pour mesurer le taux de bruit drafter (si
    /// élevé, le drafter est mal choisi → R3.S4).
    DraftRejectedAsNoise {
        /// Swarm.
        swarm_id: Uuid,
        /// Signal de bruit détecté (`"ONLY_STOPWORDS"`, `"REFUSAL_PATTERN"`…).
        signal: String,
        /// Extrait du draft rejeté (≤ 40 chars, pour debug/audit).
        draft_excerpt: String,
        /// Estampille.
        ts: DateTime<Utc>,
    },
}

impl StreamEvent {
    /// Tag (variante) sous forme de string pour les filtres UI.
    #[must_use]
    pub fn tag(&self) -> &'static str {
        match self {
            Self::SwarmStarted { .. } => "SWARM_STARTED",
            Self::RobotStarted { .. } => "ROBOT_STARTED",
            Self::RobotFinished { .. } => "ROBOT_FINISHED",
            Self::PheromoneDeposited { .. } => "PHEROMONE_DEPOSITED",
            Self::LlmTokenStreamed { .. } => "LLM_TOKEN_STREAMED",
            Self::LlmFinished { .. } => "LLM_FINISHED",
            Self::VerdictReady { .. } => "VERDICT_READY",
            Self::VoteResolved { .. } => "VOTE_RESOLVED",
            Self::DraftToken { .. } => "DRAFT_TOKEN",
            Self::SpeculationResolved { .. } => "SPECULATION_RESOLVED",
            Self::PreChewingMetrics { .. } => "PRE_CHEWING_METRICS",
            Self::SwarmFinished { .. } => "SWARM_FINISHED",
            Self::SwarmFailed { .. } => "SWARM_FAILED",
            Self::Heartbeat { .. } => "HEARTBEAT",
            // S8 R2 — Failure modes
            Self::LlmFallbackTriggered { .. } => "LLM_FALLBACK_TRIGGERED",
            Self::LlmFailed { .. } => "LLM_FAILED",
            Self::SlmCircuitOpened { .. } => "SLM_CIRCUIT_OPENED",
            Self::DrafterBypassed { .. } => "DRAFTER_BYPASSED",
            // R3.S3 — Speculator noise rejection
            Self::DraftRejectedAsNoise { .. } => "DRAFT_REJECTED_AS_NOISE",
        }
    }

    /// `swarm_id` de l'évènement (None pour `Heartbeat`).
    #[must_use]
    pub fn swarm_id(&self) -> Option<Uuid> {
        match self {
            Self::SwarmStarted { swarm_id, .. }
            | Self::RobotStarted { swarm_id, .. }
            | Self::RobotFinished { swarm_id, .. }
            | Self::PheromoneDeposited { swarm_id, .. }
            | Self::LlmTokenStreamed { swarm_id, .. }
            | Self::LlmFinished { swarm_id, .. }
            | Self::VerdictReady { swarm_id, .. }
            | Self::VoteResolved { swarm_id, .. }
            | Self::DraftToken { swarm_id, .. }
            | Self::SpeculationResolved { swarm_id, .. }
            | Self::PreChewingMetrics { swarm_id, .. }
            | Self::SwarmFinished { swarm_id, .. }
            | Self::SwarmFailed { swarm_id, .. }
            | Self::LlmFallbackTriggered { swarm_id, .. }
            | Self::LlmFailed { swarm_id, .. }
            | Self::SlmCircuitOpened { swarm_id, .. }
            | Self::DrafterBypassed { swarm_id, .. }
            | Self::DraftRejectedAsNoise { swarm_id, .. } => Some(*swarm_id),
            Self::Heartbeat { .. } => None,
        }
    }

    /// Sérialise l'évènement en frame SSE standard (`data: …\n\n`).
    #[must_use]
    pub fn to_sse_frame(&self) -> String {
        let json = serde_json::to_string(self).unwrap_or_else(|_| "{}".into());
        format!("event: {}\ndata: {json}\n\n", self.tag())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_event_tags_count_and_unique() {
        // 19 variants (cf. `tag()`). Si ce nombre change, mettre à jour
        // `ALL_EVENT_TAGS` ET `tag()` (le compilateur force ce dernier).
        assert_eq!(ALL_EVENT_TAGS.len(), 19, "ALL_EVENT_TAGS doit lister les 19 variants");
        let mut seen = std::collections::HashSet::new();
        for t in ALL_EVENT_TAGS {
            assert!(seen.insert(*t), "tag dupliqué dans ALL_EVENT_TAGS: {t}");
        }
        // Spot-checks de cohérence avec `tag()`.
        assert!(ALL_EVENT_TAGS.contains(&"SWARM_STARTED"));
        assert!(ALL_EVENT_TAGS.contains(&"PRE_CHEWING_METRICS"));
        assert!(ALL_EVENT_TAGS.contains(&"DRAFT_REJECTED_AS_NOISE"));
    }

    #[test]
    fn all_variants_roundtrip_invariant_i3() {
        // Invariant I3 — tout StreamEvent est sérialisable JSON et roundtrip-stable.
        let swarm = Uuid::now_v7();
        let evs = vec![
            StreamEvent::SwarmStarted {
                swarm_id: swarm,
                mission: MissionTag::MissionA,
                user_text: "hi".into(),
                task_class: TaskClass::FactualQa,
                ts: Utc::now(),
            },
            StreamEvent::RobotStarted {
                swarm_id: swarm,
                robot_id: "analyzer".into(),
                ts: Utc::now(),
            },
            StreamEvent::RobotFinished {
                swarm_id: swarm,
                robot_id: "analyzer".into(),
                success: true,
                latency_ms: 42,
                error: None,
                ts: Utc::now(),
            },
            StreamEvent::PheromoneDeposited {
                swarm_id: swarm,
                author: "analyzer".into(),
                kind: "ENTITY".into(),
                entry_id: Uuid::now_v7(),
                ts: Utc::now(),
            },
            StreamEvent::LlmTokenStreamed {
                swarm_id: swarm,
                provider: "claude".into(),
                token: "Hello".into(),
                index: 0,
                ts: Utc::now(),
            },
            StreamEvent::LlmFinished {
                swarm_id: swarm,
                provider: "claude".into(),
                tokens_generated: 100,
                cost_cents: 5,
                latency_ms: 1000,
                ts: Utc::now(),
            },
            StreamEvent::VerdictReady {
                swarm_id: swarm,
                claim: "x".into(),
                verdict: "SUPPORTED".into(),
                severity: "INFO".into(),
                best_source_id: Some("src-1".into()),
                similarity: 0.95,
                ts: Utc::now(),
            },
            StreamEvent::VoteResolved {
                swarm_id: swarm,
                winner: "claude".into(),
                winner_score: 0.8,
                per_llm_scores: HashMap::from([("claude".into(), 0.8), ("gpt4".into(), 0.6)]),
                ts: Utc::now(),
            },
            StreamEvent::DraftToken {
                swarm_id: swarm,
                token: "X".into(),
                index: 0,
                ts: Utc::now(),
            },
            StreamEvent::SpeculationResolved {
                swarm_id: swarm,
                tokens_accepted: 50,
                tokens_regenerated: 10,
                latency_saved_ms: 300,
                verifier_skipped: false,
                ts: Utc::now(),
            },
            StreamEvent::PreChewingMetrics {
                swarm_id: swarm,
                metrics: ChewingMetrics {
                    input_tokens_raw: 100,
                    input_tokens_compressed: 25,
                    llm_tokens_generated: 30,
                    compression_ratio: 0.25,
                    savings_tokens: 75,
                    savings_ratio: 0.75,
                    analyzer_latency_ms: 500,
                    router_latency_ms: 2,
                    compressor_latency_ms: 800,
                    llm_latency_ms: 1500,
                    verifier_latency_ms: 400,
                    total_slm_latency_ms: 1700,
                    total_pipeline_latency_ms: 3200,
                    selected_llm: "claude_claude-sonnet-4-6".into(),
                    router_reason: "highest quality".into(),
                    router_estimated_cost_cents: 2,
                    // R3.S1 : 2 cents × 1000 = 2_000 µ¢.
                    router_estimated_cost_microcents: 2_000,
                    // S7 R2 — Mission Spec fields, None pour Mission A
                    verifier_skipped: None,
                    spec_drafter_latency_ms: 0,
                    spec_verifier_latency_ms: 0,
                    spec_savings_cents: 0,
                    // R3.S1 : 0 (non-Spec mission).
                    spec_savings_microcents: 0,
                    // R3.S3 : None (non-Spec mission).
                    spec_noise_rejected: None,
                    // R3.S4 : None (non-Spec mission).
                    spec_drafter_is_local: None,
                    // S8 R2 — Graceful degradation fields
                    failure_mode: crate::failure::FailureMode::None,
                    degradations_count: 0,
                    failed_components: vec![],
                    // R3.S5 — multi-tenant fields.
                    tenant_id: None,
                    tenant_cost_cap_exceeded: None,
                    tenant_llms_filtered: None,
                    // R3.S8 — A/B test (None : mission non-A/B).
                    ab_arm: None,
                    ab_table_version: None,
                },
                ts: Utc::now(),
            },
            StreamEvent::SwarmFinished {
                swarm_id: swarm,
                mission: MissionTag::MissionA,
                latency_ms: 2000,
                audit_chain_hash: "0".repeat(64),
                ts: Utc::now(),
            },
            StreamEvent::SwarmFailed {
                swarm_id: swarm,
                mission: MissionTag::MissionA,
                stage: "vote".into(),
                error: "no quorum".into(),
                ts: Utc::now(),
            },
            StreamEvent::Heartbeat { ts: Utc::now() },
            // ── S8 R2 — Failure modes (gap pré-R3.S3 : ces 4 events
            //    n'étaient PAS couverts par l'invariant I3) ──
            StreamEvent::LlmFallbackTriggered {
                swarm_id: swarm,
                from: "claude".into(),
                to: "gemini".into(),
                reason: "HTTP 429".into(),
                ts: Utc::now(),
            },
            StreamEvent::LlmFailed {
                swarm_id: swarm,
                provider: "gpt4".into(),
                reason: "circuit open".into(),
                ts: Utc::now(),
            },
            StreamEvent::SlmCircuitOpened {
                swarm_id: swarm,
                robot: "analyzer".into(),
                failures: 3,
                ts: Utc::now(),
            },
            StreamEvent::DrafterBypassed {
                swarm_id: swarm,
                drafter: "gemini".into(),
                fallback_to_verifier: "claude".into(),
                reason: "timeout".into(),
                ts: Utc::now(),
            },
            // ── R3.S3 — Speculator noise rejection ──
            StreamEvent::DraftRejectedAsNoise {
                swarm_id: swarm,
                signal: "ONLY_STOPWORDS".into(),
                draft_excerpt: "La".into(),
                ts: Utc::now(),
            },
        ];
        // Garde-fou anti-dérive : l'invariant I3 DOIT couvrir TOUS les
        // variants de `StreamEvent`. Le match `tag()` est exhaustif
        // (compilateur-enforced), mais ce `vec!` ne l'est PAS. Si un futur
        // sprint ajoute un variant sans l'ajouter ici, ce compteur de tags
        // distincts échoue → tripwire forçant la mise à jour.
        const STREAM_EVENT_VARIANT_COUNT: usize = 19;
        let distinct_tags: std::collections::HashSet<&'static str> =
            evs.iter().map(StreamEvent::tag).collect();
        assert_eq!(
            distinct_tags.len(),
            STREAM_EVENT_VARIANT_COUNT,
            "Invariant I3 : le vec doit couvrir les {STREAM_EVENT_VARIANT_COUNT} \
             variants de StreamEvent (tags distincts vus : {}). Un variant a \
             probablement été ajouté sans l'inclure dans ce test.",
            distinct_tags.len()
        );
        for ev in evs {
            let s = serde_json::to_string(&ev).unwrap();
            let back: StreamEvent = serde_json::from_str(&s).unwrap();
            assert_eq!(back, ev, "roundtrip failed for tag {}", ev.tag());
        }
    }

    #[test]
    fn sse_frame_is_correct_format() {
        let ev = StreamEvent::Heartbeat { ts: Utc::now() };
        let frame = ev.to_sse_frame();
        assert!(frame.starts_with("event: HEARTBEAT\ndata: "));
        assert!(frame.ends_with("\n\n"));
    }

    #[test]
    fn tag_is_stable_string() {
        let ev = StreamEvent::RobotStarted {
            swarm_id: Uuid::now_v7(),
            robot_id: "a".into(),
            ts: Utc::now(),
        };
        assert_eq!(ev.tag(), "ROBOT_STARTED");
    }

    #[test]
    fn swarm_id_extraction_works() {
        let swarm = Uuid::now_v7();
        let ev = StreamEvent::RobotStarted {
            swarm_id: swarm,
            robot_id: "a".into(),
            ts: Utc::now(),
        };
        assert_eq!(ev.swarm_id(), Some(swarm));
        let hb = StreamEvent::Heartbeat { ts: Utc::now() };
        assert_eq!(hb.swarm_id(), None);
    }

    #[test]
    fn missing_fields_serialize_with_null() {
        let ev = StreamEvent::RobotFinished {
            swarm_id: Uuid::now_v7(),
            robot_id: "a".into(),
            success: false,
            latency_ms: 0,
            error: None,
            ts: Utc::now(),
        };
        let s = serde_json::to_string(&ev).unwrap();
        assert!(s.contains("\"error\":null"));
    }
}
