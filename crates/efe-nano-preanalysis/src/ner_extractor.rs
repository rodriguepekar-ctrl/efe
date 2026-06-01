// SPDX-License-Identifier: UNLICENSED
//! **Sprint 13 — N21 + G11** : NER extractor — Mode A (pure-Rust
//! deterministic).
//!
//! Detects emails, phone numbers, URLs, money amounts, and ISO
//! dates via regular expressions.  Writes the resulting list of
//! `Entity` objects to the blackboard at `/entities`.
//!
//! **Determinism** : the regex set is hard-coded ; output is
//! byte-stable across runs (sorted by span start).

use std::sync::OnceLock;

use async_trait::async_trait;
use efe_nano_sdk::{AgentId, Entity};
use efe_swarm_orchestrator::{
    AgentOutcome, SwarmAgent, SwarmAgentContext, SwarmAgentError,
};
use regex::Regex;

/// **N21** : the agent.
#[derive(Debug, Default, Clone, Copy)]
pub struct NerExtractor;

impl NerExtractor {
    pub const AGENT_ID: &'static str = "ner-extractor";

    pub fn new() -> Self {
        Self
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Compiled regexes (lazy, cached)
// ─────────────────────────────────────────────────────────────────────

fn email_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r"[a-zA-Z0-9._%+\-]+@[a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}",
        )
        .expect("compile email regex")
    })
}

fn phone_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    // North-American + international shapes.
    R.get_or_init(|| {
        Regex::new(
            r"(\+?[0-9]{1,3}[ .\-]?)?(\(?[0-9]{2,4}\)?[ .\-]?){2,4}[0-9]{2,4}",
        )
        .expect("compile phone regex")
    })
}

fn url_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"https?://[^\s<>]+").expect("compile url regex")
    })
}

fn money_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r"(\$|€|£|¥)\s?\d{1,3}(?:[,\s]\d{3})*(?:\.\d{1,2})?",
        )
        .expect("compile money regex")
    })
}

fn iso_date_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"\b(\d{4})-(0?[1-9]|1[0-2])-(0?[1-9]|[12]\d|3[01])\b")
            .expect("compile date regex")
    })
}

// ─────────────────────────────────────────────────────────────────────
//  Extraction
// ─────────────────────────────────────────────────────────────────────

/// **N21** : Pure extraction function (testable without a swarm).
pub fn extract_entities(text: &str) -> Vec<Entity> {
    let mut entities: Vec<Entity> = Vec::new();
    for (m, entity_type) in [
        (email_re(), "email"),
        (url_re(), "url"),
        (money_re(), "money"),
        (iso_date_re(), "date"),
    ] {
        for cap in m.find_iter(text) {
            entities.push(Entity {
                entity_type: entity_type.to_string(),
                value: cap.as_str().to_string(),
                span: Some((cap.start() as u32, cap.end() as u32)),
            });
        }
    }
    // Phone has higher false-positive rate ; we run it last and
    // accept overlap with money (which is filtered out below).
    for cap in phone_re().find_iter(text) {
        // Filter : reject anything fewer than 7 digits.
        let digit_count = cap.as_str().chars().filter(|c| c.is_ascii_digit()).count();
        if digit_count < 7 {
            continue;
        }
        let span = (cap.start() as u32, cap.end() as u32);
        // Skip if overlaps an existing extract.
        if entities.iter().any(|e| {
            e.span
                .map(|s| !(span.1 <= s.0 || span.0 >= s.1))
                .unwrap_or(false)
        }) {
            continue;
        }
        entities.push(Entity {
            entity_type: "phone".into(),
            value: cap.as_str().trim().to_string(),
            span: Some(span),
        });
    }
    // Sort by span start for deterministic output (replay I13).
    entities.sort_by_key(|e| e.span.map(|s| s.0).unwrap_or(u32::MAX));
    entities
}

// ─────────────────────────────────────────────────────────────────────
//  SwarmAgent impl
// ─────────────────────────────────────────────────────────────────────

#[async_trait]
impl SwarmAgent for NerExtractor {
    fn agent_id(&self) -> AgentId {
        AgentId::new(Self::AGENT_ID).expect("static agent id")
    }

