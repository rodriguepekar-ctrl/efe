// SPDX-License-Identifier: UNLICENSED
//! **Sprint 13 — N21 + G11 + P4** : PII redactor — Mode A.
//!
//! Detects emails, phone numbers, SSNs, and credit-card numbers ;
//! replaces each with a typed token `<EMAIL_0>`, `<PHONE_1>`, etc.
//! Writes :
//! - `/redactions` : Vec<Redaction> with `original_span` +
//!   `redaction_token` (transparent audit trail for the agentic
//!   LLM downstream).
//! - `/redacted_text` : the input text with PII replaced.
//!
//! **Critical security invariant** (security_pii_redactor_runs_before_claude
//! test) : this agent MUST run before any hosted-LLM call in the
//! swarm.  Architecture P4 / Incident I-2026-08-22.

use std::sync::OnceLock;

use async_trait::async_trait;
use efe_nano_sdk::{AgentId, Redaction};
use efe_swarm_orchestrator::{
    AgentOutcome, SwarmAgent, SwarmAgentContext, SwarmAgentError,
};
use regex::Regex;

#[derive(Debug, Default, Clone, Copy)]
pub struct PiiRedactor;

impl PiiRedactor {
    pub const AGENT_ID: &'static str = "pii-redactor";

    pub fn new() -> Self {
        Self
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Regexes
// ─────────────────────────────────────────────────────────────────────

fn email_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"[a-zA-Z0-9._%+\-]+@[a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}")
            .expect("email")
    })
}

fn phone_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"\+?\d[\d \-().]{7,}\d").expect("phone")
    })
}

fn ssn_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").expect("ssn")
    })
}

fn credit_card_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    // 13-19 digit groups separated by space/dash, common shape.
    R.get_or_init(|| {
        Regex::new(r"\b(?:\d[ \-]?){13,19}\b").expect("cc")
    })
}

// ─────────────────────────────────────────────────────────────────────
//  Redaction logic
// ─────────────────────────────────────────────────────────────────────

/// **N21** : the result of running the redactor on a piece of text.
#[derive(Debug, Clone, PartialEq)]
pub struct RedactionResult {
    pub redacted_text: String,
    pub redactions: Vec<Redaction>,
}

/// **N21 + P4** : pure redaction.
///
/// Algorithm :
/// 1. Walk every category in priority order : email → cc → ssn → phone.
/// 2. For each match that doesn't overlap an existing redaction,
///    add a `Redaction` row + record the replacement token.
/// 3. Apply replacements right-to-left on the original text (avoid
///    span recomputation).
///
/// Token format : `<CATEGORY_INDEX>` where INDEX is per-category
/// (0-based, increases as more PII of the same category is found).
pub fn redact(text: &str) -> RedactionResult {
    let mut all_redactions: Vec<Redaction> = Vec::new();
    let mut per_category_count: std::collections::HashMap<&str, usize> =
        std::collections::HashMap::new();

    // Priority : credit cards first (most specific) ; phones last
    // (most permissive shape).
    // Email is detected first because email-shaped digits aren't a
    // credit card.
    let categories: &[(&Regex, &str)] = &[
        (email_re(), "email"),
        (credit_card_re(), "credit_card"),
        (ssn_re(), "ssn"),
        (phone_re(), "phone"),
    ];
    for (re, category) in categories {
        for cap in re.find_iter(text) {
            let span = (cap.start() as u32, cap.end() as u32);
            // Skip if overlaps an existing redaction.
            if all_redactions.iter().any(|r| {
                !(span.1 <= r.original_span.0 || span.0 >= r.original_span.1)
            }) {
                continue;
            }
            // For phones : reject < 7 digits in the match (avoids
            // version strings, model numbers).
            if *category == "phone" {
                let digits = cap.as_str().chars().filter(|c| c.is_ascii_digit()).count();
                if digits < 7 {
                    continue;
                }
            }
            // For credit cards : reject < 13 digits.
            if *category == "credit_card" {
                let digits = cap.as_str().chars().filter(|c| c.is_ascii_digit()).count();
                if digits < 13 {
                    continue;
                }
            }
            let idx = per_category_count
                .entry(*category)
                .and_modify(|i| *i += 1)
                .or_insert(0);
            let token = format!(
                "<{}_{}>",
                category.to_uppercase(),
                idx
            );
            all_redactions.push(Redaction {
                redaction_type: category.to_string(),
                original_span: span,
                redaction_token: token,
            });
        }
    }
    // Sort by span start.
    all_redactions.sort_by_key(|r| r.original_span.0);

    // Apply : walk right-to-left, replace each span with token.
    let mut redacted = text.to_string();
    for r in all_redactions.iter().rev() {
        let (s, e) = (r.original_span.0 as usize, r.original_span.1 as usize);
        if s <= redacted.len() && e <= redacted.len() && s <= e {
            redacted.replace_range(s..e, &r.redaction_token);
        }
    }

    RedactionResult {
        redacted_text: redacted,
        redactions: all_redactions,
    }
}

// ─────────────────────────────────────────────────────────────────────
//  SwarmAgent impl
// ─────────────────────────────────────────────────────────────────────

