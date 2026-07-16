// SPDX-License-Identifier: UNLICENSED
//! `CanonicalReformulator` — convertit Claude XML / GPT JSON / Gemini MD
//! vers les 5 canonical schemas.
//!
//! ## Stratégie de parsing
//!
//! Pour chaque (`SourceDialect`, `CanonicalSchemaId`) on a une heuristique :
//!
//! - **JSON parsing first** : si le texte contient un JSON valide, on tente
//!   de le mapper sur le schema cible.
//! - **XML extraction** (Claude) : on extrait le contenu de `<plan>`, `<code>`,
//!   `<claim>`, `<argument>`, `<synthesis>` (tags conventionnels).
//! - **Markdown parsing** (Gemini) : on extrait code blocks ```rust ... ```
//!   pour `code_v1`, le 1er paragraphe pour `claim_v1`, etc.
//! - **Fallback texte brut** : si rien ne marche, on encapsule le texte
//!   brut comme `claim` ou `synthesis` selon le contexte.
//!
//! Cette stratégie est **best-effort** et déterministe — pas de LLM
//! pour faire la conversion, juste du parsing pur Rust.

use uuid::Uuid;

use crate::canonical::{
    ArgumentStance, CanonicalArgument, CanonicalClaim, CanonicalCode, CanonicalPlan,
    CanonicalSchemaId, CanonicalSynthesis, ClaimEvidence, CodeLanguage, PlanStep,
    SynthesisCitation, SynthesisSection,
};
use crate::error::{MultiLlmError, MultiLlmResult};

use super::engine::{Reformulated, ReformulatorEngine};
use super::{Reformulator, SourceDialect};

/// Reformulator unifié pour les 5 schemas canoniques.
#[derive(Clone, Default)]
pub struct CanonicalReformulator;

impl CanonicalReformulator {
    /// Construit (stateless).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reformule texte brut vers `CanonicalPlan`.
    pub fn to_plan(
        &self,
        raw: &str,
        from_dialect: SourceDialect,
    ) -> MultiLlmResult<CanonicalPlan> {
        // 1. Tentative JSON direct.
        if let Some(p) = try_parse_json::<CanonicalPlan>(raw) {
            return Ok(p);
        }
        // 2. Extraction selon dialecte.
        let title = extract_first_match(raw, &["<title>", "# "]);
        let steps_raw: Vec<&str> = raw
            .lines()
            .filter(|l| {
                let l = l.trim();
                // Détecte "1. action", "- step", "Step 1: ...", "<step>..."
                l.starts_with(|c: char| c.is_ascii_digit())
                    || l.starts_with("- ")
                    || l.starts_with("* ")
                    || l.to_lowercase().starts_with("step")
                    || l.contains("<step>")
            })
            .collect();
        let mut steps = Vec::new();
        for (i, line) in steps_raw.iter().enumerate().take(30) {
            let cleaned = strip_xml_tags(line.trim())
                .trim_start_matches(|c: char| {
                    c.is_ascii_digit() || c == '.' || c == ')' || c == '-' || c == '*' || c.is_whitespace()
                })
                .trim()
                .to_string();
            if !cleaned.is_empty() {
                steps.push(PlanStep {
                    id: (i + 1) as u32,
                    action: truncate(&cleaned, 500),
                    deps: vec![],
                    estimated_duration_min: None,
                });
            }
        }
        if steps.is_empty() {
            // Fallback : tout le texte = 1 step.
            steps.push(PlanStep {
                id: 1,
                action: truncate(raw, 500),
                deps: vec![],
                estimated_duration_min: None,
            });
        }
        Ok(CanonicalPlan {
            plan_id: Uuid::now_v7(),
            title: truncate(&title, 200),
            steps,
            rationale: String::new(),
        })
        .and_then(|p| {
            // Vérifie qu'on a au moins 1 step (invariant min)
            if p.steps.is_empty() {
                Err(MultiLlmError::ReformulationFailed {
                    from_dialect: from_dialect.tag().to_string(),
                    to_schema: "canonical/plan_v1".into(),
                    reason: "no steps extracted".into(),
                })
            } else {
                Ok(p)
            }
        })
    }

