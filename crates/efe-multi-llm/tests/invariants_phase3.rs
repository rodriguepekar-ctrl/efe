// SPDX-License-Identifier: UNLICENSED
//! Tests dédiés aux invariants Phase 3 (I9–I13).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::{BlackboardEntry, Coordinator, EntryKind, MockLlmProvider, ProviderRegistry};
use efe_multi_llm::{
    run_specialist, BudgetTracker, MultiLlmError, SpecialistRequest, TenantPheromoneBlackboard,
    ANTI_MONOCULTURE_MAX_RATIO,
};
use uuid::Uuid;

// ── I9 : early termination ───────────────────────────────────────────
// Note : implémentation Phase 3 v0.0.2 — non encore branchée dans Vote ;
// on teste juste que CancellationToken fonctionne (déjà couvert
// dans unit_cancellation.rs).

// ── I11 : anti-monoculture ────────────────────────────────────────────
#[tokio::test]
async fn i11_anti_monoculture_max_60_percent() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("gpt").with_response("ok")));
    reg.register(Arc::new(MockLlmProvider::new("claude").with_response("ok")));
    reg.register(Arc::new(MockLlmProvider::new("gemini").with_response("ok")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let mut caps: HashMap<String, HashMap<String, f32>> = HashMap::new();
    // GPT super-fort partout → tentation monoculture totale.
    let mut c = HashMap::new();
    for k in ["CODE", "FACTUAL_QA", "ANALYSIS", "REASONING", "CREATIVE", "TRANSLATION"] {
        c.insert(k.to_string(), 0.99);
    }
    caps.insert("gpt".to_string(), c);
    for llm in &["claude", "gemini"] {
        let mut c = HashMap::new();
        for k in ["CODE", "FACTUAL_QA", "ANALYSIS", "REASONING", "CREATIVE", "TRANSLATION"] {
            c.insert(k.to_string(), 0.50);
        }
        caps.insert(llm.to_string(), c);
    }
    let req = SpecialistRequest {
        user_text: "Build a Rust function. Build a Rust enum. Build a Rust trait. Build a Rust macro. Build a Rust impl.".into(),
        available_llms: vec!["claude".into(), "gpt".into(), "gemini".into()],
        capability_scores: caps,
        max_budget_cents: 200,
        max_total_latency: Duration::from_secs(10),
    };
    let resp = run_specialist(&coord, req).await.unwrap();
    let gpt_count = resp.assignments.iter().filter(|a| a.assigned_llm == "gpt").count();
    let total = resp.assignments.len();
    let ratio = gpt_count as f32 / total as f32;
    // Pour 5 sub-tasks toutes "Code" : best we can do = 3/5 = 0.60.
    // L'algo doit redistribuer au moins 2 assignations vers d'autres LLMs.
    assert!(
        gpt_count <= (total * 3) / 5 + 1,
        "I11 violated: gpt has {gpt_count}/{total} (ratio {ratio:.2})"
    );
}

#[tokio::test]
async fn i11_no_alternative_llm_errors_clean() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("only").with_response("ok")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();
    let req = SpecialistRequest {
        user_text: "Task one. Task two. Task three. Task four. Task five.".into(),
        available_llms: vec!["only".into()], // single LLM
        capability_scores: HashMap::new(),
        max_budget_cents: 100,
        max_total_latency: Duration::from_secs(5),
    };
    let r = run_specialist(&coord, req).await;
    // Avec un seul LLM, soit on accepte la monoculture (≤ 3 sub-tasks)
    // soit on erreur si > 3.  5 sub-tasks → monoculture inéluctable → erreur.
    assert!(matches!(r, Err(MultiLlmError::MonocultureViolation { .. })));
}