#[async_trait]
impl SwarmAgent for PiiRedactor {
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
        let result = redact(&text);
        let writes = 2u32;
        ctx.blackboard
            .write(
                "/redactions",
                serde_json::to_value(&result.redactions).unwrap_or(serde_json::Value::Null),
                ctx.agent_id.clone(),
            )
            .map_err(|e| SwarmAgentError::Blackboard {
                message: e.to_string(),
            })?;
        ctx.blackboard
            .write(
                "/redacted_text",
                serde_json::Value::String(result.redacted_text),
                ctx.agent_id.clone(),
            )
            .map_err(|e| SwarmAgentError::Blackboard {
                message: e.to_string(),
            })?;
        Ok(AgentOutcome::Completed {
            cost_microcents: 0,
            duration_ms: (started.elapsed().as_millis() as u32).max(1),
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
            agent_id: AgentId::new(PiiRedactor::AGENT_ID).unwrap(),
            blackboard,
            eventbus: Arc::new(EventBus::new()),
            input: Arc::new(serde_json::json!({"text": text})),
            rng_seed: 42,
        }
    }

    #[test]
    fn redacts_single_email() {
        let r = redact("Contact alice@example.com today.");
        assert_eq!(r.redactions.len(), 1);
        assert_eq!(r.redactions[0].redaction_type, "email");
        assert!(r.redacted_text.contains("<EMAIL_0>"));
        assert!(!r.redacted_text.contains("alice@example.com"));
    }

    #[test]
    fn redacts_multiple_emails_with_increasing_index() {
        let r = redact("Email a@b.co and also c@d.co please.");
        assert_eq!(r.redactions.len(), 2);
        assert_eq!(r.redactions[0].redaction_token, "<EMAIL_0>");
        assert_eq!(r.redactions[1].redaction_token, "<EMAIL_1>");
    }

    #[test]
    fn redacts_ssn() {
        let r = redact("SSN 123-45-6789.");
        assert!(r.redactions.iter().any(|x| x.redaction_type == "ssn"));
        assert!(r.redacted_text.contains("<SSN_0>"));
    }

    #[test]
    fn redacts_credit_card() {
        let r = redact("Card 4111 1111 1111 1111 used.");
        assert!(r.redactions.iter().any(|x| x.redaction_type == "credit_card"));
    }

    #[test]
    fn redacts_phone() {
        let r = redact("Call 555-123-4567 anytime.");
        assert!(r.redactions.iter().any(|x| x.redaction_type == "phone"));
    }

    #[test]
    fn mixed_pii_redacted_all() {
        let r = redact(
            "Hi I am alice@test.com, my phone is 555-123-4567 and \
             SSN 123-45-6789.",
        );
        assert!(r.redactions.iter().any(|x| x.redaction_type == "email"));
        assert!(r.redactions.iter().any(|x| x.redaction_type == "phone"));
        assert!(r.redactions.iter().any(|x| x.redaction_type == "ssn"));
        assert!(!r.redacted_text.contains("alice@test.com"));
        assert!(!r.redacted_text.contains("123-45-6789"));
    }

    #[test]
    fn no_pii_no_redactions() {
        let r = redact("Nothing sensitive here.");
        assert!(r.redactions.is_empty());
        assert_eq!(r.redacted_text, "Nothing sensitive here.");
    }

    #[test]
    fn deterministic_redaction() {
        let text = "Call alice@b.co at 555-123-4567 please.";
        let r1 = redact(text);
        let r2 = redact(text);
        assert_eq!(r1, r2);
    }

    #[test]
    fn redactions_sorted_by_span() {
        let r = redact(
            "Three contacts : a@b.co, then 555-123-4567, then 999-99-9999.",
        );
        for window in r.redactions.windows(2) {
            assert!(window[0].original_span.0 <= window[1].original_span.0);
        }
    }

    #[tokio::test]
    async fn agent_writes_redactions_and_redacted_text() {
        let agent = PiiRedactor::new();
        let ctx = ctx_for("Contact alice@example.com today.");
        let bb = ctx.blackboard.clone();
        let _ = agent.run(ctx).await.unwrap();
        let red = bb.read("/redactions").unwrap();
        let parsed: Vec<Redaction> = serde_json::from_value(red.value).unwrap();
        assert!(!parsed.is_empty());
        let txt = bb.read("/redacted_text").unwrap();
        assert!(txt.value.as_str().unwrap().contains("<EMAIL_0>"));
    }

    #[tokio::test]
    async fn agent_no_pii_writes_empty_redactions() {
        let agent = PiiRedactor::new();
        let ctx = ctx_for("Clean text with no sensitive data.");
        let bb = ctx.blackboard.clone();
        let _ = agent.run(ctx).await.unwrap();
        let red = bb.read("/redactions").unwrap();
        let parsed: Vec<Redaction> = serde_json::from_value(red.value).unwrap();
        assert!(parsed.is_empty());
    }

    #[tokio::test]
    async fn agent_id_is_stable() {
        assert_eq!(PiiRedactor::new().agent_id().as_str(), "pii-redactor");
    }

    /// **Security G11 + P4** : the redacted text never contains the
    /// original PII byte sequences (defense for P4 — Incident
    /// I-2026-08-22).
    #[test]
    fn redacted_text_never_contains_original_pii() {
        let original = "Sensitive : alice@example.com and 555-12-3456 \
                        plus 4111 1111 1111 1111.";
        let r = redact(original);
        assert!(!r.redacted_text.contains("alice@example.com"));
        assert!(!r.redacted_text.contains("4111 1111 1111 1111"));
    }
}
