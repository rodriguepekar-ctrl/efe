// SPDX-License-Identifier: UNLICENSED
//! DTOs d'entrée du bridge — couche anti-corruption mince.
//!
//! Le front n'envoie PAS un [`efe_coordinator::MissionARequest`] brut (qui
//! expose des champs internes : `task_class_scoring_table`, `tenant_id`, …). Il
//! envoie un DTO minimal qu'on mappe ici vers la requête moteur.

use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;

use efe_coordinator::{CandidateSourceRef, MissionARequest, MissionBRequest, SpeculativeRequest};
use efe_nano_app_coordinator::types::router::TaskClass;
use efe_nano_app_coordinator::types::{Budget, LlmMetadata};

/// Corps de `POST /v1/missions/a`.
#[derive(Debug, Clone, Deserialize)]
pub struct MissionADto {
    /// Le texte utilisateur (obligatoire, non vide après trim).
    pub user_text: String,
    /// Catégorie (`"FactualQa"`, `"Code"`, …). Défaut `FactualQa`.
    #[serde(default)]
    pub task_class: Option<String>,
    /// LLMs candidats. Défaut : `config.default_llms`.
    #[serde(default)]
    pub available_llms: Option<Vec<String>>,
    /// **P5.5** — Documents de référence contre lesquels le verifier fact-checke
    /// la réponse du LLM (`claim = réponse`). Optionnel ; absent → verifier
    /// dormant (comportement historique). Présent + verifier vivant
    /// (`EFE_LIVE_SLM=1`, `verifier ✓`) → événements `VERDICT_READY` dans le SSE.
    #[serde(default)]
    pub candidate_sources: Option<Vec<CandidateSourceRef>>,
}

impl MissionADto {
    /// Mappe le DTO → [`MissionARequest`].
    ///
    /// Validation : `user_text` non vide, `task_class` parseable, au moins 1
    /// LLM (du DTO ou des défauts). Les métadonnées LLM sont des placeholders
    /// de preview (le coût RÉEL affiché vient de la réponse provider, pas de
    /// ces métadonnées — cf. architecture §6).
    ///
    /// # Errors
    /// Retourne un message si `user_text` est vide, `task_class` invalide, ou
    /// aucun LLM disponible.
    pub fn into_request(self, default_llms: &[String]) -> Result<MissionARequest, String> {
        let user_text = self.user_text.trim().to_string();
        if user_text.is_empty() {
            return Err("user_text vide".to_string());
        }
        let task_class = parse_task_class(self.task_class.as_deref().unwrap_or("FactualQa"))?;

        let llms = self
            .available_llms
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| default_llms.to_vec());
        if llms.is_empty() {
            return Err("aucun LLM disponible (DTO vide + aucun défaut configuré)".to_string());
        }

        let mut llm_metadata = HashMap::new();
        for id in &llms {
            llm_metadata.insert(id.clone(), preview_meta());
        }