    /// Reformule texte brut vers `CanonicalCode`.
    pub fn to_code(
        &self,
        raw: &str,
        from_dialect: SourceDialect,
    ) -> MultiLlmResult<CanonicalCode> {
        // 1. JSON direct.
        if let Some(c) = try_parse_json::<CanonicalCode>(raw) {
            return Ok(c);
        }
        // 2. Markdown code blocks ```<lang>\n...\n``` (Gemini, GPT MD)
        let (lang_str, source) = extract_first_code_block(raw)
            .unwrap_or_else(|| ("rust".to_string(), raw.to_string()));
        let language = parse_code_language(&lang_str)
            .ok_or_else(|| MultiLlmError::ReformulationFailed {
                from_dialect: from_dialect.tag().to_string(),
                to_schema: "canonical/code_v1".into(),
                reason: format!("unknown language: {lang_str}"),
            })?;
        Ok(CanonicalCode {
            language,
            source: truncate(&source, 50_000),
            tests: vec![],
            deps: vec![],
            complexity_estimate: None,
        })
    }

    /// Reformule texte brut vers `CanonicalClaim`.
    pub fn to_claim(
        &self,
        raw: &str,
        from_dialect: SourceDialect,
    ) -> MultiLlmResult<CanonicalClaim> {
        // 1. JSON direct.
        if let Some(c) = try_parse_json::<CanonicalClaim>(raw) {
            return Ok(c);
        }
        // 2. Extraction du contenu de balise <claim>...</claim> (Claude).
        let claim_text = if let Some(inner) = extract_tag_content(raw, "claim") {
            inner.to_string()
        } else {
            // Fallback : 1ère phrase (jusqu'au 1er '.', '!', '?').
            first_sentence(raw).to_string()
        };
        // 3. Évidences : balises <source> ou Markdown footnotes [^1]: …
        let evidence = extract_evidence(raw);
        // 4. Confiance : si le LLM a écrit "confidence: 0.X" on l'extrait,
        //    sinon défaut 0.7 (médian).
        let confidence = extract_confidence(raw).unwrap_or(0.7);
        Ok(CanonicalClaim {
            claim: truncate(claim_text.trim(), 1000),
            evidence,
            confidence,
            domain: String::new(),
        })
        .and_then(|c| {
            if c.claim.is_empty() {
                Err(MultiLlmError::ReformulationFailed {
                    from_dialect: from_dialect.tag().to_string(),
                    to_schema: "canonical/claim_v1".into(),
                    reason: "empty claim".into(),
                })
            } else {
                Ok(c)
            }
        })
    }

    /// Reformule vers `CanonicalArgument`.
    pub fn to_argument(
        &self,
        raw: &str,
        stance_hint: ArgumentStance,
        from_dialect: SourceDialect,
    ) -> MultiLlmResult<CanonicalArgument> {
        if let Some(a) = try_parse_json::<CanonicalArgument>(raw) {
            return Ok(a);
        }
        let thesis = extract_tag_content(raw, "thesis")
            .unwrap_or_else(|| first_sentence(raw))
            .to_string();
        let reasoning: Vec<String> = raw
            .lines()
            .filter(|l| {
                let l = l.trim();
                l.starts_with("- ")
                    || l.starts_with("* ")
                    || l.starts_with(|c: char| c.is_ascii_digit())
            })
            .take(10)
            .map(|l| {
                truncate(
                    l.trim_start_matches(|c: char| {
                        c.is_ascii_digit() || c == '.' || c == ')' || c == '-' || c == '*' || c.is_whitespace()
                    })
                    .trim(),
                    1000,
                )
            })
            .filter(|s| !s.is_empty())
            .collect();
        let reasoning = if reasoning.is_empty() {
            vec![truncate(raw, 1000)]
        } else {
            reasoning
        };
        Ok(CanonicalArgument {
            thesis: truncate(&thesis, 500),
            reasoning,
            sources: vec![],
            addresses_counter: None,
            stance: stance_hint,
        })
        .and_then(|a| {
            if a.thesis.is_empty() {
                Err(MultiLlmError::ReformulationFailed {
                    from_dialect: from_dialect.tag().to_string(),
                    to_schema: "canonical/argument_v1".into(),
                    reason: "empty thesis".into(),
                })
            } else {
                Ok(a)
            }
        })
    }

