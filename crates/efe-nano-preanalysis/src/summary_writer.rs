// SPDX-License-Identifier: UNLICENSED
//! **Sprint 13 — N21 + G11** : Summary writer — Mode C stub.
//!
//! Sprint 13 ships a deterministic truncation-based summariser
//! (first N sentences) so the swarm orchestrator can be tested
//! end-to-end without a hosted LLM.  Sprint 14 wires the real
//! Claude Haiku call via `efe-agentic-bridge`.
//!
//! **Determinism** : sentence segmentation is regex-based ; same
//! input → same output.

use async_trait::async_trait;
use efe_nano_sdk::AgentId;
use efe_swarm_orchestrator::{
    AgentOutcome, SwarmAgent, SwarmAgentContext, SwarmAgentError,
};

#[derive(Debug, Clone)]
pub struct SummaryWriter {
    max_sentences: usize,
    max_chars: usize,
}

impl SummaryWriter {
    pub const AGENT_ID: &'static str = "summary-writer";

    /// **N21** : default — first 3 sentences, capped at 1 000 chars.
    pub const DEFAULT_MAX_SENTENCES: usize = 3;
    pub const DEFAULT_MAX_CHARS: usize = 1_000;

    pub fn new() -> Self {
        Self {
            max_sentences: Self::DEFAULT_MAX_SENTENCES,
            max_chars: Self::DEFAULT_MAX_CHARS,
        }
    }

    pub fn with_max_sentences(mut self, n: usize) -> Self {
        self.max_sentences = n;
        self
    }

    pub fn with_max_chars(mut self, n: usize) -> Self {
        self.max_chars = n;
        self
    }
}

impl Default for SummaryWriter {
    fn default() -> Self {
        Self::new()
    }
}

/// **N21** : pure summarisation.  Sprint 13 strategy : take the
/// first `max_sentences` sentences ; then truncate to `max_chars`.
pub fn summarise(text: &str, max_sentences: usize, max_chars: usize) -> String {
    if text.trim().is_empty() {
        return String::new();
    }
    // Sentence segmentation : split on `.`, `!`, `?` followed by
    // whitespace or end-of-string.  This isn't linguistically
    // perfect but Sprint 13 is a stub.
    let mut sentences: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        current.push(c);
        if matches!(c, '.' | '!' | '?') {
            let next_is_ws_or_end = chars
                .peek()
                .map(|c| c.is_whitespace())
                .unwrap_or(true);
            if next_is_ws_or_end {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    sentences.push(trimmed);
                }
                current.clear();
            }
        }
    }
    if !current.trim().is_empty() {
        sentences.push(current.trim().to_string());
    }
    sentences.truncate(max_sentences);
    let mut summary = sentences.join(" ");
    if summary.chars().count() > max_chars {
        summary = summary.chars().take(max_chars).collect();
        // Trim trailing partial sentence to closest whitespace for
        // readability.
        if let Some(last_ws) = summary.rfind(char::is_whitespace) {
            summary.truncate(last_ws);
        }
        summary.push_str(" …");
    }
    summary
}

#[async_trait]
impl SwarmAgent for SummaryWriter {
    fn agent_id(&self) -> AgentId {
        AgentId::new(Self::AGENT_ID).expect("static agent id")
    }

