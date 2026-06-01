// SPDX-License-Identifier: UNLICENSED
//! **Sprint 13 — N21 + G11** : Language detector — Mode A.
//!
//! Lightweight heuristic — counts high-frequency stopwords from a
//! small set of European + Asian languages, returns the highest-
//! scoring ISO-639-1 code.  Sprint 13 ships ~6 languages ; Sprint
//! 14 will plug `whatlang` for the full coverage.
//!
//! **Determinism** : same input → same output ; counts are simple
//! arithmetic, no RNG.

use std::collections::HashMap;
use std::sync::OnceLock;

use async_trait::async_trait;
use efe_nano_sdk::AgentId;
use efe_swarm_orchestrator::{
    AgentOutcome, SwarmAgent, SwarmAgentContext, SwarmAgentError,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct LanguageDetector;

impl LanguageDetector {
    pub const AGENT_ID: &'static str = "language-detector";

    pub fn new() -> Self {
        Self
    }
}

/// **N21** : default ISO-639-1 code returned when no signal beats
/// the default threshold (5 stopword hits).
pub const FALLBACK_LANGUAGE: &str = "en";

/// **N21** : minimum stopword hits to declare a language with
/// reasonable confidence.
pub const MIN_HITS_FOR_CONFIDENCE: usize = 3;

fn stopwords() -> &'static HashMap<&'static str, Vec<&'static str>> {
    static W: OnceLock<HashMap<&'static str, Vec<&'static str>>> = OnceLock::new();
    W.get_or_init(|| {
        let mut m = HashMap::new();
        // English
        m.insert(
            "en",
            vec![
                "the", "and", "is", "to", "of", "in", "a", "that", "it", "for", "on",
                "with", "as", "by", "this", "be", "are", "was",
            ],
        );
        // French
        m.insert(
            "fr",
            vec![
                "le", "la", "les", "de", "et", "à", "un", "une", "des", "que", "qui",
                "pour", "dans", "ce", "sur", "avec", "par", "sont", "est", "pas",
            ],
        );
        // German
        m.insert(
            "de",
            vec![
                "der", "die", "das", "und", "ist", "in", "ein", "eine", "den", "mit",
                "von", "für", "auf", "sich", "nicht", "sein", "wird",
            ],
        );
        // Spanish
        m.insert(
            "es",
            vec![
                "el", "la", "de", "que", "y", "en", "un", "una", "los", "las", "por",
                "con", "para", "se", "es", "del", "lo", "como",
            ],
        );
        // Italian
        m.insert(
            "it",
            vec![
                "il", "la", "di", "che", "e", "un", "una", "in", "per", "con",
                "non", "del", "della", "al", "si", "lo", "come",
            ],
        );
        // Portuguese
        m.insert(
            "pt",
            vec![
                "o", "a", "de", "e", "do", "da", "que", "em", "um", "para", "com",
                "não", "os", "no", "se", "uma", "as", "dos",
            ],
        );
        m
    })
}

/// **N21** : pure detection function.
///
/// Returns `Some((code, hits))` if at least `MIN_HITS_FOR_CONFIDENCE`
/// stopwords matched ; `None` otherwise.
pub fn detect_language(text: &str) -> Option<(String, usize)> {
    if text.trim().is_empty() {
        return None;
    }
    let normalized = text.to_lowercase();
    // Tokenize on whitespace + simple punctuation strip.
    let tokens: Vec<&str> = normalized
        .split(|c: char| !c.is_alphabetic())
        .filter(|t| !t.is_empty())
        .collect();
    let mut scores: HashMap<&str, usize> = HashMap::new();
    for (lang, words) in stopwords() {
        let mut hits = 0usize;
        for w in words {
            for t in &tokens {
                if t == w {
                    hits += 1;
                }
            }
        }
        scores.insert(lang, hits);
    }
    // Pick the highest score deterministically (sorted by lang code
    // on ties for replay determinism).
    let mut sorted: Vec<(&str, usize)> = scores.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let best = sorted.into_iter().next()?;
    if best.1 < MIN_HITS_FOR_CONFIDENCE {
        return None;
    }
    Some((best.0.to_string(), best.1))
}

#[async_trait]
impl SwarmAgent for LanguageDetector {
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
        let detected = detect_language(&text)
            .map(|(c, _)| c)
            .unwrap_or_else(|| FALLBACK_LANGUAGE.to_string());
        ctx.blackboard
            .write(
                "/language",
                serde_json::Value::String(detected),
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
            agent_id: AgentId::new(LanguageDetector::AGENT_ID).unwrap(),
            blackboard,
            eventbus: Arc::new(EventBus::new()),
            input: Arc::new(serde_json::json!({"text": text})),
            rng_seed: 42,
        }
    }

    #[test]
    fn detects_english() {
        let (code, hits) = detect_language(
            "The quick brown fox is jumping over the lazy dog and \
             that is a fact for sure.",
        )
        .unwrap();
        assert_eq!(code, "en");
        assert!(hits >= MIN_HITS_FOR_CONFIDENCE);
    }

    #[test]
    fn detects_french() {
        let (code, hits) = detect_language(
            "Le chat est sur la table et le chien est dans le jardin avec \
             un os pour le déjeuner.",
        )
        .unwrap();
        assert_eq!(code, "fr");
        assert!(hits >= MIN_HITS_FOR_CONFIDENCE);
    }

    #[test]
    fn detects_german() {
        let (code, _) = detect_language(
            "Das Haus ist groß und der Garten ist schön mit den \
             Blumen für die Familie.",
        )
        .unwrap();
        assert_eq!(code, "de");
    }

    #[test]
    fn detects_spanish() {
        let (code, _) = detect_language(
            "El gato está en la mesa y un perro está en el jardín \
             con la familia.",
        )
        .unwrap();
        assert_eq!(code, "es");
    }

    #[test]
    fn empty_text_returns_none() {
        assert!(detect_language("").is_none());
        assert!(detect_language("   ").is_none());
    }

    #[test]
    fn below_threshold_returns_none() {
        // Just one stopword.
        assert!(detect_language("the").is_none());
    }

    #[test]
    fn deterministic_output() {
        let text = "The quick brown fox is jumping over the lazy dog and the cat.";
        let r1 = detect_language(text);
        let r2 = detect_language(text);
        assert_eq!(r1, r2);
    }

    #[tokio::test]
    async fn agent_writes_detected_language() {
        let agent = LanguageDetector::new();
        let ctx = ctx_for(
            "Le chat est sur la table et le chien est dans le jardin avec un os.",
        );
        let bb = ctx.blackboard.clone();
        let _ = agent.run(ctx).await.unwrap();
        let lang = bb.read("/language").unwrap();
        assert_eq!(lang.value, serde_json::json!("fr"));
    }

    #[tokio::test]
    async fn agent_writes_fallback_on_empty() {
        let agent = LanguageDetector::new();
        let ctx = ctx_for("");
        let bb = ctx.blackboard.clone();
        let _ = agent.run(ctx).await.unwrap();
        let lang = bb.read("/language").unwrap();
        assert_eq!(lang.value, serde_json::json!("en"));
    }

    #[tokio::test]
    async fn agent_id_is_stable() {
        assert_eq!(
            LanguageDetector::new().agent_id().as_str(),
            "language-detector"
        );
    }
}
