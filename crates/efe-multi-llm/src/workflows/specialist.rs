// SPDX-License-Identifier: UNLICENSED
//! Workflow Specialist — Decompose + Route per sub-task + Aggregate.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use efe_coordinator::{Coordinator, LlmCall};
use efe_nano_app_coordinator::types::router::TaskClass;
use uuid::Uuid;

use crate::aggregate::{Aggregator, aggregator::AggregateInput};
use crate::budget::BudgetTracker;
use crate::decompose::Decomposer;
use crate::error::{MultiLlmError, MultiLlmResult};
use crate::events_ext::{Phase3EventExt, Phase3Publisher};
use crate::request::{SpecialistAssignment, SpecialistRequest, SpecialistResponse};
use crate::ANTI_MONOCULTURE_MAX_RATIO;

/// Exécute Specialist : decompose → route → aggregate.
pub async fn run_specialist(
    coord: &Coordinator,
    req: SpecialistRequest,
) -> MultiLlmResult<SpecialistResponse> {
    let started = Instant::now();
    let swarm_id = Uuid::now_v7();
    let publisher = Phase3Publisher;
    let budget = BudgetTracker::new(req.max_budget_cents);
    let bus = Arc::clone(&coord.bus);

    // ── Étape 1 : Decompose ──────────────────────────────────────────
    let decomposer = Decomposer::default();
    let sub_tasks = decomposer.decompose(&req.user_text)?;

    // ── Étape 2 : Route per sub-task ──────────────────────────────────
    let mut assignments: Vec<SpecialistAssignment> = Vec::new();
    let mut routing: Vec<(usize, String)> = Vec::new(); // (sub_task_idx, llm)

    for st in &sub_tasks {
        let llm = choose_best_llm(
            st.task_class,
            &req.available_llms,
            &req.capability_scores,
            &count_assignments_so_far(&routing),
        )?;
        routing.push((st.id as usize - 1, llm.clone()));
    }

    // Vérifie l'anti-monoculture : si un LLM représente > 60% des
    // sub-tasks, on re-route les excédentaires vers le 2nd-best.
    enforce_anti_monoculture(
        &mut routing,
        &sub_tasks,
        &req.available_llms,
        &req.capability_scores,
    )?;

    // ── Étape 3 : Appels parallèles ───────────────────────────────────
    let mut futs = Vec::new();
    for (idx, llm) in &routing {
        let st = sub_tasks[*idx].clone();
        let llm = llm.clone();
        let coord = coord.clone();
        let deadline = req.max_total_latency.saturating_sub(started.elapsed());
        let swarm = swarm_id;
        let publisher_clone = Phase3Publisher;
        let bus_clone = Arc::clone(&bus);
        let st_id = st.id;
        let llm_for_event = llm.clone();
        futs.push(async move {
            publisher_clone
                .publish_sub_task_started(&bus_clone, swarm, st_id, &llm_for_event)
                .await;
            let call = LlmCall {
                prompt: st.task.clone(),
                max_tokens: 600,
                temperature: 0.5,
                deadline: Some(deadline),
                trace_tag: Some(format!("phase3:specialist:{swarm}:{}", st.id)),
            };
            let resp = coord.bridge.call_one(&llm, call).await;
            (st, llm, resp)
        });
    }
    let results = futures::future::join_all(futs).await;

    // Récolte + budget charging.
    let mut agg_inputs: Vec<AggregateInput> = Vec::new();
    for (st, llm, res) in results {
        match res {
            Ok(resp) => {
                let _ = budget.try_charge(&llm, resp.cost_cents);
                publisher
                    .publish_budget_charged(
                        &bus,
                        swarm_id,
                        &llm,
                        resp.cost_cents,
                        budget.spent_cents(),
                        budget.max_cost_cents(),
                    )
                    .await;
                assignments.push(SpecialistAssignment {
                    sub_task_id: st.id,
                    task: st.task.clone(),
                    task_class: st.task_class,
                    assigned_llm: llm.clone(),
                    output_text: resp.text.clone(),
                });
                agg_inputs.push(AggregateInput {
                    llm_source: llm,
                    heading: format!("Sub-task {} ({:?})", st.id, st.task_class),
                    body: resp.text,
                });
            }
            Err(_) => {
                // Marque l'assignment avec output vide (chaos resilient).
                assignments.push(SpecialistAssignment {
                    sub_task_id: st.id,
                    task: st.task.clone(),
                    task_class: st.task_class,
                    assigned_llm: llm,
                    output_text: String::new(),
                });
            }
        }
    }

    if agg_inputs.is_empty() {
        return Err(MultiLlmError::AllLlmsDown);
    }

    // ── Étape 4 : Aggregate ───────────────────────────────────────────
    let aggregator = Aggregator::default();
    let synthesis = aggregator.aggregate(&agg_inputs)?;

    // Construit le final_text = markdown des sections.
    let mut final_text = synthesis.summary.clone();
    if !synthesis.sections.is_empty() {
        final_text.push_str("\n\n");
        for sec in &synthesis.sections {
            if !sec.heading.is_empty() {
                final_text.push_str(&format!("## {}\n", sec.heading));
            }
            final_text.push_str(&sec.body);
            final_text.push_str("\n\n");
        }
    }
    if !synthesis.uncertainty.is_empty() {
        final_text.push_str(&format!("\n_Uncertainty: {}_\n", synthesis.uncertainty));
    }

    Ok(SpecialistResponse {
        final_text,
        assignments,
        total_cost_cents: budget.spent_cents(),
        latency_ms: started.elapsed().as_millis() as u64,
        swarm_id_hex: swarm_id.simple().to_string(),
    })
}