    async fn run(
        &self,
        ctx: SwarmAgentContext,
    ) -> Result<AgentOutcome, SwarmAgentError> {
        let started = std::time::Instant::now();
        // Read the redacted_text first if present (security : never
        // summarise raw PII) ; fall back to /input/text.
        let text = ctx
            .blackboard
            .read("/redacted_text")
            .and_then(|v| v.value.as_str().map(String::from))
            .or_else(|| {
                ctx.blackboard
                    .read(crate::INPUT_TEXT_POINTER)
                    .and_then(|v| v.value.as_str().map(String::from))
            })
            .unwrap_or_default();
        let summary = summarise(&text, self.max_sentences, self.max_chars);
        ctx.blackboard
            .write(
                "/summary",
                serde_json::Value::String(summary),
                ctx.agent_id.clone(),
            )
            .map_err(|e| SwarmAgentError::Blackboard {
                message: e.to_string(),
            })?;
        Ok(AgentOutcome::Completed {
            // Sprint 13 stub : deterministic, no cost.
            // Sprint 14 will be Mode C (Haiku, ~10 microcents).
            cost_microcents: 0,
            duration_ms: (started.elapsed().as_millis() as u32).max(1),
            blackboard_writes: 1,
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

    fn ctx_for_with(text: &str, redacted: Option<&str>) -> SwarmAgentContext {
        let blackboard = Arc::new(Blackboard::new(SwarmId::new()));
        blackboard
            .write(
                crate::INPUT_TEXT_POINTER,
                serde_json::json!(text),
                AgentId::new("test-fixture").unwrap(),
            )
            .unwrap();
        if let Some(r) = redacted {
            blackboard
                .write(
                    "/redacted_text",
                    serde_json::json!(r),
                    AgentId::new("test-fixture").unwrap(),
                )
                .unwrap();
        }
        SwarmAgentContext {
            swarm_id: blackboard.swarm_id(),
            agent_id: AgentId::new(SummaryWriter::AGENT_ID).unwrap(),
            blackboard,
            eventbus: Arc::new(EventBus::new()),
            input: Arc::new(serde_json::json!({"text": text})),
            rng_seed: 42,
        }
    }

    #[test]
    fn summarise_takes_first_n_sentences() {
        let text = "First sentence. Second sentence. Third sentence. Fourth.";
        let s = summarise(text, 2, 1_000);
        assert!(s.contains("First sentence"));
        assert!(s.contains("Second sentence"));
        assert!(!s.contains("Third"));
    }

    #[test]
    fn summarise_handles_no_punctuation() {
        let text = "Just a long phrase without ending punctuation";
        let s = summarise(text, 3, 1_000);
        assert_eq!(s, text);
    }

    #[test]
    fn summarise_truncates_at_char_cap_with_ellipsis() {
        let long = "word ".repeat(500);
        let s = summarise(&long, 100, 50);
        assert!(s.chars().count() <= 60);
        assert!(s.ends_with('…') || s.is_empty());
    }

    #[test]
    fn summarise_empty_input_returns_empty() {
        assert_eq!(summarise("", 3, 100), "");
    }

    #[test]
    fn summarise_deterministic() {
        let text = "First. Second. Third.";
        assert_eq!(summarise(text, 2, 100), summarise(text, 2, 100));
    }

    #[tokio::test]
    async fn agent_writes_summary_from_input_text() {
        let agent = SummaryWriter::new();
        let ctx = ctx_for_with("Sentence one. Sentence two.", None);
        let bb = ctx.blackboard.clone();
        let _ = agent.run(ctx).await.unwrap();
        let s = bb.read("/summary").unwrap();
        assert!(s.value.as_str().unwrap().contains("Sentence"));
    }

    #[tokio::test]
    async fn agent_prefers_redacted_text_when_present() {
        let agent = SummaryWriter::new();
        let ctx = ctx_for_with(
            "Original with email@x.co",
            Some("Redacted with <EMAIL_0>"),
        );
        let bb = ctx.blackboard.clone();
        let _ = agent.run(ctx).await.unwrap();
        let s = bb.read("/summary").unwrap();
        let str_val = s.value.as_str().unwrap();
        assert!(str_val.contains("<EMAIL_0>"));
        assert!(!str_val.contains("email@x.co"));
    }

    #[tokio::test]
    async fn agent_id_stable() {
        assert_eq!(SummaryWriter::new().agent_id().as_str(), "summary-writer");
    }

    #[test]
    fn builder_max_chars() {
        let w = SummaryWriter::new().with_max_chars(50);
        assert_eq!(w.max_chars, 50);
    }

    #[test]
    fn builder_max_sentences() {
        let w = SummaryWriter::new().with_max_sentences(5);
        assert_eq!(w.max_sentences, 5);
    }
}