    /// Reformule vers `CanonicalSynthesis`.
    pub fn to_synthesis(
        &self,
        raw: &str,
        from_dialect: SourceDialect,
        llm_source: &str,
    ) -> MultiLlmResult<CanonicalSynthesis> {
        if let Some(s) = try_parse_json::<CanonicalSynthesis>(raw) {
            return Ok(s);
        }
        // 1. Sections : extraction des headings markdown `## ...`.
        let mut sections = Vec::new();
        let mut current_heading = String::new();
        let mut current_body = String::new();
        for line in raw.lines() {
            if let Some(rest) = line.strip_prefix("## ") {
                if !current_heading.is_empty() || !current_body.is_empty() {
                    sections.push(SynthesisSection {
                        heading: current_heading.clone(),
                        body: current_body.trim().to_string(),
                    });
                }
                current_heading = rest.trim().to_string();
                current_body.clear();
            } else {
                current_body.push_str(line);
                current_body.push('\n');
            }
        }
        if !current_heading.is_empty() || !current_body.trim().is_empty() {
            sections.push(SynthesisSection {
                heading: current_heading,
                body: current_body.trim().to_string(),
            });
        }
        let summary = truncate(
            sections
                .first()
                .map(|s| s.body.as_str())
                .unwrap_or_else(|| raw),
            5000,
        );
        let citations = vec![SynthesisCitation {
            llm_source: short_llm_id(llm_source),
            snippet: truncate(raw, 500),
        }];
        Ok(CanonicalSynthesis {
            summary,
            citations,
            uncertainty: String::new(),
            sections,
        })
        .and_then(|s| {
            if s.summary.is_empty() {
                Err(MultiLlmError::ReformulationFailed {
                    from_dialect: from_dialect.tag().to_string(),
                    to_schema: "canonical/synthesis_v1".into(),
                    reason: "empty summary".into(),
                })
            } else {
                Ok(s)
            }
        })
    }

    /// Routeur générique selon `CanonicalSchemaId`.  Sortie typée
    /// via `serde_json::Value` pour cohérence avec le crate Phase 1.
    pub fn route(
        &self,
        raw: &str,
        from_dialect: SourceDialect,
        target: CanonicalSchemaId,
        llm_source: &str,
    ) -> MultiLlmResult<serde_json::Value> {
        match target {
            CanonicalSchemaId::PlanV1 => Ok(serde_json::to_value(self.to_plan(raw, from_dialect)?)
                .map_err(|e| MultiLlmError::ReformulationFailed {
                    from_dialect: from_dialect.tag().into(),
                    to_schema: target.asset_id().into(),
                    reason: e.to_string(),
                })?),
            CanonicalSchemaId::CodeV1 => Ok(serde_json::to_value(self.to_code(raw, from_dialect)?)
                .map_err(|e| MultiLlmError::ReformulationFailed {
                    from_dialect: from_dialect.tag().into(),
                    to_schema: target.asset_id().into(),
                    reason: e.to_string(),
                })?),
            CanonicalSchemaId::ClaimV1 => {
                Ok(serde_json::to_value(self.to_claim(raw, from_dialect)?).map_err(|e| {
                    MultiLlmError::ReformulationFailed {
                        from_dialect: from_dialect.tag().into(),
                        to_schema: target.asset_id().into(),
                        reason: e.to_string(),
                    }
                })?)
            }
            CanonicalSchemaId::ArgumentV1 => {
                Ok(serde_json::to_value(
                    self.to_argument(raw, ArgumentStance::Neutral, from_dialect)?,
                )
                .map_err(|e| MultiLlmError::ReformulationFailed {
                    from_dialect: from_dialect.tag().into(),
                    to_schema: target.asset_id().into(),
                    reason: e.to_string(),
                })?)
            }
            CanonicalSchemaId::SynthesisV1 => Ok(serde_json::to_value(
                self.to_synthesis(raw, from_dialect, llm_source)?,
            )
            .map_err(|e| MultiLlmError::ReformulationFailed {
                from_dialect: from_dialect.tag().into(),
                to_schema: target.asset_id().into(),
                reason: e.to_string(),
            })?),
        }
    }
}

