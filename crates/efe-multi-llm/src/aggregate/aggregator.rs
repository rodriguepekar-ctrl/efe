// SPDX-License-Identifier: UNLICENSED
//! Implémentation `Aggregator`.

use crate::canonical::{
    CanonicalSynthesis, SynthesisCitation, SynthesisSection,
};
use crate::error::{MultiLlmError, MultiLlmResult};

/// Mode de l'Aggregator.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregatorMode {
    /// Concaténation déterministe ordonnée.
    Concatenate,
    /// SLM (Llama-3.2-1B avec grammar) — Phase 3 prod.
    LlmBacked,
}

/// Aggregator — combine outputs canoniques en `CanonicalSynthesis`.
#[derive(Clone, Debug)]
pub struct Aggregator {
    mode: AggregatorMode,
}

impl Default for Aggregator {
    fn default() -> Self {
        Self {
            mode: AggregatorMode::Concatenate,
        }
    }
}

/// Entrée pour l'Aggregator : une contribution d'un LLM.
#[derive(Debug, Clone)]
pub struct AggregateInput {
    /// LLM source (`claude`/`gpt`/`gemini`).
    pub llm_source: String,
    /// Titre de section (souvent = nom de la sub-task).
    pub heading: String,
    /// Texte canonique (typiquement summary d'une CanonicalSynthesis).
    pub body: String,
}

impl Aggregator {
    /// Construit avec mode.
    #[must_use]
    pub fn new(mode: AggregatorMode) -> Self {
        Self { mode }
    }

    /// Mode actuel.
    #[must_use]
    pub fn mode(&self) -> AggregatorMode {
        self.mode
    }

    /// Agrège N inputs en une `CanonicalSynthesis`.
    ///
    /// Le résultat contient :
    /// - `summary` : résumé fait à partir du body[0] (premier input).
    /// - `citations` : une `SynthesisCitation` par input.
    /// - `sections` : une `SynthesisSection` par input (heading + body).
    /// - `uncertainty` : note si les inputs divergent fortement (Jaccard < 0.3).
    pub fn aggregate(
        &self,
        inputs: &[AggregateInput],
    ) -> MultiLlmResult<CanonicalSynthesis> {
        if inputs.is_empty() {
            return Err(MultiLlmError::Config(
                "aggregator requires at least 1 input".into(),
            ));
        }
        let citations: Vec<SynthesisCitation> = inputs
            .iter()
            .map(|i| SynthesisCitation {
                llm_source: i.llm_source.clone(),
                snippet: truncate(&i.body, 500),
            })
            .collect();
        let sections: Vec<SynthesisSection> = inputs
            .iter()
            .map(|i| SynthesisSection {
                heading: truncate(&i.heading, 200),
                body: truncate(&i.body, 5000),
            })
            .collect();
        let summary = if inputs.len() == 1 {
            truncate(&inputs[0].body, 5000)
        } else {
            // Concaténation des headings : "Translation, Explanation, Haiku".
            let combined_headings = inputs
                .iter()
                .map(|i| i.heading.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "Combined multi-LLM response with {} sub-results: {combined_headings}.",
                inputs.len()
            )
        };
        // Détecte divergence inter-LLM via Jaccard mot-à-mot.
        let uncertainty = if inputs.len() >= 2 {
            let avg_jaccard = average_pairwise_jaccard(inputs);
            if avg_jaccard < 0.3 {
                format!("LLMs diverge significantly (avg Jaccard {avg_jaccard:.2}).")
            } else {
                String::new()
            }
        } else {
            String::new()
        };
        Ok(CanonicalSynthesis {
            summary,
            citations,
            uncertainty,
            sections,
        })
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect()
    }
}

fn jaccard(a: &str, b: &str) -> f32 {
    let sa: std::collections::HashSet<&str> = a.split_whitespace().collect();
    let sb: std::collections::HashSet<&str> = b.split_whitespace().collect();
    if sa.is_empty() && sb.is_empty() {
        return 1.0;
    }
    let inter = sa.intersection(&sb).count();
    let union = sa.union(&sb).count().max(1);
    inter as f32 / union as f32
}

fn average_pairwise_jaccard(inputs: &[AggregateInput]) -> f32 {
    let n = inputs.len();
    if n < 2 {
        return 1.0;
    }
    let mut sum = 0.0_f32;
    let mut pairs = 0_u32;
    for i in 0..n {
        for j in (i + 1)..n {
            sum += jaccard(&inputs[i].body, &inputs[j].body);
            pairs += 1;
        }
    }
    if pairs == 0 {
        return 1.0;
    }
    sum / pairs as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_input_works() {
        let a = Aggregator::default();
        let out = a
            .aggregate(&[AggregateInput {
                llm_source: "claude".into(),
                heading: "Test".into(),
                body: "Hello world.".into(),
            }])
            .unwrap();
        assert_eq!(out.summary, "Hello world.");
        assert_eq!(out.sections.len(), 1);
    }

    #[test]
    fn multiple_inputs_combined() {
        let a = Aggregator::default();
        let out = a
            .aggregate(&[
                AggregateInput {
                    llm_source: "claude".into(),
                    heading: "Translation".into(),
                    body: "fn main() {}".into(),
                },
                AggregateInput {
                    llm_source: "gemini".into(),
                    heading: "Explanation".into(),
                    body: "It's a Rust main function.".into(),
                },
                AggregateInput {
                    llm_source: "gpt".into(),
                    heading: "Haiku".into(),
                    body: "Code flows like water.".into(),
                },
            ])
            .unwrap();
        assert_eq!(out.sections.len(), 3);
        assert_eq!(out.citations.len(), 3);
        assert!(out.summary.contains("3 sub-results"));
    }

    #[test]
    fn divergent_inputs_get_uncertainty_note() {
        let a = Aggregator::default();
        let out = a
            .aggregate(&[
                AggregateInput {
                    llm_source: "a".into(),
                    heading: "X".into(),
                    body: "alpha beta gamma".into(),
                },
                AggregateInput {
                    llm_source: "b".into(),
                    heading: "Y".into(),
                    body: "delta epsilon zeta".into(),
                },
            ])
            .unwrap();
        // Aucun mot commun → Jaccard = 0 → uncertainty notée.
        assert!(out.uncertainty.contains("diverge"));
    }

    #[test]
    fn similar_inputs_no_uncertainty() {
        let a = Aggregator::default();
        let out = a
            .aggregate(&[
                AggregateInput {
                    llm_source: "a".into(),
                    heading: "X".into(),
                    body: "the answer is 42".into(),
                },
                AggregateInput {
                    llm_source: "b".into(),
                    heading: "Y".into(),
                    body: "the answer is 42".into(),
                },
            ])
            .unwrap();
        assert!(out.uncertainty.is_empty());
    }

    #[test]
    fn empty_inputs_errors() {
        let a = Aggregator::default();
        let r = a.aggregate(&[]);
        assert!(matches!(r, Err(MultiLlmError::Config(_))));
    }

    #[test]
    fn citations_preserve_llm_source() {
        let a = Aggregator::default();
        let out = a
            .aggregate(&[AggregateInput {
                llm_source: "gemini_2_5".into(),
                heading: "h".into(),
                body: "b".into(),
            }])
            .unwrap();
        assert_eq!(out.citations[0].llm_source, "gemini_2_5");
    }

    #[test]
    fn modes_set_correctly() {
        let a = Aggregator::new(AggregatorMode::LlmBacked);
        assert_eq!(a.mode(), AggregatorMode::LlmBacked);
    }
}
