// SPDX-License-Identifier: UNLICENSED
//! # `lot_replay` — le **GATE** d'ADR-017 : le rejeu déterministe re-prouve-t-il la racine
//! (R0) et re-dérive-t-il les sorties (R1) — et l'asymétrie identité est-elle honnête ?
//!
//! Question : la `provenance_root` se **re-prouve**-t-elle par re-scellement des records
//! (R0, ordre canonique ADR-007), et la **computation** se **re-dérive**-t-elle (R1, mêmes
//! `output_hash`) — tout en assumant que la racine **lie l'identité par-run** (`swarm_id`,
//! `record_id`) donc **ne se reproduit PAS** par re-exécution ? Une divergence est-elle
//! **localisée** ? À coût négligeable ?
//!
//! Harnais HONNÊTE (doctrine Lot P4) : **crypto RÉELLE** (BLAKE3 + HMAC + HKDF) +
//! `replay`/`replay_root`/`replay_outputs` RÉELS ; agents **purs synthétiques** (fonction de
//! `(task, variant)`), donc Mode A — **clairement étiqueté**.
//!
//! GO ⇔ ① R0 re-prouve la racine (re-seal → root identique) ∧ ② asymétrie identité démontrée
//! (deux runs de la MÊME computation → racines DIFFÉRENTES, mais sorties IDENTIQUES) ∧
//! ③ R1 re-dérive toutes les sorties (replay_outputs déterministe) ∧ ④ falsification localisée
//! (R0 content-break + R1 output-mismatch) ∧ ⑤ coût borné.
//!
//! Run :  cargo run --release --example lot_replay -p efe-swarm-provenance

use std::fs;
use std::time::Instant;

use efe_nano_sdk::{AgentId, SwarmId, TenantId};
use efe_swarm_provenance::{
    deterministic_order, replay, replay_outputs, replay_root, AgentCallDraft, AgentCallRecord,
    AgentKind, AgentRole, CellRef, HmacKeySource, SwarmChain,
};
use serde_json::{json, Value};

const RUNS: usize = 300; // itérations de mesure
const WARMUP: usize = 30;
const K: usize = 8; // agents purs (générateurs) par run synthétique
const VERIFY_US_PER_RECORD_BUDGET: f64 = 2000.0;

fn tenant_key() -> Vec<u8> {
    HmacKeySource::test_key().derive(TenantId::system()).unwrap()
}

/// Un agent PUR : sa sortie est une fonction déterministe de `(task, variant)`. C'est le
/// cœur rejouable du Mode A — re-l'exécuter redonne mot pour mot la même valeur.
fn pure_agent(task: &Value, variant: u64) -> Value {
    json!({ "echo": task, "variant": variant, "score": (variant * 7 + 3) % 11 })
}

/// Scelle un run synthétique de `K` agents purs (générateurs) avec un `swarm_id` DONNÉ.
/// Deux appels avec des `swarm_id` (et record_id frais) différents → racines différentes,
/// mais MÊME computation (mêmes entrées/sorties).
fn seal_run(task: &Value, swarm: SwarmId, key: &[u8]) -> Vec<AgentCallRecord> {
    let mut chain = SwarmChain::new(TenantId::system(), swarm, key.to_vec());
    let mut drafts: Vec<AgentCallDraft> = Vec::with_capacity(K);
    for i in 0..K as u64 {
        let input = json!({ "task": task, "variant": i });
        let output = pure_agent(task, i);
        drafts.push(
            AgentCallDraft::from_io(
                swarm,
                TenantId::system(),
                AgentId::new("generator").unwrap(),
                AgentRole::Generator,
                AgentKind::ModeA,
                &input,
                &output,
            )
            .with_variant(i)
            .wrote(format!("/cand/{i}"), 1),
        );
    }
    // `replay`/`replay_root` ré-ordonnent canoniquement (ADR-007) ; l'ordre de scellement ici
    // est sans incidence sur la racine re-prouvée.
    chain.append_all(drafts)
}

/// Empreinte de calcul IDENTITÉ-LIBRE (mêmes champs que la passerelle) : tout sauf
/// record_id / swarm_id / parents — ce qui lie le run et empêche la racine de se reproduire.
fn compute_fp(r: &AgentCallRecord) -> String {
    let cells = |cs: &[CellRef]| {
        let mut v: Vec<String> = cs.iter().map(|c| format!("{}@{}", c.pointer, c.version)).collect();
        v.sort();
        v.join(",")
    };
    format!(
        "{}|{}|{}|{:?}|{}|{}|r[{}]|w[{}]",
        r.agent_id.as_str(),
        r.agent_role.as_str(),
        r.agent_kind.as_str(),
        r.variant,
        r.input_hash.to_hex(),
        r.output_hash.to_hex(),
        cells(&r.reads),
        cells(&r.writes),
    )
}