fn count_assignments_so_far(routing: &[(usize, String)]) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for (_, llm) in routing {
        *counts.entry(llm.clone()).or_insert(0) += 1;
    }
    counts
}

fn choose_best_llm(
    class: TaskClass,
    available: &[String],
    capability_scores: &HashMap<String, HashMap<String, f32>>,
    _current_counts: &HashMap<String, usize>,
) -> MultiLlmResult<String> {
    if available.is_empty() {
        return Err(MultiLlmError::Config("no LLMs available".into()));
    }
    let class_key = format!("{:?}", class).to_uppercase();
    // Le format!("{:?}") sur TaskClass donne "FactualQa" → on convertit en SCREAMING_SNAKE_CASE.
    let class_key = pascal_to_screaming_snake(&class_key);
    let mut best: Option<(String, f32)> = None;
    for llm in available {
        let score = capability_scores
            .get(llm)
            .and_then(|m| m.get(&class_key))
            .copied()
            .unwrap_or(0.5); // default neutre
        match &best {
            None => best = Some((llm.clone(), score)),
            Some((_, s)) if score > *s => best = Some((llm.clone(), score)),
            Some((current, s)) if (score - *s).abs() < 1e-6 && llm < current => {
                best = Some((llm.clone(), score));
            }
            _ => {}
        }
    }
    Ok(best.unwrap_or_else(|| (available[0].clone(), 0.0)).0)
}

fn enforce_anti_monoculture(
    routing: &mut Vec<(usize, String)>,
    sub_tasks: &[crate::decompose::SubTask],
    available: &[String],
    capability_scores: &HashMap<String, HashMap<String, f32>>,
) -> MultiLlmResult<()> {
    let total = routing.len();
    if total < 3 {
        return Ok(()); // pas pertinent pour moins de 3 sub-tasks
    }
    // Borne d'itérations : avec `n` sub-tasks on ne peut re-router au plus
    // `n` fois — au-delà on est forcément dans un cycle (cas pathologique
    // n=3 où aucune distribution ne donne ≤ 60 % : 2/3 = 0.667 reste le min).
    let max_iters = total * 2;
    let mut iter = 0;
    loop {
        iter += 1;
        let counts = count_assignments_so_far(routing);
        let (dominant, max_count) = counts
            .iter()
            .max_by_key(|e| *e.1)
            .map(|(k, v)| (k.clone(), *v))
            .unwrap_or_default();
        let ratio = max_count as f32 / total as f32;
        if ratio <= ANTI_MONOCULTURE_MAX_RATIO {
            return Ok(());
        }
        if iter > max_iters {
            // Soft-stop : on a fait de notre mieux ; le ratio résiduel est
            // intrinsèque (n=3 → impossible de descendre sous 33 %).
            return Ok(());
        }
        // Re-route une assignation du `dominant` vers son 2nd-best LLM.
        let target_idx = routing
            .iter()
            .position(|(_, llm)| llm == &dominant)
            .ok_or_else(|| MultiLlmError::Config("routing empty".into()))?;
        let st_idx = routing[target_idx].0;
        let st = &sub_tasks[st_idx];
        // Trouve le 2nd-best (en excluant le dominant).
        let alt_available: Vec<String> = available
            .iter()
            .filter(|l| **l != dominant)
            .cloned()
            .collect();
        if alt_available.is_empty() {
            return Err(MultiLlmError::MonocultureViolation {
                dominant_llm: dominant,
                ratio,
                limit: ANTI_MONOCULTURE_MAX_RATIO,
                total,
            });
        }
        let new_llm = choose_best_llm(
            st.task_class,
            &alt_available,
            capability_scores,
            &HashMap::new(),
        )?;
        // Détecte oscillation : si on s'apprête à remplacer par le même LLM
        // qui était déjà dominant (cas pathologique cyclique), on stop.
        if new_llm == dominant {
            return Ok(());
        }
        routing[target_idx].1 = new_llm;
    }
}

