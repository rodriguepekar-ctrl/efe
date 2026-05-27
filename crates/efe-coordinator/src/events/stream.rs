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
            | Self::SwarmFailed { swarm_id, .. } => Some(*swarm_id),
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
        ];
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
