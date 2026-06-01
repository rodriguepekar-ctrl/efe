//! Concept stack — turn-by-turn reasoning trace for Mode-3.
//!
//! Persisted as JSONB in `llm_call_audit.concept_stack_jsonb` on the
//! final audit row of a Mode-3 call. Right-to-explanation API
//! (`efe_get_concept_stack`) retrieves this for GDPR Art. 22.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::LLM_REASONING_MAX_CHARS;

/// One entry in the concept stack — one turn of reasoning.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConceptStackEntry {
    pub turn: u32,
    pub candidates_in: Vec<String>,
    pub candidates_out: Vec<String>,
    /// Truncated to `LLM_REASONING_MAX_CHARS` chars.
    /// `None` for swarm-only turns (no LLM call).
    pub llm_reasoning: Option<String>,
    pub swarm_label_attempt: Option<String>,
    pub swarm_confidence: Option<f32>,
    pub cost_usd: f64,
    pub duration_ms: u32,
    pub provider_used: Option<String>,
    pub timestamp: DateTime<Utc>,
}

impl ConceptStackEntry {
    pub fn truncate_reasoning(reasoning: Option<String>) -> Option<String> {
        reasoning.map(|s| {
            if s.chars().count() > LLM_REASONING_MAX_CHARS {
                let truncated: String = s.chars().take(LLM_REASONING_MAX_CHARS).collect();
                format!("{}...[truncated]", truncated)
            } else {
                s
            }
        })
    }
}

/// Full concept stack with finalisation + cost aggregation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConceptStack {
    pub entries: Vec<ConceptStackEntry>,
    pub final_label: Option<String>,
    pub final_confidence: Option<f32>,
    pub converged: bool,
    pub total_cost_usd: f64,
}

impl ConceptStack {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            final_label: None,
            final_confidence: None,
            converged: false,
            total_cost_usd: 0.0,
        }
    }

    pub fn append(&mut self, entry: ConceptStackEntry) {
        self.total_cost_usd += entry.cost_usd;
        self.entries.push(entry);
    }

    pub fn finalize(&mut self, label: String, confidence: f32, converged: bool) {
        self.final_label = Some(label);
        self.final_confidence = Some(confidence);
        self.converged = converged;
    }

    pub fn total_turns(&self) -> u32 {
        self.entries.len() as u32
    }

    pub fn total_input_tokens(&self) -> u32 {
        // Tokens are tracked per-LLM-call in audit chain ; concept_stack
        // stores cost_usd which encodes them. Caller can derive from
        // M21 llm_call_audit rows. For convenience, we expose 0 here
        // and let the audit query aggregate.
        0
    }

    pub fn total_output_tokens(&self) -> u32 {
        0
    }

    pub fn total_duration_ms(&self) -> u32 {
        self.entries.iter().map(|e| e.duration_ms).sum()
    }

    /// Number of LLM calls actually made (some turns are swarm-only).
    pub fn llm_calls_count(&self) -> u32 {
        self.entries
            .iter()
            .filter(|e| e.llm_reasoning.is_some())
            .count() as u32
    }
}

impl Default for ConceptStack {
    fn default() -> Self {
        Self::new()
    }
}