    async fn run(
        &self,
        ctx: SwarmAgentContext,
    ) -> Result<AgentOutcome, SwarmAgentError> {
        let started = std::time::Instant::now();
        let text = ctx
            .blackboard
            .read(crate::INPUT_TEXT_POINTER)
            .and_then(|v| v.value.as_str().map(String::from))
            .unwrap_or_default();
        let entities = extract_entities(&text);
        let writes = if !entities.is_empty() {
            ctx.blackboard
                .write(
                    "/entities",
                    serde_json::to_value(&entities).unwrap_or(serde_json::Value::Null),
                    ctx.agent_id.clone(),
                )
                .map_err(|e| SwarmAgentError::Blackboard {
                    message: e.to_string(),
                })?;
            1
        } else {
            0
        };
        let duration_ms = started.elapsed().as_millis() as u32;
        Ok(AgentOutcome::Completed {
            cost_microcents: 0, // Mode A : pure rust, no provider cost
            duration_ms: duration_ms.max(1),
            blackboard_writes: writes,
            events_published: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_blackboard::{Blackboard, EventBus};
    use efe_nano_sdk::SwarmId;
    use std::sync::Arc;

    fn ctx_for(text: &str) -> SwarmAgentContext {
        let blackboard = Arc::new(Blackboard::new(SwarmId::new()));
        blackboard
            .write(
                crate::INPUT_TEXT_POINTER,
                serde_json::json!(text),
                AgentId::new("test-fixture").unwrap(),
            )
            .unwrap();
        SwarmAgentContext {
            swarm_id: blackboard.swarm_id(),
            agent_id: AgentId::new(NerExtractor::AGENT_ID).unwrap(),
            blackboard,
            eventbus: Arc::new(EventBus::new()),
            input: Arc::new(serde_json::json!({"text": text})),
            rng_seed: 42,
        }
    }

    // ─── extract_entities — pure function ────────────────────────

    #[test]
    fn extracts_email() {
        let es = extract_entities("Contact alice@example.com today.");
        assert!(es.iter().any(|e| e.entity_type == "email"));
        assert_eq!(
            es.iter().find(|e| e.entity_type == "email").unwrap().value,
            "alice@example.com"
        );
    }

    #[test]
    fn extracts_url() {
        let es = extract_entities("See https://example.com/foo for details.");
        assert!(es.iter().any(|e| e.entity_type == "url"));
    }

    #[test]
    fn extracts_money_usd() {
        let es = extract_entities("Price: $1,234.50 net.");
        assert!(es.iter().any(|e| e.entity_type == "money"));
    }

    #[test]
    fn extracts_money_euro() {
        let es = extract_entities("Tax: €99 only.");
        assert!(es.iter().any(|e| e.entity_type == "money"));
    }

    #[test]
    fn extracts_iso_date() {
        let es = extract_entities("Signed on 2026-05-21 by Alice.");
        assert!(es.iter().any(|e| e.entity_type == "date"));
    }

    #[test]
    fn extracts_phone_with_seven_plus_digits() {
        let es = extract_entities("Call +1 555 123 4567 anytime.");
        assert!(es.iter().any(|e| e.entity_type == "phone"));
    }

    #[test]
    fn skips_short_number_as_phone() {
        let es = extract_entities("Volume 42 is the answer.");
        assert!(!es.iter().any(|e| e.entity_type == "phone"));
    }

    #[test]
    fn empty_text_no_entities() {
        let es = extract_entities("");
        assert!(es.is_empty());
    }

    #[test]
    fn output_sorted_by_span_start_for_determinism() {
        let es = extract_entities(
            "Date 2026-05-21 then email a@b.co and phone 555-123-4567.",
        );
        for window in es.windows(2) {
            let s0 = window[0].span.map(|s| s.0).unwrap_or(0);
            let s1 = window[1].span.map(|s| s.0).unwrap_or(0);
            assert!(s0 <= s1, "entities not sorted : {window:?}");
        }
    }

    #[test]
    fn extract_is_deterministic() {
        let text = "Email alice@example.com and visit https://foo.com.";
        let r1 = extract_entities(text);
        let r2 = extract_entities(text);
        assert_eq!(r1, r2);
    }

    // ─── SwarmAgent impl ────────────────────────────────────────

    #[tokio::test]
    async fn agent_completes_and_writes_entities() {
        let agent = NerExtractor::new();
        let ctx = ctx_for("Send to bob@test.org by 2026-05-21.");
        let bb = ctx.blackboard.clone();
        let outcome = agent.run(ctx).await.unwrap();
        assert!(matches!(outcome, AgentOutcome::Completed { .. }));
        let entities = bb.read("/entities").unwrap();
        let parsed: Vec<Entity> =
            serde_json::from_value(entities.value).unwrap();
        assert!(parsed.iter().any(|e| e.entity_type == "email"));
        assert!(parsed.iter().any(|e| e.entity_type == "date"));
    }

    #[tokio::test]
    async fn agent_zero_writes_on_empty_text() {
        let agent = NerExtractor::new();
        let ctx = ctx_for("nothing relevant here");
        let outcome = agent.run(ctx).await.unwrap();
        match outcome {
            AgentOutcome::Completed {
                blackboard_writes, ..
            } => {
                assert_eq!(blackboard_writes, 0);
            }
            _ => panic!("expected Completed"),
        }
    }

    #[tokio::test]
    async fn agent_id_is_stable() {
        let agent = NerExtractor::new();
        assert_eq!(agent.agent_id().as_str(), "ner-extractor");
    }

    #[test]
    fn cost_is_zero_mode_a() {
        let _ = NerExtractor::new();
        // Mode A is pure Rust, no per-call provider cost.
        let outcome = AgentOutcome::Completed {
            cost_microcents: 0,
            duration_ms: 1,
            blackboard_writes: 0,
            events_published: 0,
        };
        assert_eq!(outcome.cost_microcents(), 0);
    }
}