// ─── Fil B.0 : impl du trait pluggable ────────────────────────────

/// Le déterministe **est** un [`Reformulator`] : c'est le défaut et le repli.
/// Chaque sortie est estampillée [`ReformulatorEngine::Rule`]. On délègue aux
/// méthodes inhérentes (`route`, `to_argument`) — nommées différemment des
/// méthodes du trait (`to_canonical`, `to_argument_value`), donc `self.route`
/// et `self.to_argument` ci-dessous résolvent sans ambiguïté vers l'inhérent.
#[async_trait::async_trait]
impl Reformulator for CanonicalReformulator {
    async fn to_canonical(
        &self,
        raw: &str,
        from: SourceDialect,
        target: crate::canonical::CanonicalSchemaId,
        llm_source: &str,
    ) -> MultiLlmResult<Reformulated> {
        let value = self.route(raw, from, target, llm_source)?;
        Ok(Reformulated::new(value, ReformulatorEngine::Rule))
    }

    async fn to_argument_value(
        &self,
        raw: &str,
        stance: ArgumentStance,
        from: SourceDialect,
    ) -> MultiLlmResult<Reformulated> {
        let arg = self.to_argument(raw, stance, from)?;
        let value = serde_json::to_value(arg).map_err(|e| MultiLlmError::ReformulationFailed {
            from_dialect: from.tag().to_string(),
            to_schema: "canonical/argument_v1".to_string(),
            reason: e.to_string(),
        })?;
        Ok(Reformulated::new(value, ReformulatorEngine::Rule))
    }
}

// ─── Helpers ──────────────────────────────────────────────────────

fn try_parse_json<T: serde::de::DeserializeOwned>(raw: &str) -> Option<T> {
    // Cherche le 1er '{' (fortuit s'il y a du texte de préambule).
    let trimmed = raw.trim();
    if let Ok(v) = serde_json::from_str::<T>(trimmed) {
        return Some(v);
    }
    // Strip code fence ```json ... ```
    if let Some(rest) = trimmed.strip_prefix("```json") {
        if let Some(end) = rest.rfind("```") {
            if let Ok(v) = serde_json::from_str::<T>(&rest[..end].trim()) {
                return Some(v);
            }
        }
    }
    // Trouve le 1er { et tente jusqu'au dernier }.
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            if end > start {
                if let Ok(v) = serde_json::from_str::<T>(&trimmed[start..=end]) {
                    return Some(v);
                }
            }
        }
    }
    None
}

fn extract_tag_content<'a>(raw: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let s = raw.find(&open)?;
    let after = &raw[s + open.len()..];
    let e = after.find(&close)?;
    Some(&after[..e])
}

fn extract_first_match(raw: &str, patterns: &[&str]) -> String {
    for p in patterns {
        if let Some(start) = raw.find(p) {
            let after = &raw[start + p.len()..];
            let end = after.find(|c: char| c == '\n' || c == '<').unwrap_or(after.len());
            return after[..end].trim().to_string();
        }
    }
    String::new()
}

fn strip_xml_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

fn extract_first_code_block(raw: &str) -> Option<(String, String)> {
    let start = raw.find("```")?;
    let after = &raw[start + 3..];
    // Le langage est jusqu'au premier '\n'.
    let lang_end = after.find('\n')?;
    let lang = after[..lang_end].trim().to_string();
    let body = &after[lang_end + 1..];
    let end = body.find("```")?;
    Some((lang, body[..end].to_string()))
}

fn parse_code_language(s: &str) -> Option<CodeLanguage> {
    match s.to_lowercase().as_str() {
        "rust" | "rs" => Some(CodeLanguage::Rust),
        "python" | "py" => Some(CodeLanguage::Python),
        "typescript" | "ts" => Some(CodeLanguage::Typescript),
        "go" | "golang" => Some(CodeLanguage::Go),
        "java" => Some(CodeLanguage::Java),
        "cpp" | "c++" | "cxx" => Some(CodeLanguage::Cpp),
        "sql" => Some(CodeLanguage::Sql),
        "bash" | "sh" | "shell" => Some(CodeLanguage::Bash),
        _ => None,
    }
}

