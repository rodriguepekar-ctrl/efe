// SPDX-License-Identifier: UNLICENSED
//! Extensions Phase 3 du protocole SSE — émis comme `StreamEvent::Trace`
//! enrichis (rétro-compatible v1.0.0).
//!
//! ## Pourquoi pas de nouvelle variante de l'enum ?
//!
//! Modifier l'enum `StreamEvent` dans `efe-coordinator` constituerait
//! un changement breaking au binaire (Rust enum layout) même si SemVer
//! MINOR.  Phase 3 préfère **publier les évènements Phase 3 via le
//! variant existant `StreamEvent::Heartbeat` enrichi d'un payload
//! JSON** … non, en fait préférable d'utiliser un canal séparé.
//!
//! Solution effective : helpers `Phase3EventExt::publish_*` qui mappent
//! les évènements Phase 3 vers des `StreamEvent` Phase 2 existants
//! (`RobotStarted`/`RobotFinished`/`PheromoneDeposited`/`VoteResolved`)
//! en abusant des champs string pour porter le contexte Phase 3
//! (par exemple `robot_id="phase3:debate_round_2"`).
//!
//! Cette approche garantit **zéro breaking change** sur les clients SSE
//! v1.0.0 tout en délivrant l'information sémantique Phase 3.

use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use efe_coordinator::{EventBus, StreamEvent};

/// Trait d'extension pour publier les évènements Phase 3.
#[async_trait::async_trait]
pub trait Phase3EventExt: Send + Sync {
    /// Publie le démarrage d'un round de Debate.
    async fn publish_debate_round_started(
        &self,
        bus: &Arc<dyn EventBus>,
        swarm_id: Uuid,
        round: u8,
        participants: &[String],
    );

    /// Publie un argument posé dans un Debate.
    async fn publish_debate_argument(
        &self,
        bus: &Arc<dyn EventBus>,
        swarm_id: Uuid,
        round: u8,
        author: &str,
        stance: &str,
    );

    /// Publie le démarrage d'une sub-task d'un Specialist.
    async fn publish_sub_task_started(
        &self,
        bus: &Arc<dyn EventBus>,
        swarm_id: Uuid,
        sub_task_id: u32,
        assigned_llm: &str,
    );

    /// Publie la fin du drafter d'un Cascade.
    async fn publish_drafter_finished(
        &self,
        bus: &Arc<dyn EventBus>,
        swarm_id: Uuid,
        draft_tokens: u32,
    );

    /// Publie la décision du verifier (Cascade).
    async fn publish_verifier_decision(
        &self,
        bus: &Arc<dyn EventBus>,
        swarm_id: Uuid,
        decision: &str,
        ranges_count: u32,
    );

    /// Publie une reformulation effectuée.
    async fn publish_reformulation(
        &self,
        bus: &Arc<dyn EventBus>,
        swarm_id: Uuid,
        from_llm: &str,
        target_schema: &str,
    );

    /// Publie un charge sur le budget.
    async fn publish_budget_charged(
        &self,
        bus: &Arc<dyn EventBus>,
        swarm_id: Uuid,
        llm: &str,
        cents: u32,
        running_total: u32,
        max_budget: u32,
    );

    /// Publie l'annulation d'un LLM in-flight.
    async fn publish_llm_cancelled(
        &self,
        bus: &Arc<dyn EventBus>,
        swarm_id: Uuid,
        llm: &str,
        reason: &str,
    );
}

/// Implémentation par défaut, partagée par tous les workflows Phase 3.
pub struct Phase3Publisher;

impl Phase3Publisher {
    /// Helper interne : encode les évènements Phase 3 dans
    /// `StreamEvent::PheromoneDeposited` avec author = "phase3" et
    /// kind = `<event_name>` pour que l'UI puisse filtrer dessus.
    async fn emit(
        bus: &Arc<dyn EventBus>,
        swarm_id: Uuid,
        author: &str,
        kind: &str,
        payload: serde_json::Value,
    ) {
        // entry_id frais (UUID v7) — porteur de l'horloge.
        let entry_id = Uuid::now_v7();
        let _ = bus
            .publish(StreamEvent::PheromoneDeposited {
                swarm_id,
                author: format!("phase3:{author}"),
                kind: format!("PHASE3_{kind}"),
                entry_id,
                ts: Utc::now(),
            })
            .await;
        // On stocke le payload dans un événement Trace secondaire pour
        // que le client UI puisse récupérer le contenu si nécessaire.
        // (Réutilise PheromoneDeposited car le payload n'est pas exposé
        // dans cet event — on stocke via un Vote en se servant de
        // per_llm_scores comme container clé/valeur.)
        if let Ok(map) = serde_json::from_value::<std::collections::HashMap<String, f32>>(
            payload.clone(),
        ) {
            let _ = bus
                .publish(StreamEvent::VoteResolved {
                    swarm_id,
                    winner: format!("phase3:{kind}"),
                    winner_score: 0.0,
                    per_llm_scores: map,
                    ts: Utc::now(),
                })
                .await;
        } else {
            // Payload non-numérique : sérialise en JSON string dans winner.
            let s = payload.to_string();
            let _ = bus
                .publish(StreamEvent::VoteResolved {
                    swarm_id,
                    winner: format!("phase3:{kind}:{}", &s[..s.len().min(120)]),
                    winner_score: 0.0,
                    per_llm_scores: std::collections::HashMap::new(),
                    ts: Utc::now(),
                })
                .await;
        }
    }
}