fn fps(records: &[AgentCallRecord]) -> Vec<String> {
    let mut v: Vec<String> = records.iter().map(compute_fp).collect();
    v.sort();
    v
}

fn main() {
    println!("══════════════════════════════════════════════════════════════════════════════");
    println!(" LOT REPLAY — gate ADR-017 : rejeu déterministe (R0 re-prouve la racine, R1");
    println!(" re-dérive les sorties) — l'asymétrie identité assumée honnêtement.");
    println!(" crypto RÉELLE · replay/replay_root/replay_outputs RÉELS · agents purs SYNTHÉTIQUES");
    println!(" K={K} agents Mode A · {RUNS} runs de mesure");
    println!("══════════════════════════════════════════════════════════════════════════════");

    let key = tenant_key();
    let task = json!({ "text": "classify", "expected": "legal" });

    // ── run de référence (swarm_id A). ───────────────────────────────────────────
    let run_a = seal_run(&task, SwarmId::new(), &key);
    let sealed_root_a = replay_root(&run_a, &key);

    // ── ① R0 : re-sceller depuis les records → la racine se reproduit (ordre neutre). ──
    let mut shuffled = run_a.clone();
    shuffled.reverse();
    let r0 = replay(&shuffled, &key);
    let r0_root_reproduced = r0.matches(sealed_root_a) && r0.content_intact;

    // ── ② asymétrie identité : MÊME computation, swarm_id B → racine DIFFÉRENTE, mais
    //    sorties IDENTIQUES (mêmes empreintes de calcul). ─────────────────────────────
    let run_b = seal_run(&task, SwarmId::new(), &key);
    let root_b = replay_root(&run_b, &key);
    let roots_differ = sealed_root_a != root_b; // attendu : la racine lie l'identité par-run
    let outputs_identical = fps(&run_a) == fps(&run_b); // attendu : la computation se reproduit

    // ── ③ R1 : re-dérive les sorties (replay_outputs) — closure pure (task + variant). ──
    let rout = replay_outputs(&run_a, |r| pure_agent(&task, r.variant.unwrap_or(0)));
    let outputs_rederived = rout.deterministic() && rout.matched == run_a.len();

    // ── ④ falsification localisée : R0 content-break + R1 output-mismatch. ───────────
    let mut tampered = run_a.clone();
    let victim_pos = 3.min(tampered.len() - 1);
    tampered[victim_pos].output_hash = efe_nano_sdk::Blake3Hash::from_bytes([0xEE; 32]);
    let r0_tamper = replay(&tampered, &key);
    // localisation R0 : l'index du break est sur l'ordre CANONIQUE.
    let ordered = deterministic_order(&tampered);
    let r0_localized = !r0_tamper.content_intact
        && r0_tamper.first_content_break.is_some_and(|i| {
            ordered.get(i).map(|r| r.output_hash) == Some(efe_nano_sdk::Blake3Hash::from_bytes([0xEE; 32]))
        });
    // localisation R1 : une repro fausse pour un seul variant → mismatch localisé.
    let rout_tamper = replay_outputs(&run_a, |r| {
        let v = r.variant.unwrap_or(0);
        if v == 5 {
            json!({ "MENTEUR": true })
        } else {
            pure_agent(&task, v)
        }
    });
    let r1_localized = rout_tamper.mismatched == 1 && rout_tamper.first_mismatch.is_some();

    // ── ⑤ coût : latence de replay (R0) + replay_outputs (R1). ───────────────────────
    let mut r0_ns = Vec::with_capacity(RUNS);
    let mut r1_ns = Vec::with_capacity(RUNS);
    for i in 0..(RUNS + WARMUP) {
        let t0 = Instant::now();
        let _ = replay(&run_a, &key);
        let d0 = t0.elapsed().as_nanos() as f64;
        let t1 = Instant::now();
        let _ = replay_outputs(&run_a, |r| pure_agent(&task, r.variant.unwrap_or(0)));
        let d1 = t1.elapsed().as_nanos() as f64;
        if i >= WARMUP {
            r0_ns.push(d0);
            r1_ns.push(d1);
        }
    }
    let mean = |xs: &[f64]| xs.iter().sum::<f64>() / xs.len() as f64;
    let us = |ns: f64| ns / 1000.0;
    let r0_us = us(mean(&r0_ns));
    let r1_us = us(mean(&r1_ns));
    let r0_us_per_record = r0_us / K as f64;
    let r1_us_per_record = r1_us / K as f64;
    let cost_ok = r0_us_per_record <= VERIFY_US_PER_RECORD_BUDGET && r1_us_per_record <= VERIFY_US_PER_RECORD_BUDGET;

    // ── verdict. ─────────────────────────────────────────────────────────────────
    let go = r0_root_reproduced && roots_differ && outputs_identical && outputs_rederived && r0_localized && r1_localized && cost_ok;
    let verdict = if go {
        "GO ✅ — R0 re-prouve la racine · R1 re-dérive les sorties · asymétrie identité honnête · divergence localisée"
    } else {
        "NO-GO ❌"
    };

    let hex = |o: Option<[u8; 32]>| o.map(|b| b.iter().map(|x| format!("{x:02x}")).collect::<String>()).unwrap_or_else(|| "∅".into());
    println!("\n ① R0 — re-sceller depuis les records (ADR-007) :");
    println!("   racine scellée (A)   : {}", hex(sealed_root_a));
    println!("   racine re-scellée    : {}  → reproduite={r0_root_reproduced}", hex(r0.root));
    println!("\n ② ASYMÉTRIE IDENTITÉ — même computation, swarm_id différent :");
    println!("   racine run A         : {}", hex(sealed_root_a));
    println!("   racine run B         : {}", hex(root_b));
    println!("   → racines DIFFÉRENTES (la racine lie swarm_id/record_id) : {roots_differ}");
    println!("   → sorties IDENTIQUES (mêmes empreintes de calcul)        : {outputs_identical}");
    println!("\n ③ R1 — re-dériver les sorties (replay_outputs) :");
    println!("   {} / {} sorties re-dérivées à l'identique → deterministic={outputs_rederived}", rout.matched, rout.checked);
    println!("\n ④ FALSIFICATION localisée :");
    println!("   R0 (output muté @cand/{victim_pos}) → content-break localisé : {r0_localized}");
    println!("   R1 (repro fausse pour variant 5)    → output-mismatch localisé (1) : {r1_localized}");
    println!("\n ⑤ COÛT ({RUNS} runs) :");
    println!("   R0 replay           : {r0_us:.1} µs/run · {r0_us_per_record:.1} µs/record (budget {VERIFY_US_PER_RECORD_BUDGET:.0})");
    println!("   R1 replay_outputs   : {r1_us:.1} µs/run · {r1_us_per_record:.1} µs/record");

    println!("\n────────────────────────── VERDICT LOT REPLAY ──────────────────────────");
    println!(" {verdict}");
    println!("─────────────────────────────────────────────────────────────────────────");

    let payload = json!({
        "_note": "Agents PURS synthétiques (Mode A) ; crypto (BLAKE3+HMAC+HKDF) + replay/replay_root/replay_outputs RÉELS. La racine lie l'identité par-run (swarm_id, record_id) → re-provable par R0, non reproductible par re-exécution ; R1 re-dérive les SORTIES.",
        "k": K,
        "r0_reseal": { "root_reproduced": r0_root_reproduced, "content_intact": r0.content_intact },
        "identity_asymmetry": { "roots_differ": roots_differ, "outputs_identical": outputs_identical },
        "r1_outputs": { "checked": rout.checked, "matched": rout.matched, "rederived": outputs_rederived },
        "tamper_localized": { "r0_content_break": r0_localized, "r1_output_mismatch": r1_localized },
        "cost": { "r0_us_per_run": r0_us, "r1_us_per_run": r1_us, "r0_us_per_record": r0_us_per_record, "r1_us_per_record": r1_us_per_record },
        "budgets": { "verify_us_per_record": VERIFY_US_PER_RECORD_BUDGET },
        "verdict": verdict,
        "go": go
    });
    let _ = fs::create_dir_all("scripts/swarm");
    let path = "scripts/swarm/EFE_SWARM_REPLAY_2026-06-17.json";
    let _ = fs::write(path, serde_json::to_string_pretty(&payload).unwrap());
    println!("\n✅ Résultats : {path}");
    assert!(go, "GATE ADR-017 : le rejeu déterministe doit tenir");
}