fn first_sentence(raw: &str) -> &str {
    let end = raw
        .find(|c: char| c == '.' || c == '!' || c == '?')
        .map(|i| i + 1)
        .unwrap_or(raw.len().min(500));
    raw[..end].trim()
}

fn extract_evidence(raw: &str) -> Vec<ClaimEvidence> {
    let mut out = Vec::new();
    // 1. Tags <source>...</source>.
    let mut search_from = 0;
    while let Some(s) = raw[search_from..].find("<source>") {
        let s = search_from + s + "<source>".len();
        if let Some(e) = raw[s..].find("</source>") {
            let excerpt = raw[s..s + e].trim().to_string();
            out.push(ClaimEvidence {
                source: format!("source-{}", out.len() + 1),
                excerpt: truncate(&excerpt, 500),
            });
            search_from = s + e + "</source>".len();
        } else {
            break;
        }
    }
    // 2. Markdown footnotes [^N]: …
    for line in raw.lines() {
        if line.trim_start().starts_with("[^") {
            if let Some(end) = line.find("]:") {
                let label = &line[..end + 1];
                let excerpt = line[end + 2..].trim();
                if !excerpt.is_empty() {
                    out.push(ClaimEvidence {
                        source: truncate(label, 200),
                        excerpt: truncate(excerpt, 500),
                    });
                }
            }
        }
    }
    out.truncate(10);
    out
}