#[async_trait::async_trait]
impl Phase3EventExt for Phase3Publisher {
    async fn publish_debate_round_started(
        &self,
        bus: &Arc<dyn EventBus>,
        swarm_id: Uuid,
        round: u8,
        participants: &[String],
    ) {
        Self::emit(
            bus,
            swarm_id,
            "debate",
            "ROUND_STARTED",
            serde_json::json!({"round": round, "participants": participants}),
        )
        .await;
    }

    async fn publish_debate_argument(
        &self,
        bus: &Arc<dyn EventBus>,
        swarm_id: Uuid,
        round: u8,
        author: &str,
        stance: &str,
    ) {
        Self::emit(
            bus,
            swarm_id,
            "debate",
            "ARGUMENT_POSTED",
            serde_json::json!({"round": round, "author": author, "stance": stance}),
        )
        .await;
    }

    async fn publish_sub_task_started(
        &self,
        bus: &Arc<dyn EventBus>,
        swarm_id: Uuid,
        sub_task_id: u32,
        assigned_llm: &str,
    ) {
        Self::emit(
            bus,
            swarm_id,
            "specialist",
            "SUBTASK_STARTED",
            serde_json::json!({"sub_task_id": sub_task_id, "assigned_llm": assigned_llm}),
        )
        .await;
    }

    async fn publish_drafter_finished(
        &self,
        bus: &Arc<dyn EventBus>,
        swarm_id: Uuid,
        draft_tokens: u32,
    ) {
        Self::emit(
            bus,
            swarm_id,
            "cascade",
            "DRAFTER_FINISHED",
            serde_json::json!({"draft_tokens": draft_tokens}),
        )
        .await;
    }

    async fn publish_verifier_decision(
        &self,
        bus: &Arc<dyn EventBus>,
        swarm_id: Uuid,
        decision: &str,
        ranges_count: u32,
    ) {
        Self::emit(
            bus,
            swarm_id,
            "cascade",
            "VERIFIER_DECISION",
            serde_json::json!({"decision": decision, "ranges_count": ranges_count}),
        )
        .await;
    }

    async fn publish_reformulation(
        &self,
        bus: &Arc<dyn EventBus>,
        swarm_id: Uuid,
        from_llm: &str,
        target_schema: &str,
    ) {
        Self::emit(
            bus,
            swarm_id,
            "reformulator",
            "DONE",
            serde_json::json!({"from_llm": from_llm, "target_schema": target_schema}),
        )
        .await;
    }

    async fn publish_budget_charged(
        &self,
        bus: &Arc<dyn EventBus>,
        swarm_id: Uuid,
        llm: &str,
        cents: u32,
        running_total: u32,
        max_budget: u32,
    ) {
        Self::emit(
            bus,
            swarm_id,
            "budget",
            "CHARGED",
            serde_json::json!({
                llm.to_string(): cents as f32,
                "running_total": running_total as f32,
                "max_budget": max_budget as f32,
            }),
        )
        .await;
    }

    async fn publish_llm_cancelled(
        &self,
        bus: &Arc<dyn EventBus>,
        swarm_id: Uuid,
        llm: &str,
        reason: &str,
    ) {
        Self::emit(
            bus,
            swarm_id,
            "cancel",
            "LLM_CANCELLED",
            serde_json::json!({"llm": llm, "reason": reason}),
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_coordinator::InMemoryBus;

    #[tokio::test]
    async fn publisher_emits_via_existing_events() {
        let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::new());
        let (_t, mut rx) = bus.subscribe(Some("test"));
        let publisher = Phase3Publisher;
        let swarm = Uuid::now_v7();
        publisher
            .publish_debate_round_started(&bus, swarm, 1, &["claude".into(), "gpt".into()])
            .await;

        // On reçoit au moins 1 évènement (le PheromoneDeposited de la phase3).
        let got = rx.recv().await.unwrap();
        assert!(matches!(got, StreamEvent::PheromoneDeposited { .. }));
    }

    #[tokio::test]
    async fn cancel_publishes_two_events() {
        let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::new());
        let (_t, mut rx) = bus.subscribe(Some("test"));
        let publisher = Phase3Publisher;
        let swarm = Uuid::now_v7();
        publisher
            .publish_llm_cancelled(&bus, swarm, "claude", "EARLY_QUORUM")
            .await;
        // Le helper emit publie 2 events (PheromoneDeposited + VoteResolved).
        let _ = rx.recv().await.unwrap();
        let _ = rx.recv().await.unwrap();
    }
}
