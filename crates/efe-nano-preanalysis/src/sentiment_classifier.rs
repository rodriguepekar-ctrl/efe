// SPDX-License-Identifier: UNLICENSED
//! **Sprint 13 — N21 + G11** : Sentiment classifier — Mode B stub.
//!
//! Sprint 13 ships a deterministic lexicon-based classifier
//! (positive / negative keyword lists, log-odds confidence).  Sprint
//! 14 wires a real fine-tuned BERT via `candle` for the canonical
//! Mode B behaviour.
//!
//! **Determinism** : pure function over the input text + frozen
//! lexicons.  Same input → same output.

use std::sync::OnceLock;

use async_trait::async_trait;
use efe_nano_sdk::AgentId;
use efe_swarm_orchestrator::{
    AgentOutcome, SwarmAgent, SwarmAgentContext, SwarmAgentError,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy)]
pub struct SentimentClassifier;

impl SentimentClassifier {
    pub const AGENT_ID: &'static str = "sentiment-classifier";

    pub fn new() -> Self {
        Self
    }
}

/// **N21** : the verdict written to `/sentiment`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SentimentVerdict {
    /// One of `"positive"`, `"negative"`, `"neutral"`.
    pub label: String,
    /// Confidence in `[0.0, 1.0]`.
    pub confidence: f64,
    pub positive_hits: u32,
    pub negative_hits: u32,
}

fn positive_words() -> &'static [&'static str] {
    static W: OnceLock<Vec<&'static str>> = OnceLock::new();
    W.get_or_init(|| {
        vec![
            "good", "great", "excellent", "happy", "love", "wonderful",
            "amazing", "fantastic", "best", "awesome", "perfect", "brilliant",
            "outstanding", "superb", "delightful", "pleasant", "beautiful",
            "satisfied", "thrilled", "joyful", "positive", "success",
        ]
    })
}

fn negative_words() -> &'static [&'static str] {
    static W: OnceLock<Vec<&'static str>> = OnceLock::new();
    W.get_or_init(|| {
        vec![
            "bad", "terrible", "awful", "horrible", "hate", "worst",
            "broken", "useless", "disappointing", "annoying", "angry",
            "sad", "poor", "disgusting", "frustrated", "complaint",
            "failed", "error", "wrong", "negative", "unhappy", "buggy",
        ]
    })
}

/// **N21** : pure classification.
pub fn classify(text: &str) -> SentimentVerdict {
    if text.trim().is_empty() {
        return SentimentVerdict {
            label: "neutral".into(),
            confidence: 0.0,
            positive_hits: 0,
            negative_hits: 0,
        };
    }
    let lower = text.to_lowercase();
    let tokens: Vec<&str> = lower
        .split(|c: char| !c.is_alphabetic())
        .filter(|t| !t.is_empty())
        .collect();
    let mut pos = 0u32;
    let mut neg = 0u32;
    for t in &tokens {
        if positive_words().contains(t) {
            pos += 1;
        }
        if negative_words().contains(t) {
            neg += 1;
        }
    }
    let total = pos + neg;
    if total == 0 {
        return SentimentVerdict {
            label: "neutral".into(),
            confidence: 0.5,
            positive_hits: 0,
            negative_hits: 0,
        };
    }
    let (label, confidence) = if pos > neg {
        let c = f64::from(pos) / f64::from(total);
        ("positive".to_string(), c)
    } else if neg > pos {
        let c = f64::from(neg) / f64::from(total);
        ("negative".to_string(), c)
    } else {
        ("neutral".to_string(), 0.5)
    };
    SentimentVerdict {
        label,
        confidence,
        positive_hits: pos,
        negative_hits: neg,
    }
}

#[async_trait]
impl SwarmAgent for SentimentClassifier {
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
        let verdict = classify(&text);
        ctx.blackboard
            .write(
                "/sentiment",
                serde_json::to_value(&verdict).unwrap_or(serde_json::Value::Null),
                ctx.agent_id.clone(),
            )
            .map_err(|e| SwarmAgentError::Blackboard {
                message: e.to_string(),
            })?;
        Ok(AgentOutcome::Completed {
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
            agent_id: AgentId::new(SentimentClassifier::AGENT_ID).unwrap(),
            blackboard,
            eventbus: Arc::new(EventBus::new()),
            input: Arc::new(serde_json::json!({"text": text})),
            rng_seed: 42,
        }
    }

    #[test]
    fn positive_text_classified_positive() {
        let v = classify("This is wonderful and amazing, I love it!");
        assert_eq!(v.label, "positive");
        assert!(v.confidence > 0.5);
    }

    #[test]
    fn negative_text_classified_negative() {
        let v = classify("Terrible, awful, useless and broken.");
        assert_eq!(v.label, "negative");
        assert!(v.confidence > 0.5);
    }

    #[test]
    fn mixed_text_classifies_based_on_majority() {
        let v = classify("good good good but awful");
        assert_eq!(v.label, "positive");
    }

    #[test]
    fn balanced_returns_neutral() {
        let v = classify("good bad");
        assert_eq!(v.label, "neutral");
        assert_eq!(v.confidence, 0.5);
    }

    #[test]
    fn no_keywords_returns_neutral() {
        let v = classify("blue water cloud sky");
        assert_eq!(v.label, "neutral");
    }

    #[test]
    fn empty_text_returns_zero_confidence() {
        let v = classify("");
        assert_eq!(v.label, "neutral");
        assert_eq!(v.confidence, 0.0);
    }

    #[test]
    fn deterministic_output() {
        let text = "I love this amazing product, fantastic !";
        assert_eq!(classify(text), classify(text));
    }

    #[tokio::test]
    async fn agent_writes_sentiment() {
        let agent = SentimentClassifier::new();
        let ctx = ctx_for("Wonderful and amazing experience.");
        let bb = ctx.blackboard.clone();
        let _ = agent.run(ctx).await.unwrap();
        let v = bb.read("/sentiment").unwrap();
        let parsed: SentimentVerdict = serde_json::from_value(v.value).unwrap();
        assert_eq!(parsed.label, "positive");
    }

    #[tokio::test]
    async fn agent_id_stable() {
        assert_eq!(
            SentimentClassifier::new().agent_id().as_str(),
            "sentiment-classifier"
        );
    }

    #[test]
    fn verdict_serde_round_trip() {
        let v = SentimentVerdict {
            label: "positive".into(),
            confidence: 0.85,
            positive_hits: 3,
            negative_hits: 1,
        };
        let json = serde_json::to_string(&v).unwrap();
        let back: SentimentVerdict = serde_json::from_str(&json).unwrap();
        assert_eq!(v, back);
    }
}
