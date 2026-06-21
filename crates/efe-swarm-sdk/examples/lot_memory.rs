// SPDX-License-Identifier: UNLICENSED
//! # `lot_memory` — le **GATE** d'ADR-019 : la mémoire inter-runs **apprend**-elle, se
//! **re-prouve**-t-elle, et reste-t-elle **honnête** ? Le jumeau, côté mémoire, de `lot_replay`
//! et `lot_n17persist`.
//!
//! Quatre questions, exécutées AVEC le build (la discipline des lots) :
//! - **§A — K cold vs warm** : un run sur une famille APPRISE (4 runs passés à K=6) démarre-t-il
//!   près du K historique (warm-start `k_min`=6) là où un run FROID part du plancher (k_min=4) ?
//!   Iso-qualité (les deux finissent propres). La mémoire AMORCE, bornée (I-MEM4).
//! - **§B — `memory_root` re-prouvé + falsification localisée** : la chaîne se re-scelle hors
//!   ligne (root reproduite), et une mutation d'un `FoldRecord` la CASSE + la LOCALISE (I-MEM5).
//! - **§C — la porte clean (I-MEM3)** : un run propre replie ; un run SALE est mis en quarantaine.
//! - **§D — pin reproduit (I-MEM7)** : `snapshot_at(N)` reconstruit l'épingle EXACTE d'alors.
//!
//! Harnais **HONNÊTE** (doctrine Lot P4) : agents **purs synthétiques** (Mode A, keyless —
//! clairement étiqueté) + **crypto RÉELLE** (BLAKE3 feuille `content_hash` / SHA-256 feuille de
//! chaîne / HMAC `memory_root` / HKDF clé tenant) + `seal_memory`/`verify_memory` RÉELS.
//!
//! GO ⇔ §A ∧ §B ∧ §C ∧ §D. Écrit `scripts/swarm/EFE_SWARM_MEMORY_2026-06-17.json`.
//!
//! Run :  cargo run --release --example lot_memory -p efe-swarm-sdk

use std::fs;
use std::sync::Arc;
use std::time::Instant;

use efe_nano_sdk::TenantId;
use efe_swarm_sdk::{
    family_key, seal_memory, verify_memory, Budget, FamilyMode, HmacKeySource, InMemoryMemoryStore,
    MemoryConfig, MemoryHandle, MemoryRunSummary, MemoryStore, MemoryUpdate, Swarm, TopologySpec,
};
use serde_json::json;
use uuid::Uuid;

const SEAL_RUNS: usize = 300; // itérations de mesure du coût seal/verify
const WARMUP: usize = 30;
const VERIFY_US_PER_RECORD_BUDGET: f64 = 2000.0;