        Ok(MissionARequest {
            user_text,
            task_class,
            available_llms: llms,
            llm_metadata,
            budget: Budget {
                max_cost_cents: 0,
                max_cost_microcents: 1_000_000,
                max_latency_ms: 30_000,
                min_quality_score: 0.0,
            },
            candidate_sources: validate_candidate_sources(self.candidate_sources)?,
            max_total_latency: Duration::from_secs(30),
            task_class_scoring_table: None,
            tenant_id: None,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Mission B (fan-out N LLMs + vote)
// ─────────────────────────────────────────────────────────────────────────

/// Corps de `POST /v1/missions/b`.
#[derive(Debug, Clone, Deserialize)]
pub struct MissionBDto {
    /// Texte utilisateur (obligatoire).
    pub user_text: String,
    /// Catégorie (défaut `FactualQa`).
    #[serde(default)]
    pub task_class: Option<String>,
    /// LLMs candidats (≥ 2). Défaut : `config.default_llms`.
    #[serde(default)]
    pub llms: Option<Vec<String>>,
    /// Poids de vote par LLM (défaut : uniformes).
    #[serde(default)]
    pub vote_weights: Option<std::collections::HashMap<String, f32>>,
    /// Quorum requis (défaut : `ceil(|llms| / 2)`).
    #[serde(default)]
    pub quorum: Option<usize>,
    /// **P5.5** — Sources de référence pour le verifier (cf. [`MissionADto`]).
    /// Mêmes règles de validation. Absent → verifier dormant.
    #[serde(default)]
    pub candidate_sources: Option<Vec<CandidateSourceRef>>,
}

impl MissionBDto {
    /// Mappe le DTO → [`MissionBRequest`].
    ///
    /// # Errors
    /// `user_text` vide, `task_class` invalide, ou < 2 LLMs.
    pub fn into_request(self, default_llms: &[String]) -> Result<MissionBRequest, String> {
        let user_text = self.user_text.trim().to_string();
        if user_text.is_empty() {
            return Err("user_text vide".to_string());
        }
        let task_class = parse_task_class(self.task_class.as_deref().unwrap_or("FactualQa"))?;
        let llms = self
            .llms
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| default_llms.to_vec());
        if llms.len() < 2 {
            return Err(format!("Mission B requiert ≥ 2 LLMs (reçu {})", llms.len()));
        }
        let mut llm_metadata = HashMap::new();
        for id in &llms {
            llm_metadata.insert(id.clone(), preview_meta());
        }
        Ok(MissionBRequest {
            user_text,
            task_class,
            llms,
            llm_metadata,
            vote_weights: self.vote_weights.unwrap_or_default(),
            quorum: self.quorum,
            candidate_sources: validate_candidate_sources(self.candidate_sources)?,
            max_total_latency: Duration::from_secs(30),
            tenant_id: None,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Mission Speculative (drafter cheap + verifier premium)
// ─────────────────────────────────────────────────────────────────────────

/// Corps de `POST /v1/missions/speculative`.
#[derive(Debug, Clone, Deserialize)]
pub struct SpecDto {
    /// Texte utilisateur (obligatoire).
    pub user_text: String,
    /// Catégorie (défaut `FactualQa`).
    #[serde(default)]
    pub task_class: Option<String>,
    /// LLM drafter (cheap, e.g. `"mock-gemini"`).
    pub drafter_llm: String,
    /// LLM verifier (premium, e.g. `"mock-claude"`).
    pub verifier_llm: String,
}

impl SpecDto {
    /// Mappe le DTO → [`SpeculativeRequest`] (via le constructeur moteur qui
    /// dérive `verifier_metadata` de la table S6 par défaut).
    ///
    /// # Errors
    /// `user_text` vide, `task_class` invalide, drafter/verifier vides.
    pub fn into_request(self) -> Result<SpeculativeRequest, String> {
        let user_text = self.user_text.trim().to_string();
        if user_text.is_empty() {
            return Err("user_text vide".to_string());
        }
        if self.drafter_llm.trim().is_empty() {
            return Err("drafter_llm vide".to_string());
        }
        if self.verifier_llm.trim().is_empty() {
            return Err("verifier_llm vide".to_string());
        }
        let task_class = parse_task_class(self.task_class.as_deref().unwrap_or("FactualQa"))?;
        Ok(SpeculativeRequest::with_drafter_and_verifier(
            user_text,
            task_class,
            self.drafter_llm,
            self.verifier_llm,
        ))
    }
}

/// Métadonnées LLM placeholder pour le routing en preview.
fn preview_meta() -> LlmMetadata {
    LlmMetadata {
        cost_input_per_million: 0,
        cost_output_per_million: 0,
        cost_input_per_million_microcents: 1_000,
        cost_output_per_million_microcents: 1_000,
        p50_latency_ms: 200,
        p99_latency_ms: 800,
        success_rate: 1.0,
        capability_scores: HashMap::new(),
        circuit_open: false,
    }
}

/// Parse une string `task_class` (accepte CamelCase et SCREAMING_SNAKE_CASE).
fn parse_task_class(s: &str) -> Result<TaskClass, String> {
    let upper = match s {
        "FactualQa" | "FACTUAL_QA" => "FACTUAL_QA",
        "Code" | "CODE" => "CODE",
        "Creative" | "CREATIVE" => "CREATIVE",
        "Analysis" | "ANALYSIS" => "ANALYSIS",
        "Reasoning" | "REASONING" => "REASONING",
        "Translation" | "TRANSLATION" => "TRANSLATION",
        other => other,
    };
    serde_json::from_value::<TaskClass>(serde_json::json!(upper))
        .map_err(|_| format!("task_class invalide: {s}"))
}

// ─────────────────────────────────────────────────────────────────────────
// P5.5 — Sources candidates (active le verifier + Verdicts)
// ─────────────────────────────────────────────────────────────────────────

/// Plafond du nombre de sources candidates — borne le fan de fact-check (coût
/// verifier + taille du payload).
const MAX_CANDIDATE_SOURCES: usize = 32;
/// Plafond de la taille (en caractères) d'une source candidate.
const MAX_SOURCE_TEXT_CHARS: usize = 4_000;

/// **P5.5** — Valide + normalise les sources candidates du DTO vers le champ
/// moteur `candidate_sources` (consommé par le verifier dans `mission_a`/`b`).
///
/// - `None` ou `Some([])` → `[]` : verifier dormant (comportement historique),
///   ce n'est **pas** une erreur.
/// - Sinon : `id`+`text` sont trimés ; les entrées dont `id` OU `text` est vide
///   sont **droppées**. Si la liste fournie était non vide mais que TOUT est
///   droppé → erreur (on surface la faute plutôt que de désactiver
///   silencieusement la vérification). Plafonds `MAX_CANDIDATE_SOURCES` /
///   `MAX_SOURCE_TEXT_CHARS` → erreur.
///
/// # Errors
/// Liste non vide mais entièrement malformée, trop de sources, ou source dont le
/// texte dépasse `MAX_SOURCE_TEXT_CHARS`.
fn validate_candidate_sources(
    raw: Option<Vec<CandidateSourceRef>>,
) -> Result<Vec<CandidateSourceRef>, String> {
    let Some(list) = raw else {
        return Ok(vec![]);
    };
    if list.is_empty() {
        return Ok(vec![]);
    }
    let provided = list.len();

    let kept: Vec<CandidateSourceRef> = list
        .into_iter()
        .filter_map(|cs| {
            let id = cs.id.trim();
            let text = cs.text.trim();
            if id.is_empty() || text.is_empty() {
                None
            } else {
                Some(CandidateSourceRef {
                    id: id.to_string(),
                    text: text.to_string(),
                })
            }
        })
        .collect();

    if kept.is_empty() {
        return Err(format!(
            "candidate_sources fournies ({provided}) mais toutes invalides (id+text requis)"
        ));
    }
    if kept.len() > MAX_CANDIDATE_SOURCES {
        return Err(format!(
            "trop de candidate_sources ({} > max {MAX_CANDIDATE_SOURCES})",
            kept.len()
        ));
    }
    if let Some(too_long) = kept
        .iter()
        .find(|cs| cs.text.chars().count() > MAX_SOURCE_TEXT_CHARS)
    {
        return Err(format!(
            "source '{}' trop longue ({} > max {MAX_SOURCE_TEXT_CHARS} caractères)",
            too_long.id,
            too_long.text.chars().count()
        ));
    }

    Ok(kept)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_user_text_rejected() {
        let dto = MissionADto {
            user_text: "   ".into(),
            task_class: None,
            available_llms: None,
            candidate_sources: None,
        };
        assert!(dto.into_request(&["mock".into()]).is_err());
    }

    #[test]
    fn defaults_to_factual_qa_and_default_llms() {
        let dto = MissionADto {
            user_text: "Capitale ?".into(),
            task_class: None,
            available_llms: None,
            candidate_sources: None,
        };
        let req = dto.into_request(&["mock".into()]).expect("ok");
        assert_eq!(req.task_class, TaskClass::FactualQa);
        assert_eq!(req.available_llms, vec!["mock".to_string()]);
        assert!(req.llm_metadata.contains_key("mock"));
    }

    #[test]
    fn parses_screaming_and_camel_task_class() {
        assert_eq!(parse_task_class("Code").unwrap(), TaskClass::Code);
        assert_eq!(parse_task_class("CODE").unwrap(), TaskClass::Code);
        assert!(parse_task_class("Nonsense").is_err());
    }

    #[test]
    fn mission_b_requires_two_llms() {
        let one = MissionBDto {
            user_text: "x".into(),
            task_class: None,
            llms: Some(vec!["a".into()]),
            vote_weights: None,
            quorum: None,
            candidate_sources: None,
        };
        assert!(one.into_request(&[]).is_err(), "1 LLM → refus");

        let two = MissionBDto {
            user_text: "x".into(),
            task_class: None,
            llms: Some(vec!["a".into(), "b".into()]),
            vote_weights: None,
            quorum: None,
            candidate_sources: None,
        };
        let req = two.into_request(&[]).expect("2 LLMs ok");
        assert_eq!(req.llms.len(), 2);
        assert!(req.llm_metadata.contains_key("a") && req.llm_metadata.contains_key("b"));
    }

    #[test]
    fn mission_b_defaults_to_config_llms() {
        let dto = MissionBDto {
            user_text: "x".into(),
            task_class: None,
            llms: None,
            vote_weights: None,
            quorum: None,
            candidate_sources: None,
        };
        let defaults = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let req = dto.into_request(&defaults).expect("défauts ok");
        assert_eq!(req.llms, defaults);
    }

    #[test]
    fn speculative_validates_drafter_verifier() {
        let bad = SpecDto {
            user_text: "x".into(),
            task_class: None,
            drafter_llm: "  ".into(),
            verifier_llm: "v".into(),
        };
        assert!(bad.into_request().is_err(), "drafter vide → refus");

        let ok = SpecDto {
            user_text: "Capitale ?".into(),
            task_class: Some("FactualQa".into()),
            drafter_llm: "mock-gemini".into(),
            verifier_llm: "mock-claude".into(),
        };
        let req = ok.into_request().expect("ok");
        assert_eq!(req.drafter_llm, "mock-gemini");
        assert_eq!(req.verifier_llm, "mock-claude");
    }

    // ── P5.5 — sources candidates (active le verifier) ─────────────────────

    /// Petit constructeur de `CandidateSourceRef` pour les tests.
    fn cs(id: &str, text: &str) -> CandidateSourceRef {
        CandidateSourceRef {
            id: id.to_string(),
            text: text.to_string(),
        }
    }

    /// `None` et `Some([])` → liste vide, sans erreur (verifier dormant).
    #[test]
    fn csources_none_and_empty_are_empty() {
        assert!(validate_candidate_sources(None).unwrap().is_empty());
        assert!(validate_candidate_sources(Some(vec![])).unwrap().is_empty());
    }

    /// Sources valides → préservées (id+text), dans l'ordre.
    #[test]
    fn csources_maps_valid() {
        let out = validate_candidate_sources(Some(vec![cs("a", "ta"), cs("b", "tb")])).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].id, "a");
        assert_eq!(out[0].text, "ta");
        assert_eq!(out[1].id, "b");
    }

    /// id/text trimés ; entrées à id OU text vide droppées.
    #[test]
    fn csources_trims_and_drops_malformed() {
        let out = validate_candidate_sources(Some(vec![
            cs("  x  ", "  texte  "), // valide → trimé
            cs("", "t"),              // id vide → drop
            cs("id", "   "),          // text blanc → drop
            cs("  ", "  "),           // tout blanc → drop
        ]))
        .unwrap();
        assert_eq!(out.len(), 1, "seule la 1re survit");
        assert_eq!(out[0].id, "x", "id trimé");
        assert_eq!(out[0].text, "texte", "text trimé");
    }

    /// Liste non vide mais entièrement malformée → erreur (on surface la faute).
    #[test]
    fn csources_all_malformed_is_error() {
        let err = validate_candidate_sources(Some(vec![cs("", ""), cs("  ", "  ")]));
        assert!(err.is_err(), "tout malformé → Err");
    }

    /// Plafond du nombre de sources : MAX ok, MAX+1 rejeté.
    #[test]
    fn csources_count_cap_boundary() {
        let at_cap: Vec<_> = (0..MAX_CANDIDATE_SOURCES)
            .map(|i| cs(&format!("id{i}"), "t"))
            .collect();
        assert_eq!(
            validate_candidate_sources(Some(at_cap)).unwrap().len(),
            MAX_CANDIDATE_SOURCES,
            "exactement MAX → ok"
        );

        let over_cap: Vec<_> = (0..=MAX_CANDIDATE_SOURCES)
            .map(|i| cs(&format!("id{i}"), "t"))
            .collect();
        assert!(
            validate_candidate_sources(Some(over_cap)).is_err(),
            "MAX+1 → Err"
        );
    }

    /// Plafond de taille d'une source : MAX chars ok, MAX+1 rejeté.
    #[test]
    fn csources_text_length_cap_boundary() {
        let at_max = "x".repeat(MAX_SOURCE_TEXT_CHARS);
        assert!(
            validate_candidate_sources(Some(vec![cs("big", &at_max)])).is_ok(),
            "exactement MAX caractères → ok"
        );
        let over_max = "x".repeat(MAX_SOURCE_TEXT_CHARS + 1);
        assert!(
            validate_candidate_sources(Some(vec![cs("big", &over_max)])).is_err(),
            "MAX+1 caractères → Err"
        );
    }

    /// Intégration : `MissionADto` porte les sources jusqu'à `MissionARequest`.
    #[test]
    fn mission_a_into_request_carries_sources() {
        let dto = MissionADto {
            user_text: "Capitale ?".into(),
            task_class: None,
            available_llms: None,
            candidate_sources: Some(vec![
                cs("wiki", "Paris est la capitale."),
                cs("doc", "Capitale : Paris."),
            ]),
        };
        let req = dto.into_request(&["mock".into()]).expect("ok");
        assert_eq!(req.candidate_sources.len(), 2, "2 sources mappées");
        assert_eq!(req.candidate_sources[0].id, "wiki");
    }

    /// Intégration : pas de sources → `candidate_sources` vide (verifier dormant).
    #[test]
    fn mission_a_into_request_no_sources_empty() {
        let dto = MissionADto {
            user_text: "Capitale ?".into(),
            task_class: None,
            available_llms: None,
            candidate_sources: None,
        };
        let req = dto.into_request(&["mock".into()]).expect("ok");
        assert!(req.candidate_sources.is_empty(), "absent → vide");
    }

    /// Intégration : sources toutes invalides → `into_request` propage l'erreur
    /// (→ 400 côté handler). Verrouille le `?` sur `validate_candidate_sources`.
    #[test]
    fn mission_a_into_request_rejects_all_malformed_sources() {
        let dto = MissionADto {
            user_text: "Capitale ?".into(),
            task_class: None,
            available_llms: None,
            candidate_sources: Some(vec![cs("", "")]),
        };
        assert!(
            dto.into_request(&["mock".into()]).is_err(),
            "sources toutes invalides → erreur (400)"
        );
    }

    /// Symétrie : `MissionBDto` porte aussi les sources jusqu'à `MissionBRequest`.
    #[test]
    fn mission_b_into_request_carries_sources() {
        let dto = MissionBDto {
            user_text: "x".into(),
            task_class: None,
            llms: Some(vec!["a".into(), "b".into()]),
            vote_weights: None,
            quorum: None,
            candidate_sources: Some(vec![cs("s1", "ref un"), cs("s2", "ref deux")]),
        };
        let req = dto.into_request(&[]).expect("ok");
        assert_eq!(req.candidate_sources.len(), 2, "B : 2 sources mappées");
    }

    // ── P5.5 — vérification : le CONTRAT WIRE (serde JSON) ─────────────────
    // Tous les tests ci-dessus construisent le DTO par littéral → court-circuitent
    // serde. Or P5.5 est d'abord un contrat JSON front→bridge. Ces tests
    // verrouillent la désérialisation réelle (`#[serde(default)]` + le couple
    // {id,text} de `CandidateSourceRef`), seule chose que les littéraux n'exercent
    // pas — et que `bridge.rs` ne couvre pas non plus.

    /// Payload SANS `candidate_sources` (front historique) → désérialise toujours
    /// (`#[serde(default)]` → `None`). Verrouille la backward-compat du DTO.
    #[test]
    fn dto_deserializes_without_candidate_sources() {
        let dto: MissionADto = serde_json::from_str(r#"{"user_text":"x"}"#).expect("parse");
        assert!(dto.candidate_sources.is_none(), "champ absent → None");
    }

    /// Payload AVEC `candidate_sources` → désérialise vers `Vec<CandidateSourceRef>`
    /// (contrat wire : objets `{id, text}`, ordre préservé).
    #[test]
    fn dto_deserializes_with_candidate_sources() {
        let json = r#"{"user_text":"x","candidate_sources":[{"id":"a","text":"ta"},{"id":"b","text":"tb"}]}"#;
        let dto: MissionADto = serde_json::from_str(json).expect("parse");
        let srcs = dto.candidate_sources.expect("some");
        assert_eq!(srcs.len(), 2);
        assert_eq!(srcs[0].id, "a");
        assert_eq!(srcs[1].text, "tb");
    }

    /// Chemin WIRE complet : JSON front → `from_str` → `into_request` → champ
    /// moteur `candidate_sources`. Verrouille la chaîne de bout en bout.
    #[test]
    fn dto_json_to_request_carries_sources() {
        let json = r#"{"user_text":"Capitale ?","candidate_sources":[{"id":"wiki","text":"Paris."}]}"#;
        let dto: MissionADto = serde_json::from_str(json).expect("parse");
        let req = dto.into_request(&["mock".into()]).expect("ok");
        assert_eq!(req.candidate_sources.len(), 1);
        assert_eq!(req.candidate_sources[0].id, "wiki");
    }

    /// Au niveau WIRE, `text` est REQUIS : une source sans `text` échoue à la
    /// désérialisation (→ 400 axum). Distinct du 400 « text vide » produit par
    /// `validate_candidate_sources` — borne les deux chemins d'erreur.
    #[test]
    fn dto_source_missing_text_is_deserialize_error() {
        let json = r#"{"user_text":"x","candidate_sources":[{"id":"a"}]}"#;
        let parsed: Result<MissionADto, _> = serde_json::from_str(json);
        assert!(parsed.is_err(), "source sans 'text' → erreur de parse (champ requis)");
    }
}
