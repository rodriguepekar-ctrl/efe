// SPDX-License-Identifier: UNLICENSED
//! **ADR-016, Mode C RÉEL** — la forensique N17 d'appels LLM **véritables** (Ollama local)
//! survit-elle à la **mort du process**, et se re-prouve-t-elle **hors ligne** ?
//!
//! Le test `lot_n17persist` et les gardiens de la passerelle prouvent la mécanique avec une
//! forensique *synthétique*. Ici, la forensique est **authentique** : chaque
//! `LlmProvenanceRecord` naît d'un vrai appel `gemma3:12b`. On simule la mort du process par
//! un **aller-retour serde JSON** (la frontière exacte du store : `JSONB`), on **jette**
//! l'`out` vivant, puis on re-prouve depuis les seuls octets + une **re-dérivation** de la
//! clé tenant (comme le ferait un tiers, des jours plus tard) — essaim + chaîne N17 + pont.
//!
//! Ignoré par défaut (CI sans modèle). Lancer :
//!   cargo test -p efe-swarm-sdk --test n17_persist_real -- --ignored --nocapture

use efe_nano_sdk::TenantId;
use efe_swarm_sdk::{
    verify_run_offline, AgentCallRecord, Budget, ClientMode, HmacKeySource, LlmProvenanceRecord,
    Swarm, TopologySpec,
};
use serde_json::json;

/// Décode 32 octets depuis un hex de 64 caractères (sinon `None`).
fn unhex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let b = s.as_bytes();
    let nib = |c: u8| -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    };
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = (nib(b[i * 2])? << 4) | nib(b[i * 2 + 1])?;
    }
    Some(out)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "nécessite un Ollama local (ollama serve + gemma3:12b)"]
async fn real_llm_forensics_survive_process_death() {
    // ── 1) un run Mode C RÉEL : Ollama local, forensique authentique. ────────────
    let swarm = Swarm::builder()
        .mode(ClientMode::SelfHost {
            endpoint: "http://127.0.0.1:11434".into(),
            model: "gemma3:12b".into(),
        })
        .require_healthy_backend(true)
        .build()
        .expect("backend Ollama sain");

    // Reproductibilité (« graine épinglée », §8.2) : le SDK fixe DÉTERMINISTIQUEMENT la graine
    // et la température de chaque appel (llm.rs : générateur seed=42+round*31+idx ; juges
    // temperature=0.0, seed=7) — best-effort de reproductibilité côté Ollama. `SamplingParams`
    // est pub(crate) : le test ne peut (ni n'a besoin de) la régler ; la graine épinglée vit
    // dans le SDK et atterrit telle quelle dans chaque `LlmProvenanceRecord.sampling.seed`.
    let spec = TopologySpec::consensus(
        "Classify the topic in one word from {technology,finance,health,legal}.",
        &["Judge how well the label fits the text."],
        3,
    );
    let out = swarm
        .run_traced(
            json!({ "text": "The court ruled the contract clause unenforceable.", "expected": "legal" }),
            spec,
            Budget::cents(1000),
        )
        .await
        .expect("run Mode C réel");

    // La forensique doit être NON VIDE (sinon ce n'est pas un run Mode C — test vide).
    assert!(!out.llm_records.is_empty(), "≥ 1 LlmProvenanceRecord forensique (Mode C réel)");
    assert!(out.run.provenance.verified, "scellé + re-prouvé en vol");
    let n17_in_run = out.run.n17.as_ref().expect("résumé N17 (Mode C)");
    assert!(n17_in_run.all_consistent && n17_in_run.broken == 0, "pont cohérent en vol : {n17_in_run:?}");
    let expected_calls = out.llm_records.len();

    // ── 2) PERSISTANCE (la frontière du store) : sérialise records + forensique +
    //    feuilles + n17_root en JSON, comme `StoredRun` → `JSONB`. ─────────────────
    let records_json = serde_json::to_string(out.records()).expect("records → json");
    let llm_json = serde_json::to_string(&out.llm_records).expect("forensique → json");
    let leaves_hex: Vec<String> = out.n17_sealed.leaves_hex();
    let root_hex: Option<String> = out.n17_sealed.root_hex();

    // ── 3) MORT DU PROCESS : on jette tout objet vivant. Il ne reste que des octets. ──
    drop(out);
    drop(swarm);

    // ── 4) un TIERS re-dérive la clé tenant (test_key ∘ system, comme le SDK keyless),
    //    relit les octets, et RE-PROUVE — sans process, sans provider, sans réseau. ──
    let key = HmacKeySource::test_key().derive(TenantId::system()).expect("clé tenant");
    let records: Vec<AgentCallRecord> = serde_json::from_str(&records_json).expect("records relus");
    let mut llm: Vec<LlmProvenanceRecord> = serde_json::from_str(&llm_json).expect("forensique relue");
    let leaves: Vec<[u8; 32]> = leaves_hex.iter().filter_map(|h| unhex32(h)).collect();
    let root = root_hex.as_deref().and_then(unhex32);

    let v = verify_run_offline(&records, &llm, &leaves, root, &key);
    assert!(v.fully_proven, "la forensique RÉELLE se re-prouve après la mort du process");
    assert!(v.n17_chain.intact, "la chaîne N17 se recompose exactement depuis les octets");
    assert_eq!(v.n17_chain.records, expected_calls, "tous les appels LLM ont survécu");
    assert!(v.bridge.all_mode_c_consistent(), "chaque Mode C résout en forensique cohérente");
    assert_eq!(v.bridge.broken, 0);

    // ── 5) FALSIFICATION : muter un champ forensique d'un record relu casse la
    //    re-preuve et LOCALISE la faute (le store n'est pas digne de confiance). ────
    let victim = llm[0].provenance_id;
    llm[0].op_id = "swarm.FORGÉ".into(); // un attaquant réécrit l'op (sans la clé)
    let vt = verify_run_offline(&records, &llm, &leaves, root, &key);
    assert!(!vt.fully_proven, "forensique altérée → re-preuve cassée");
    assert!(!vt.n17_chain.intact, "la chaîne N17 détecte la mutation");
    assert_eq!(vt.n17_chain.first_break, Some(victim), "la 1ʳᵉ faute est localisée au record muté");
    // l'essaim, lui, reste intègre (on n'a touché QUE la forensique).
    assert!(vt.swarm.ok(), "la chaîne d'essaim reste valide (mutation côté N17 seulement)");

    println!("✅ Mode C réel : {expected_calls} forensiques LLM survivent à la mort du process, re-prouvées hors ligne, falsification localisée.");
}