#[tokio::main]
async fn main() {
    let tenant = TenantId::system().to_string();
    let key = HmacKeySource::test_key().derive(TenantId::system()).unwrap();
    let task = json!({ "text": "x", "expected": "x" });
    let judges = ["consistency", "j_non_empty"];
    let agents = ["const_generator".to_string(), "consistency".to_string(), "j_non_empty".to_string()];
    let family = family_key(&task, "consensus", &agents, FamilyMode::Structural);

    // ── §A — K cold vs warm ──────────────────────────────────────────────────────────
    // FROID : aucune mémoire → le sizer adaptatif part du plancher (k_min = 4). const_generator
    // est unanime (valeur constante « CONST », présente) → accord 1.0 → s'arrête au plancher → K = 4.
    let cold = Swarm::keyless()
        .run(
            task.clone(),
            TopologySpec::consensus_adaptive("const_generator", &judges, 4, 24, 0.9),
            Budget::cents(1_000_000),
        )
        .await
        .unwrap();
    let cold_k = cold.quality.candidates;

    // CHAUD : 4 runs passés à K=6 seedés → le prior suggère 6 → warm-start k_min = 6 → K = 6.
    let store = Arc::new(InMemoryMemoryStore::new());
    for r in 0u128..4 {
        store
            .fold(
                &tenant,
                MemoryUpdate { run_id: Uuid::from_u128(0x6000 + r), family: family.clone(), summary: MemoryRunSummary::clean(6) },
            )
            .await
            .unwrap();
    }
    let warm = Swarm::builder()
        .memory(MemoryHandle::with_config(store.clone(), MemoryConfig::recall_only()))
        .build()
        .unwrap()
        .run(
            task.clone(),
            TopologySpec::consensus_adaptive("const_generator", &judges, 4, 24, 0.9),
            Budget::cents(1_000_000),
        )
        .await
        .unwrap();
    let warm_k = warm.quality.candidates;
    let warm_start = warm.memory.as_ref().and_then(|m| m.warm_start_k_min);
    let a_ok = cold_k == 4 && warm_k == 6 && warm_start == Some(6) && cold.is_clean() && warm.is_clean();

    // ── §B — memory_root re-prouvé + falsification localisée ─────────────────────────
    // On construit un journal RÉEL (5 replis propres) via le store, on le relit, et on prouve la
    // re-provabilité hors ligne + la détection/localisation d'une mutation de contenu.
    let bstore = InMemoryMemoryStore::new();
    for r in 0u128..5 {
        bstore
            .fold(
                &tenant,
                MemoryUpdate { run_id: Uuid::from_u128(0xB000 + r), family: family.clone(), summary: MemoryRunSummary::clean(4 + (r as u32 % 3)) },
            )
            .await
            .unwrap();
    }
    let journal = bstore.journal(&tenant, &family).await.unwrap();
    let sealed = seal_memory(&journal, &key);
    // re-scelle indépendamment → root identique (re-provabilité, I-MEM5).
    let reseal = seal_memory(&journal, &key);
    let root_reproduced = sealed.memory_root.is_some() && sealed.memory_root == reseal.memory_root;
    let intact = verify_memory(&journal, &sealed.leaf_hashes, sealed.memory_root, &key).intact;
    // falsification : on mute le contenu d'un repli → la chaîne casse + localise.
    let mut tampered = journal.clone();
    tampered[2].summary.k_used = 999;
    let tv = verify_memory(&tampered, &sealed.leaf_hashes, sealed.memory_root, &key);
    let tamper_localized = !tv.intact && tv.first_break.is_some();
    let b_ok = root_reproduced && intact && tamper_localized;

    // mesure du coût (seal + verify) sur le journal réel.
    for _ in 0..WARMUP {
        let s = seal_memory(&journal, &key);
        let _ = verify_memory(&journal, &s.leaf_hashes, s.memory_root, &key);
    }
    let t_seal = Instant::now();
    for _ in 0..SEAL_RUNS {
        let _ = seal_memory(&journal, &key);
    }
    let seal_us = t_seal.elapsed().as_micros() as f64 / SEAL_RUNS as f64;
    let t_verify = Instant::now();
    for _ in 0..SEAL_RUNS {
        let _ = verify_memory(&journal, &sealed.leaf_hashes, sealed.memory_root, &key);
    }
    let verify_us = t_verify.elapsed().as_micros() as f64 / SEAL_RUNS as f64;
    let verify_us_per_record = verify_us / journal.len().max(1) as f64;
    let cost_ok = verify_us_per_record <= VERIFY_US_PER_RECORD_BUDGET;

    // ── §C — la porte clean (I-MEM3) ─────────────────────────────────────────────────
    let cstore = InMemoryMemoryStore::new();
    let clean_folded = cstore
        .fold(&tenant, MemoryUpdate { run_id: Uuid::from_u128(0xC001), family: family.clone(), summary: MemoryRunSummary::clean(6) })
        .await
        .unwrap()
        .folded;
    let mut dirty = MemoryRunSummary::clean(6);
    dirty.clean = false; // run tronqué / non vérifié
    let dirty_folded = cstore
        .fold(&tenant, MemoryUpdate { run_id: Uuid::from_u128(0xC002), family: family.clone(), summary: dirty })
        .await
        .unwrap()
        .folded;
    let c_version = cstore.snapshot(&tenant, &family).await.unwrap().version;
    let c_ok = clean_folded && !dirty_folded && c_version == 1;

    // ── §D — pin reproduit (I-MEM7) ──────────────────────────────────────────────────
    let dstore = InMemoryMemoryStore::new();
    dstore.fold(&tenant, MemoryUpdate { run_id: Uuid::from_u128(0xD001), family: family.clone(), summary: MemoryRunSummary::clean(6) }).await.unwrap();
    dstore.fold(&tenant, MemoryUpdate { run_id: Uuid::from_u128(0xD002), family: family.clone(), summary: MemoryRunSummary::clean(4) }).await.unwrap();
    let pin_v2 = dstore.snapshot(&tenant, &family).await.unwrap().content_hash; // épingle à v2
    dstore.fold(&tenant, MemoryUpdate { run_id: Uuid::from_u128(0xD003), family: family.clone(), summary: MemoryRunSummary::clean(8) }).await.unwrap();
    let current = dstore.snapshot(&tenant, &family).await.unwrap();
    let past = dstore.snapshot_at(&tenant, &family, 2).await.unwrap();
    let d_ok = past.content_hash == pin_v2 && past.version == 2 && past.content_hash != current.content_hash;

    // ── verdict ──────────────────────────────────────────────────────────────────────
    let go = a_ok && b_ok && c_ok && d_ok;
    let verdict = if go {
        "GO ✅ — la mémoire apprend (warm-start borné) · se re-prouve · localise une falsification · la porte clean tient · le pin reproduit"
    } else {
        "NO-GO ❌"
    };

    let hexr = sealed.memory_root.map(|b| b.iter().map(|x| format!("{x:02x}")).collect::<String>()).unwrap_or_else(|| "∅".into());
    println!("\n §A K COLD vs WARM :");
    println!("   froid (sans mémoire)   K = {cold_k}  (plancher k_min)");
    println!("   chaud (4×K=6 appris)   K = {warm_k}  · warm_start_k_min = {warm_start:?}   → A_ok={a_ok}");
    println!("\n §B memory_root RE-PROUVÉ + falsification :");
    println!("   root                   {hexr}");
    println!("   re-scellée == root     : {root_reproduced} · journal intègre : {intact}");
    println!("   mutation @repli #2 → cassée+localisée : {tamper_localized}   → B_ok={b_ok}");
    println!("\n §C porte CLEAN (I-MEM3) :");
    println!("   clean replie={clean_folded} · dirty replie={dirty_folded} · version={c_version}   → C_ok={c_ok}");
    println!("\n §D PIN reproduit (I-MEM7) :");
    println!("   snapshot_at(2) == épingle d'alors, ≠ courant : {d_ok}");
    println!("\n §coût ({SEAL_RUNS} itérations, {} replis) :", journal.len());
    println!("   seal {seal_us:.1} µs · verify {verify_us:.1} µs · {verify_us_per_record:.1} µs/record (budget {VERIFY_US_PER_RECORD_BUDGET:.0}) → cost_ok={cost_ok}");

    println!("\n────────────────────────── VERDICT LOT MEMORY ──────────────────────────");
    println!(" {verdict}");
    println!("─────────────────────────────────────────────────────────────────────────");

    let payload = json!({
        "_note": "Agents PURS synthétiques (Mode A, keyless) ; crypto RÉELLE (BLAKE3 épingle content_hash · SHA-256 feuille de chaîne · HMAC memory_root · HKDF clé tenant) + seal_memory/verify_memory RÉELS. Le warm-start AMORCE k_min, borné aux bornes du run (I-MEM4).",
        "A_cold_warm": { "cold_k": cold_k, "warm_k": warm_k, "warm_start_k_min": warm_start, "iso_quality_both_clean": cold.is_clean() && warm.is_clean(), "ok": a_ok },
        "B_reprove": { "root": hexr, "root_reproduced": root_reproduced, "journal_intact": intact, "tamper_localized": tamper_localized, "records": journal.len(), "ok": b_ok },
        "C_clean_gate": { "clean_folded": clean_folded, "dirty_folded": dirty_folded, "version": c_version, "ok": c_ok },
        "D_pin_repro": { "snapshot_at_matches_past_pin": d_ok, "ok": d_ok },
        "cost": { "seal_us": seal_us, "verify_us": verify_us, "verify_us_per_record": verify_us_per_record, "cost_ok": cost_ok },
        "budgets": { "verify_us_per_record": VERIFY_US_PER_RECORD_BUDGET },
        "verdict": verdict,
        "go": go
    });
    let _ = fs::create_dir_all("scripts/swarm");
    let path = "scripts/swarm/EFE_SWARM_MEMORY_2026-06-17.json";
    let _ = fs::write(path, serde_json::to_string_pretty(&payload).unwrap());
    println!("\n✅ Résultats : {path}");
    assert!(go, "GATE ADR-019 : la mémoire inter-runs doit apprendre, se re-prouver, et rester honnête");
}