fn extract_confidence(raw: &str) -> Option<f32> {
    let lower = raw.to_lowercase();
    let needle = "confidence";
    let idx = lower.find(needle)?;
    let after = &lower[idx + needle.len()..];
    // Skip ":", "=", ", ", "score", etc.
    let after = after.trim_start_matches(|c: char| {
        !c.is_ascii_digit() && c != '-'
    });
    // Lit le 1er nombre flottant : chiffres + au plus 1 point.
    let mut end = 0;
    let mut seen_dot = false;
    for (i, c) in after.char_indices() {
        if c.is_ascii_digit() || (c == '-' && i == 0) {
            end = i + c.len_utf8();
        } else if c == '.' && !seen_dot {
            seen_dot = true;
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    // Si le dernier char est un point, le retire (ex : "0.92." → "0.92").
    let candidate = after[..end].trim_end_matches('.');
    candidate.parse::<f32>().ok().map(|v| v.clamp(0.0, 1.0))
}

fn short_llm_id(id: &str) -> String {
    let l = id.to_lowercase();
    if l.contains("claude") {
        "claude".into()
    } else if l.contains("gpt") || l.contains("openai") {
        "gpt".into()
    } else if l.contains("gemini") {
        "gemini".into()
    } else if l.contains("draft") {
        "drafter".into()
    } else {
        "unknown".into()
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_direct_passthrough() {
        let r = CanonicalReformulator::new();
        let json = serde_json::json!({
            "plan_id": "01900000-0000-7000-8000-000000000000",
            "title": "Test",
            "steps": [{"id": 1, "action": "do it"}],
            "rationale": ""
        });
        let plan = r.to_plan(&json.to_string(), SourceDialect::Gpt4Json).unwrap();
        assert_eq!(plan.title, "Test");
        assert_eq!(plan.steps.len(), 1);
    }

    #[test]
    fn xml_plan_extraction() {
        let r = CanonicalReformulator::new();
        let xml = "<plan>\n<step>1. Setup tokio</step>\n<step>2. Build axum</step>\n</plan>";
        let plan = r.to_plan(xml, SourceDialect::ClaudeXml).unwrap();
        assert!(plan.steps.len() >= 1);
    }

    #[test]
    fn markdown_plan_numbered() {
        let r = CanonicalReformulator::new();
        let md = "Here's the plan:\n\n1. Setup the environment\n2. Build the API\n3. Test\n";
        let plan = r.to_plan(md, SourceDialect::GeminiMarkdown).unwrap();
        assert_eq!(plan.steps.len(), 3);
        assert!(plan.steps[0].action.contains("Setup"));
    }

    #[test]
    fn markdown_code_block_extraction() {
        let r = CanonicalReformulator::new();
        let md = "Here's the code:\n\n```rust\nfn main() { println!(\"hi\"); }\n```\n";
        let code = r.to_code(md, SourceDialect::GeminiMarkdown).unwrap();
        assert_eq!(code.language, CodeLanguage::Rust);
        assert!(code.source.contains("fn main"));
    }

    #[test]
    fn json_code_passthrough() {
        let r = CanonicalReformulator::new();
        let json = serde_json::json!({"language": "python", "source": "print('hi')"});
        let code = r.to_code(&json.to_string(), SourceDialect::Gpt4Json).unwrap();
        assert_eq!(code.language, CodeLanguage::Python);
    }

    #[test]
    fn claim_xml_extraction() {
        let r = CanonicalReformulator::new();
        let xml = "<claim>Paris is the capital of France.</claim>\n<source>WP</source>";
        let claim = r.to_claim(xml, SourceDialect::ClaudeXml).unwrap();
        assert!(claim.claim.contains("Paris"));
        assert_eq!(claim.evidence.len(), 1);
    }

    #[test]
    fn claim_confidence_extraction() {
        let r = CanonicalReformulator::new();
        let txt = "The answer is yes, with confidence: 0.95";
        let claim = r.to_claim(txt, SourceDialect::Gpt4Json).unwrap();
        assert!((claim.confidence - 0.95).abs() < 1e-3);
    }

    #[test]
    fn argument_thesis_extraction() {
        let r = CanonicalReformulator::new();
        let xml = "<thesis>Rust is safer than Go.</thesis>\n- Memory safety\n- No GC pauses";
        let a = r
            .to_argument(xml, ArgumentStance::Pro, SourceDialect::ClaudeXml)
            .unwrap();
        assert!(a.thesis.contains("Rust"));
        assert_eq!(a.stance, ArgumentStance::Pro);
        assert!(a.reasoning.len() >= 2);
    }

    #[test]
    fn synthesis_with_md_sections() {
        let r = CanonicalReformulator::new();
        let md = "## Overview\nMain summary.\n\n## Details\nDeep dive.\n";
        let s = r
            .to_synthesis(md, SourceDialect::GeminiMarkdown, "gemini_2_5")
            .unwrap();
        assert_eq!(s.sections.len(), 2);
        assert_eq!(s.citations.len(), 1);
        assert_eq!(s.citations[0].llm_source, "gemini");
    }

    #[test]
    fn route_dispatches_correctly() {
        let r = CanonicalReformulator::new();
        let v = r
            .route(
                "Some content",
                SourceDialect::ClaudeXml,
                CanonicalSchemaId::ClaimV1,
                "claude_opus_4_6",
            )
            .unwrap();
        let claim: CanonicalClaim = serde_json::from_value(v).unwrap();
        assert!(!claim.claim.is_empty());
    }

    #[test]
    fn source_dialect_from_provider_id() {
        assert_eq!(SourceDialect::from_provider_id("claude_opus_4_6"), SourceDialect::ClaudeXml);
        assert_eq!(SourceDialect::from_provider_id("gpt_5"), SourceDialect::Gpt4Json);
        assert_eq!(SourceDialect::from_provider_id("gemini_2_5_pro"), SourceDialect::GeminiMarkdown);
        assert_eq!(SourceDialect::from_provider_id("unknown_llm"), SourceDialect::Unknown);
    }

    #[test]
    fn short_llm_id_works() {
        assert_eq!(short_llm_id("claude_opus_4_6"), "claude");
        assert_eq!(short_llm_id("GPT-5"), "gpt");
        assert_eq!(short_llm_id("Gemini-2.5-Pro"), "gemini");
        assert_eq!(short_llm_id("local_drafter_7b"), "drafter");
    }

    #[test]
    fn truncate_respects_max() {
        let s = "a".repeat(100);
        assert_eq!(truncate(&s, 50).len(), 50);
        assert_eq!(truncate(&s, 200).len(), 100);
    }
}
