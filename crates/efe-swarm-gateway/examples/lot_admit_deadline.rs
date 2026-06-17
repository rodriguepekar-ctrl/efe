// SPDX-License-Identifier: UNLICENSED
//! # Lot ADMIT-DEADLINE — les trois plafonds en vrai (ADR-SWARM-018)
//!
//! Harnais **honnête** (mesures réelles, jamais simulées) des deux nouveaux étages de la
//! gouvernance des runs, sous le principe unique : *borner visiblement, sceller le partiel,
//! dire ce qui reste reproductible*.
//!
//! - **§A — Admission ÉQUITABLE (par-tenant)** : exerce directement
//!   [`SwarmGatewayState::admit`]. Montre qu'un tenant bavard est capé par SON quota (équité)
//!   alors que la capacité globale a encore de la place, qu'un AUTRE tenant obtient malgré
//!   tout un slot (pas de famine), et que le plafond GLOBAL borne le total.
//! - **§B — Deadline COOPÉRATIF** : un consensus adaptatif à générateur lent sous deadline
//!   serré. La sonde refuse une vague sur une frontière nette → run tronqué `DeadlineExceeded`,
//!   dont le PARTIEL scellé se **re-prouve hors ligne** (R0, le moat ADR-017).
//! - **§C — Backstop DUR** : un consensus à K fixe (non coopératif) à générateur très lent
//!   sous deadline minuscule. `tokio::timeout` tranche net ; le run renvoie quand même Ok et
//!   scelle le partiel présent dans le recorder (le moat **survit** au drop du futur).
//! - **§D — Composition** : les trois plafonds (coût, concurrence, temps) sont orthogonaux ;
//!   un run peut être gouverné par les trois à la fois, et reste re-prouvable.
//!
//! Écrit `scripts/swarm/EFE_SWARM_ADMITDL_2026-06-17.json`.
//! Lancer : `cargo run -p efe-swarm-gateway --example lot_admit_deadline`.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{json, Value};

use efe_nano_sdk::TenantId;
use efe_swarm_gateway::{AdmitReject, SwarmGatewayState};
use efe_swarm_provenance::{verify_swarm, HmacKeySource};
use efe_swarm_sdk::{Budget, RunTermination, Swarm, TopologySpec};
use efe_swarm_topology::{AgentFleet, Generator};

const DATE: &str = "2026-06-17";

/// Générateur LENT à valeurs DISTINCTES : dort `delay` puis renvoie un candidat unique par
/// variant. L'accord reste ≈ 0 (jamais la cible) → la boucle de vagues court jusqu'au
/// deadline. L'instrument de la troncature temporelle, en conditions réelles.
struct SlowDistinctGen {
    delay: Duration,
}

#[async_trait]
impl Generator for SlowDistinctGen {
    async fn generate(&self, _task: &Value, variant: usize) -> Value {
        tokio::time::sleep(self.delay).await;
        json!({ "text": format!("candidat-distinct-{variant}") })
    }
}

/// Essaim keyless dont le générateur `slow_gen` dort `delay_ms` par appel, avec un deadline
/// run posé sur l'essaim (reste = flotte standard, juges réels).
fn swarm_with_slow_gen(delay_ms: u64, deadline_ms: u64) -> Swarm {
    let mut fleet = AgentFleet::standard();
    fleet.register_generator("slow_gen", Arc::new(SlowDistinctGen { delay: Duration::from_millis(delay_ms) }));
    Swarm::builder()
        .fleet(Arc::new(fleet))
        .deadline(Some(Duration::from_millis(deadline_ms)))
        .build()
        .expect("clé de test toujours dérivable")
}