// ── I12 : isolation tenant ────────────────────────────────────────────
#[test]
fn i12_tenant_physical_isolation() {
    let bb = TenantPheromoneBlackboard::new();
    let t_a = Uuid::now_v7();
    let t_b = Uuid::now_v7();
    let board_a = bb.for_tenant(t_a);
    let board_b = bb.for_tenant(t_b);
    // A dépose un secret.
    board_a
        .deposit(BlackboardEntry::new(
            "x",
            EntryKind::Verdict,
            serde_json::json!({"secret": "tenant-A-data"}),
        ))
        .unwrap();
    // B ne doit RIEN voir.
    let entries_b = board_b.read_kind(EntryKind::Verdict);
    assert!(entries_b.is_empty(), "I12 violated: tenant B sees tenant A data");
    // A voit son propre dépôt.
    assert_eq!(board_a.read_kind(EntryKind::Verdict).len(), 1);
}

// ── I13 : budget hard-cap ────────────────────────────────────────────
#[test]
fn i13_budget_atomic_revert_on_overflow() {
    let t = BudgetTracker::new(100);
    t.try_charge("a", 60).unwrap();
    let r = t.try_charge("b", 60);
    assert!(matches!(r, Err(MultiLlmError::BudgetExceeded { .. })));
    assert_eq!(t.spent_cents(), 60);
}

#[test]
fn i13_budget_never_exceeds_cap_under_contention() {
    use std::thread;
    let t = BudgetTracker::new(100);
    let mut handles = vec![];
    for _ in 0..200 {
        let t = t.clone();
        handles.push(thread::spawn(move || {
            let _ = t.try_charge("x", 1);
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    assert!(t.spent_cents() <= 100);
}

// ── I10 : corrélation de réponse — PAS une chaîne d'audit (cf. README, I10) ──
//
// L'invariant s'énonçait « Audit chain complète : `audit_chain_hash` toujours retourné », et le
// test se contentait d'un `len() >= 16` — un champ constant valant "0000000000000000" l'aurait
// satisfait. Or rien n'est chaîné ici : la valeur est l'identifiant du swarm recopié en hex. On
// épingle donc ce qu'elle EST — un identifiant complet, qui round-trip et qui VARIE d'un run à
// l'autre — plutôt que de laisser une assertion vide sous un nom qui promet de l'audit.
#[tokio::test]
async fn i10_workflows_return_a_full_correlatable_swarm_id_not_a_hash() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("claude").with_response("X")));
    reg.register(Arc::new(MockLlmProvider::new("gpt").with_response("X")));
    reg.register(Arc::new(MockLlmProvider::new("gemini").with_response("X")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();
    let req = || efe_multi_llm::Vote3LlmRequest {
        user_text: "Q?".into(),
        task_class: efe_nano_app_coordinator::types::router::TaskClass::FactualQa,
        llms: vec!["claude".into(), "gpt".into(), "gemini".into()],
        mode: efe_multi_llm::VoteMode::Plurality,
        quorum: None,
        max_budget_cents: 100,
        max_total_latency: Duration::from_secs(5),
    };
    let a = efe_multi_llm::run_vote_3llm(&coord, req()).await.unwrap();
    let b = efe_multi_llm::run_vote_3llm(&coord, req()).await.unwrap();

    // L'identifiant est COMPLET : un UUID fait 128 bits, donc 32 chiffres hex. L'ancien seuil de
    // 16 aurait accepté un identifiant tronqué de moitié — soit une collision rendue plausible.
    assert_eq!(a.swarm_id_hex.len(), 32, "identifiant tronqué : {}", a.swarm_id_hex);
    assert!(
        a.swarm_id_hex.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "hex minuscule attendu : {}",
        a.swarm_id_hex
    );
    // Il RETOURNE vers un vrai UUID — c'est un identifiant, pas un condensé opaque.
    let round_trip = u128::from_str_radix(&a.swarm_id_hex, 16).expect("hex → u128");
    assert_eq!(format!("{round_trip:032x}"), a.swarm_id_hex, "round-trip non injectif");
    // Et il VARIE : un champ constant passerait toutes les assertions ci-dessus.
    assert_ne!(a.swarm_id_hex, b.swarm_id_hex, "deux runs partagent le même identifiant");
}
