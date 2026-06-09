// SPDX-License-Identifier: UNLICENSED
//! `DashboardEvent` — la taxonomie de fil du dashboard live (le contrat SSE).
//!
//! Enum tagué par `kind` (snake_case). Chaque variante alimente un panneau :
//! trafic inter-agent · consensus · provenance · budget. Tout `Serialize`.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

/// Longueur max d'un aperçu de valeur (un dashboard ne déverse pas de données massives/PII).
const PREVIEW_MAX: usize = 160;

/// Tronque une valeur JSON en aperçu sûr (chaîne brute si string, sinon JSON compact).
pub fn preview(v: &Value) -> String {
    let s = match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    if s.chars().count() > PREVIEW_MAX {
        let mut t: String = s.chars().take(PREVIEW_MAX).collect();
        t.push('…');
        t
    } else {
        s
    }
}

/// Couloir visuel d'une écriture (classé par pointeur) — pour le panneau trafic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Lane {
    Candidate,
    Ballot,
    Output,
    Other,
}

impl Lane {
    /// Classe un pointeur Blackboard en couloir (`/cand/*`, `/ballot/*`, `/out/*`, …).
    pub fn classify(pointer: &str) -> Lane {
        if pointer.starts_with("/cand/") {
            Lane::Candidate
        } else if pointer.starts_with("/ballot/") {
            Lane::Ballot
        } else if pointer.starts_with("/out/") {
            Lane::Output
        } else {
            Lane::Other
        }
    }
}

/// Un micro-événement du run, diffusé en SSE vers l'UI.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DashboardEvent {
    // ── cycle de vie ─────────────────────────────────────────────────────────
    RunStarted { topology: String, mode: String, budget_cap_cents: u32, assumed_agents: usize },
    RunFinished { elapsed_ms: u64, agents_used: usize, output_preview: String, verified: bool, root: Option<String> },
    /// Le job a échoué (budget pré-vol, agent inconnu…) — terminal, message sûr.
    RunFailed { message: String },

    // ── PANNEAU 1 : trafic inter-agent (seams WriteSink + EventSink + Meter) ──
    AgentWrite { pointer: String, version: u64, agent_id: String, bytes: usize, lane: Lane },
    /// Message inter-agent (bus). `latency_ms` = latence de propagation publish→observation.
    AgentMessage { topic: String, from: String, bytes: usize, latency_ms: Option<f64> },
    AgentCall { agent_id: String, role: String, agent_kind: String, cost_cents: u32 },

    // ── PANNEAU 2 : consensus (dérivé des appels Meter générateur/juge) ───────
    Candidate { variant: usize, value_preview: String },
    Ballot { judge: String, scores: Vec<f64> },
    /// Verdict de l'agrégateur (pattern Vote/Borda/Condorcet + confiance + #juges).
    Decided { pattern: String, confidence: f64, judges_used: usize, winner_preview: String },

    // ── PANNEAU 3 : provenance (seam ProvenanceMetrics + seal) ────────────────
    Sealed { records: usize, traced_pct: f64, root: Option<String>, verified: bool },

    // ── PANNEAU 4 : budget (seam BudgetGuard/Meter) ───────────────────────────
    BudgetTick { spent_cents: u32, max_cents: u32, per_agent: BTreeMap<String, u32>, exhausted: bool },

    // ── jauges agrégées (tick périodique des snapshots de métriques) ──────────
    MetricsTick { bb_writes: u64, events_published: u64, traced_ratio: f64, chain_breaks: u64 },
}

impl DashboardEvent {
    /// Nom d'événement SSE (`event: <kind>`).
    pub fn kind(&self) -> &'static str {
        match self {
            DashboardEvent::RunStarted { .. } => "run_started",
            DashboardEvent::RunFinished { .. } => "run_finished",
            DashboardEvent::RunFailed { .. } => "run_failed",
            DashboardEvent::AgentWrite { .. } => "agent_write",
            DashboardEvent::AgentMessage { .. } => "agent_message",
            DashboardEvent::AgentCall { .. } => "agent_call",
            DashboardEvent::Candidate { .. } => "candidate",
            DashboardEvent::Ballot { .. } => "ballot",
            DashboardEvent::Decided { .. } => "decided",
            DashboardEvent::Sealed { .. } => "sealed",
            DashboardEvent::BudgetTick { .. } => "budget_tick",
            DashboardEvent::MetricsTick { .. } => "metrics_tick",
        }
    }

    /// L'événement clôt-il le run (le SSE peut se terminer après) ?
    pub fn is_terminal(&self) -> bool {
        matches!(self, DashboardEvent::RunFinished { .. } | DashboardEvent::RunFailed { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lane_classifies_pointers() {
        assert_eq!(Lane::classify("/cand/3"), Lane::Candidate);
        assert_eq!(Lane::classify("/ballot/1"), Lane::Ballot);
        assert_eq!(Lane::classify("/out/00000000000000000001"), Lane::Output);
        assert_eq!(Lane::classify("/misc/x"), Lane::Other);
    }

    #[test]
    fn preview_truncates_and_unwraps_strings() {
        assert_eq!(preview(&json!("legal")), "legal");
        let long = "x".repeat(500);
        let p = preview(&json!(long));
        assert!(p.chars().count() <= PREVIEW_MAX + 1);
        assert!(p.ends_with('…'));
    }

    #[test]
    fn event_serializes_with_kind_tag() {
        let e = DashboardEvent::AgentWrite {
            pointer: "/cand/0".into(), version: 1, agent_id: "generator".into(), bytes: 12, lane: Lane::Candidate,
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["kind"], "agent_write");
        assert_eq!(v["lane"], "candidate");
        assert_eq!(e.kind(), "agent_write");
    }
}