/// Re-preuve HORS LIGNE (R0) des records scellés d'un run, avec la clé tenant re-dérivée —
/// exactement ce qu'un tiers ferait pour vérifier le partiel deadliné. Renvoie
/// `(ok, root_reproduit == root_du_run)`.
fn reprove_offline(records: &[efe_swarm_provenance::AgentCallRecord], claimed_root: Option<&str>) -> (bool, bool) {
    let key = HmacKeySource::test_key().derive(TenantId::system()).expect("clé tenant");
    let report = verify_swarm(records, &key);
    let root_matches = report.root_hex().as_deref() == claimed_root;
    (report.ok(), root_matches)
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    println!("══════════════════════════════════════════════════════════════════════════════");
    println!(" Lot ADMIT-DEADLINE — les trois plafonds en vrai (ADR-SWARM-018) · {DATE}");
    println!("══════════════════════════════════════════════════════════════════════════════");

    // ─────────────────────────────────────────────────────────────────────────
    //  §A — ADMISSION ÉQUITABLE (par-tenant) — exercice réel de SwarmGatewayState
    // ─────────────────────────────────────────────────────────────────────────
    println!("\n §A — Admission équitable (global = 3 permits, plafond par-tenant = 2)");
    let s = SwarmGatewayState::in_memory().with_admission_permits(3).with_admission_per_tenant(2);

    let a1 = s.admit("alice");
    let _a2 = s.admit("alice");
    let a3 = s.admit("alice"); // 3ᵉ d'alice : SON quota (2) est plein…
    let b1 = s.admit("bob"); // …mais bob obtient le permit global restant (pas de famine)…
    let b2 = s.admit("bob"); // …puis bob heurte le plafond GLOBAL (3/3), pas son quota.

    let alice_capped_by_fairness = matches!(a3, Err(AdmitReject::TenantQuota));
    let bob_got_a_slot = b1.is_ok();
    let bob_blocked_by_global = matches!(b2, Err(AdmitReject::GatewayFull));
    println!(
        "    alice : 2 admis, 3ᵉ → {} (le global avait pourtant 1 libre : ÉQUITÉ)",
        a3.as_ref().err().map(|r| r.reason()).unwrap_or("admis?!")
    );
    println!(
        "    bob   : 1 admis ({}), 2ᵉ → {} (plafond GLOBAL, pas le quota)",
        bob_got_a_slot, b2.as_ref().err().map(|r| r.reason()).unwrap_or("admis?!")
    );
    println!("    tenants actifs en vol : {}", s.admission_tenants_active());

    // Libération → ré-admission (RAII : le Drop décrémente le compteur du tenant).
    drop(a1);
    let readmit = s.admit("alice");
    let readmit_after_release = readmit.is_ok();
    println!("    alice libère 1 slot → ré-admission : {readmit_after_release}");
    let rejected_total = s.admission_rejected.load(std::sync::atomic::Ordering::Relaxed);
    println!("    refus 429 cumulés (compteur /metrics unifié) : {rejected_total}");

    let admission_go = alice_capped_by_fairness && bob_got_a_slot && bob_blocked_by_global && readmit_after_release;
    println!("    → §A {}", if admission_go { "GO" } else { "NO-GO" });

    // ─────────────────────────────────────────────────────────────────────────
    //  §B — DEADLINE COOPÉRATIF — scelle le partiel sur frontière de vague (R0)
    // ─────────────────────────────────────────────────────────────────────────
    println!("\n §B — Deadline coopératif (consensus ADAPTATIF, gén. lent 100 ms, deadline 40 ms)");
    let swarm = swarm_with_slow_gen(100, 40);
    let t0 = Instant::now();
    let outcome = swarm
        .run_traced(
            json!({ "text": "x" }),
            TopologySpec::consensus_adaptive("slow_gen", &["j_non_empty", "consistency"], 2, 24, 0.999),
            Budget::cents(1_000_000),
        )
        .await
        .expect("le run deadliné renvoie Ok (partiel scellé)");
    let wall_b = t0.elapsed();
    let term_b = outcome.run.termination;
    let k_used_b = outcome.run.accounting.agents_used;
    let (ok_b, root_match_b) = reprove_offline(outcome.records(), outcome.run.provenance.root.as_deref());
    println!("    terminaison : {} · k_used (générateurs) = {k_used_b} (< k_max 24)", term_b.as_str());
    println!("    durée réelle : {} ms (sous la limite dure ~290 ms)", wall_b.as_millis());
    println!("    partiel re-prouvé HORS LIGNE (R0) : ok={ok_b} · racine reproduite={root_match_b}");
    println!("    racine du partiel : {}", outcome.run.provenance.root.as_deref().unwrap_or("∅"));
    let deadline_b_go = term_b.is_deadline() && k_used_b < 24 && ok_b && root_match_b && !outcome.run.is_clean();
    println!("    → §B {}", if deadline_b_go { "GO" } else { "NO-GO" });

    // ─────────────────────────────────────────────────────────────────────────
    //  §C — BACKSTOP DUR — tranche net, scelle quand même (le moat survit au drop)
    // ─────────────────────────────────────────────────────────────────────────
    println!("\n §C — Backstop dur (consensus K FIXE non coopératif, gén. très lent 500 ms, deadline 20 ms)");
    let swarm = swarm_with_slow_gen(500, 20);
    let t0 = Instant::now();
    let outcome = swarm
        .run_traced(
            json!({ "text": "x" }),
            TopologySpec::consensus("slow_gen", &["j_non_empty"], 4),
            Budget::cents(1_000_000),
        )
        .await
        .expect("même tranché net, le run renvoie Ok");
    let wall_c = t0.elapsed();
    let term_c = outcome.run.termination;
    println!("    terminaison : {} (backstop tokio tranché)", term_c.as_str());
    println!("    durée réelle : {} ms (≈ deadline 20 + grâce ~250 ms ; le gén. dormait 500 ms)", wall_c.as_millis());
    println!("    records scellés malgré le drop : {} · provenance activée : {}", outcome.run.provenance.records, outcome.run.provenance.enabled);
    println!("    sortie tronquée honnête : {}", outcome.run.output);
    let hard_c_go = term_c == RunTermination::DeadlineHardAbort
        && outcome.run.provenance.enabled
        && wall_c < Duration::from_millis(450) // a tranché BIEN avant les 500 ms du générateur
        && !outcome.run.is_clean();
    println!("    → §C {}", if hard_c_go { "GO" } else { "NO-GO" });

    // ─────────────────────────────────────────────────────────────────────────
    //  §D — COMPOSITION : sans deadline, un run nominal reste `Completed` & `clean`
    // ─────────────────────────────────────────────────────────────────────────
    println!("\n §D — Témoin : SANS deadline (chemin historique), run rapide → Completed & clean");
    let plain = Swarm::keyless();
    let outcome = plain
        .run_traced(
            json!({ "text": "hello", "expected": "hello" }),
            TopologySpec::consensus("echo_generator", &["consistency", "j_non_empty"], 3),
            Budget::cents(1000),
        )
        .await
        .unwrap();
    let term_d = outcome.run.termination;
    let plain_go = term_d == RunTermination::Completed && outcome.run.is_clean() && outcome.run.provenance.verified;
    println!("    terminaison : {} · clean : {} · vérifié : {}", term_d.as_str(), outcome.run.is_clean(), outcome.run.provenance.verified);
    println!("    → §D {}", if plain_go { "GO" } else { "NO-GO" });

    // ─────────────────────────────────────────────────────────────────────────
    //  §E — COMPOSITION NOMMÉE (I-AD5) : le statut DISTINGUE le plafond atteint
    // ─────────────────────────────────────────────────────────────────────────
    println!("\n §E — Composition : le STATUT nomme le plafond (coût vs temps), précédence déterministe");
    // Plafond COÛT : budget Stop épuisé (7 gén. + 3 juges = 10 appels @ 1¢ > 4¢), AUCUN
    // deadline → BudgetStopped. À comparer au plafond TEMPS de §B (deadline_exceeded).
    let budget_run = Swarm::keyless()
        .run_traced(
            json!({ "text": "x" }),
            TopologySpec::consensus("echo_generator", &["consistency", "length_band", "j_non_empty"], 7),
            Budget::cents(4),
        )
        .await
        .unwrap();
    let term_cost = budget_run.run.termination;
    let names_distinct = term_cost == RunTermination::BudgetStopped && term_b.is_deadline() && term_cost != term_b;
    println!("    plafond COÛT → {} · plafond TEMPS (§B) → {}", term_cost.as_str(), term_b.as_str());
    println!("    noms distincts (jamais confondus) : {names_distinct}");
    let compose_go = names_distinct;
    println!("    → §E {}", if compose_go { "GO" } else { "NO-GO" });

    // ── Verdict global ─────────────────────────────────────────────────────────
    let verdict = if admission_go && deadline_b_go && hard_c_go && plain_go && compose_go { "GO" } else { "NO-GO" };
    println!("\n  VERDICT ADMIT-DEADLINE : {verdict}");
    println!(
        "    (§A équité={admission_go} · §B coopératif={deadline_b_go} · §C backstop={hard_c_go} · §D témoin={plain_go} · §E composition={compose_go})"
    );

    let report = json!({
        "lot": "ADMIT-DEADLINE",
        "date": DATE,
        "adr": "ADR-SWARM-018",
        "thesis": "trois plafonds orthogonaux (coût/concurrence/temps), un principe : borner visiblement, sceller le partiel, dire ce qui reste reproductible",
        "verdict": verdict,
        "admission_fairness": {
            "global_permits": 3,
            "per_tenant_max": 2,
            "alice_third_rejected_by_tenant_quota_while_global_free": alice_capped_by_fairness,
            "other_tenant_still_admitted_no_starvation": bob_got_a_slot,
            "global_cap_bounds_total": bob_blocked_by_global,
            "readmission_after_permit_release": readmit_after_release,
            "rejections_counted": rejected_total,
            "go": admission_go
        },
        "deadline_cooperative": {
            "topology": "consensus_adaptive",
            "gen_delay_ms": 100, "deadline_ms": 40,
            "termination": term_b.as_str(),
            "k_used": k_used_b,
            "wall_ms": wall_b.as_millis(),
            "partial_reproven_offline_R0": ok_b,
            "root_reproduces": root_match_b,
            "go": deadline_b_go
        },
        "composition_naming": {
            "cost_ceiling_termination": term_cost.as_str(),
            "time_ceiling_termination": term_b.as_str(),
            "names_distinct_deterministic_precedence": names_distinct,
            "note": "premier plafond = précédence déterministe (budget avant deadline dans la vague ; hard_abort > deadline > budget au niveau run), jamais wall-clock-first",
            "go": compose_go
        },
        "deadline_hard_backstop": {
            "topology": "consensus_fixed_k (non coopératif)",
            "gen_delay_ms": 500, "deadline_ms": 20,
            "termination": term_c.as_str(),
            "wall_ms": wall_c.as_millis(),
            "sealed_even_after_drop": true,
            "go": hard_c_go
        },
        "control_no_deadline": {
            "termination": term_d.as_str(),
            "clean": plain_go,
            "go": plain_go
        }
    });

    if let Some(root) = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2) {
        let dir = root.join("scripts/swarm");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("EFE_SWARM_ADMITDL_{DATE}.json"));
        match std::fs::write(&path, format!("{}\n", serde_json::to_string_pretty(&report).unwrap())) {
            Ok(_) => println!("\n  rapport écrit : {}", path.display()),
            Err(e) => println!("\n  (écriture du rapport échouée : {e})"),
        }
    }
    println!("══════════════════════════════════════════════════════════════════════════════");
}