fn pascal_to_screaming_snake(s: &str) -> String {
    // Si déjà SCREAMING_SNAKE_CASE, retourne tel quel.
    if s.chars().all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit()) {
        return s.to_string();
    }
    let mut out = String::new();
    let chars: Vec<char> = s.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            let prev_is_lower = chars.get(i - 1).map(|p| p.is_ascii_lowercase()).unwrap_or(false);
            if prev_is_lower {
                out.push('_');
            }
        }
        out.push(c.to_ascii_uppercase());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pascal_conversion() {
        assert_eq!(pascal_to_screaming_snake("FactualQa"), "FACTUAL_QA");
        assert_eq!(pascal_to_screaming_snake("Code"), "CODE");
        assert_eq!(pascal_to_screaming_snake("Translation"), "TRANSLATION");
        assert_eq!(pascal_to_screaming_snake("CREATIVE"), "CREATIVE");
    }

    #[test]
    fn choose_best_llm_picks_max_capability() {
        let mut caps: HashMap<String, HashMap<String, f32>> = HashMap::new();
        let mut c1: HashMap<String, f32> = HashMap::new();
        c1.insert("CODE".to_string(), 0.94);
        caps.insert("gpt".to_string(), c1);
        let mut c2: HashMap<String, f32> = HashMap::new();
        c2.insert("CODE".to_string(), 0.85);
        caps.insert("claude".to_string(), c2);
        let pick = choose_best_llm(
            TaskClass::Code,
            &["gpt".into(), "claude".into()],
            &caps,
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(pick, "gpt");
    }

    #[test]
    fn anti_monoculture_redistributes() {
        // 5 sub-tasks, toutes routées vers "claude" → ratio 100% → re-route.
        let sub_tasks = (0..5)
            .map(|i| crate::decompose::SubTask {
                id: (i + 1) as u32,
                task: "x".into(),
                task_class: TaskClass::Reasoning,
            })
            .collect::<Vec<_>>();
        let available = vec!["claude".to_string(), "gpt".to_string()];
        let mut caps: HashMap<String, HashMap<String, f32>> = HashMap::new();
        let mut c = HashMap::new();
        c.insert("REASONING".to_string(), 0.95);
        caps.insert("claude".to_string(), c);
        let mut c2 = HashMap::new();
        c2.insert("REASONING".to_string(), 0.80);
        caps.insert("gpt".to_string(), c2);

        let mut routing: Vec<(usize, String)> = (0..5).map(|i| (i, "claude".to_string())).collect();
        enforce_anti_monoculture(&mut routing, &sub_tasks, &available, &caps).unwrap();
        let claude_count = routing.iter().filter(|(_, l)| l == "claude").count();
        // Doit avoir limité claude à ≤ 60% de 5 = 3.
        assert!(claude_count <= 3, "claude_count = {claude_count}");
    }

    #[test]
    fn anti_monoculture_no_alternatives_errors() {
        let sub_tasks = (0..5)
            .map(|i| crate::decompose::SubTask {
                id: (i + 1) as u32,
                task: "x".into(),
                task_class: TaskClass::Code,
            })
            .collect::<Vec<_>>();
        let available = vec!["only-llm".to_string()];
        let caps: HashMap<String, HashMap<String, f32>> = HashMap::new();
        let mut routing: Vec<(usize, String)> = (0..5).map(|i| (i, "only-llm".to_string())).collect();
        let r = enforce_anti_monoculture(&mut routing, &sub_tasks, &available, &caps);
        assert!(matches!(r, Err(MultiLlmError::MonocultureViolation { .. })));
    }
}
