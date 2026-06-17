// SPDX-License-Identifier: UNLICENSED
//! # `lot_n17persist` — le **GATE** d'ADR-016 : la forensique N17 survit-elle à la mort
//! du process, et se laisse-t-elle re-prouver **hors ligne** ?
//!
//! Question : une fois le run terminé (process mort, provider parti, réseau coupé), les
//! `LlmProvenanceRecord` du pont N17 — **persistés** à côté des `AgentCallRecord` — se
//! re-prouvent-ils **exactement** (chaîne d'essaim + chaîne N17 + pont), et une **mutation**
//! d'un champ forensique est-elle **détectée et localisée** ? À **coût négligeable** ?
//!
//! Harnais HONNÊTE (même doctrine que Lot P4) : **crypto RÉELLE** (BLAKE3 + SHA-256 + HMAC +
//! HKDF) + **`SwarmChain`/`seal_n17`/`verify_run_offline` RÉELS** ; seuls les **appels
//! LLM** sont simulés — la forensique est fabriquée à la main, **CLAIREMENT étiquetée
//! synthétique**. La *persistance* est éprouvée par un **aller-retour serde JSON** (la
//! frontière exacte du store : `JSONB` en prod) : on sérialise records + forensique +
//! feuilles + `n17_root`, on **jette** les objets vivants, on relit depuis les octets, et
//! on re-prouve — la définition même de « survivre à la mort du process ».
//!
//! On mesure : aller-retour re-prouvé (fully_proven), 4 vecteurs de falsification
//! (champ forensique muté → localisé ; record d'essaim muté ; feuille forgée ; record N17
//! retiré → compte faux), octets/forensique, latence de `seal_n17` + `verify_run_offline`.
//!
//! GO ⇔ aller-retour re-prouvé ∧ Mode A trivialement vérifié ∧ les 4 falsifications
//! détectées (mutation forensique **localisée**) ∧ coût sous budget.
//!
//! Run :  cargo run --release --example lot_n17persist -p efe-swarm-provenance

use std::collections::HashMap;
use std::fs;
use std::time::{Instant, SystemTime};

use efe_nano_sdk::{
    AgentId, Blake3Hash, Completeness, CostAccounting, FinishReason, LlmProvenanceRecord,
    ModelIdentity, NanoAgentId, NanoAgentVersion, PiiStrategy, ProvenanceId, RetryOutcome,
    SamplingParams, SwarmId, TenantId, TokenUsage,
};
use efe_swarm_provenance::{
    seal_n17, verify_run_offline, AgentCallDraft, AgentCallRecord, AgentKind, AgentRole,
    HmacKeySource, SwarmChain,
};
use serde_json::{json, Value};

const RUNS: usize = 300; // itérations de mesure (seal + verify_offline)
const WARMUP: usize = 30;
const MODE_C_CALLS: usize = 5; // appels Mode C du run représentatif (volets ① round-trip, ② falsif.)
const N_SWEEP: [usize; 3] = [1, 10, 100]; // §8.1 ③ : balayage de charge pour la courbe de coût
// budgets figés AU LANCEMENT.
const BYTES_PER_RECORD_BUDGET: f64 = 8192.0; // 8 Ko / forensique (record N17 riche)
const VERIFY_US_PER_RECORD_BUDGET: f64 = 2000.0; // 2 ms / record re-prouvé hors ligne

/// La clé tenant (HKDF réel), comme côté serveur.
fn tenant_key(tenant: TenantId) -> Vec<u8> {
    HmacKeySource::test_key().derive(tenant).unwrap()
}

/// Une forensique N17 déterministe (id fixe → ordre canonique reproductible).
/// **Synthétique** : les hash d'E/S sont des constantes, pas un vrai appel LLM.
fn synth_n17(prov_u128: u128, tenant: TenantId, swarm: SwarmId, model: &str) -> LlmProvenanceRecord {
    LlmProvenanceRecord {
        provenance_id: ProvenanceId::from_uuid(uuid_from_u128(prov_u128)),
        trace_id: uuid_from_u128(0xAA00 + prov_u128),
        swarm_request_id: swarm.as_uuid(),
        op_id: "swarm.generator".into(),
        tenant_id: tenant,
        agent_id: NanoAgentId::new("generator").unwrap(),
        agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
        model_identity: ModelIdentity::hosted("ollama", model, "local"),
        model_hash_verified: false,
        prompt_template_id: "swarm.selfhost".into(),
        prompt_template_version: "1.0.0".into(),
        prompt_inputs_hash: Blake3Hash::ZERO,
        system_prompt_hash: Blake3Hash::ZERO,
        user_prompt_hash: Blake3Hash::from_bytes([(prov_u128 & 0xff) as u8; 32]),
        final_prompt_hash: Blake3Hash::from_bytes([(prov_u128 & 0xff) as u8; 32]),
        sampling: SamplingParams { temperature: 0.2, top_p: None, max_tokens: 256, seed: Some(7), stop_sequences: vec![] },
        token_usage: TokenUsage { client_estimated_input: 120, client_estimated_output: 64, provider_reported_input: None, provider_reported_output: None, is_estimated: true },
        cost_accounting: CostAccounting { cost_cents: 0.0, prices_table_version: 0, provider_quoted_cost_cents: None, overflow_capped: false },
        provider_request_id: None,
        provider_response_time_ms: 42,
        provider_finish_reason: FinishReason::Stop,
        provider_region: None,
        publisher_id: None,
        publisher_kyc_tier: None,
        gold_case_id: None,
        provider_metadata: HashMap::new(),
        output_hash: Some(Blake3Hash::from_bytes([(prov_u128.wrapping_mul(7) & 0xff) as u8; 32])),
        completeness: Completeness::Complete,
        safety_flags: vec![],
        tool_calls: vec![],
        pii_strategy_applied: PiiStrategy::None,
        redaction_token_set: vec![],
        replay_of: None,
        cache_hit: None,
        recorded_at: SystemTime::UNIX_EPOCH,
        streaming_mode: false,
        retry_count: 0,
        retry_outcome: RetryOutcome::NoRetry,
        time_to_first_token_ms: None,
    }
}

/// Construit un run Mode C synthétique : `n` appels LLM, chacun référencé par un
/// `AgentCallRecord` Mode C scellé en chaîne d'essaim, + un juge Mode A (le pont aura donc
/// du Mode A ET du Mode C). Renvoie (records d'essaim, forensique N17, clé).
fn synth_mode_c_run(n: usize) -> (Vec<AgentCallRecord>, Vec<LlmProvenanceRecord>, Vec<u8>) {
    let tenant = TenantId::system();
    let swarm = SwarmId::from_uuid(uuid_from_u128(0x5EED));
    let key = tenant_key(tenant);

    let mut n17: Vec<LlmProvenanceRecord> = Vec::with_capacity(n);
    let mut drafts: Vec<AgentCallDraft> = Vec::with_capacity(n + 1);
    for i in 0..n {
        let prov_u128 = 0xC0FFEE00 + i as u128;
        let rec = synth_n17(prov_u128, tenant, swarm, "gemma3:12b");
        // l'atome Mode C qui RÉFÉRENCE la forensique (le lien du pont N17).
        let draft = AgentCallDraft::new(
            swarm,
            tenant,
            AgentId::new("generator").unwrap(),
            AgentRole::Generator,
            AgentKind::ModeC,
            Blake3Hash::from_bytes([(i + 1) as u8; 32]),
            Blake3Hash::from_bytes([(i + 100) as u8; 32]),
        )
        .with_llm_provenance(rec.provenance_id);
        n17.push(rec);
        drafts.push(draft);
    }
    // un juge Mode A (déterministe, sans LLM) : le pont doit l'ignorer (pas de lien attendu).
    drafts.push(AgentCallDraft::new(
        swarm,
        tenant,
        AgentId::new("judge").unwrap(),
        AgentRole::Judge,
        AgentKind::ModeA,
        Blake3Hash::from_bytes([200u8; 32]),
        Blake3Hash::from_bytes([201u8; 32]),
    ));

    // SCELLER dans l'ordre CANONIQUE (ADR-007) : `verify_swarm` rejoue la chaîne en
    // `deterministic_order` (tri par `key_of(writes, payload_hash)`). `append_all_canonical`
    // applique ce tri AVANT de chaîner (le MÊME chemin que le recorder réel) ⇒ l'ordre de
    // scellement == l'ordre de re-preuve, et la chaîne vérifie sous le moat. (Sceller un lot
    // via `append_all` sans trier casserait `verify_swarm` à tort — cf. chain.rs.)
    let mut chain = SwarmChain::new(tenant, swarm, key.clone());
    let records = chain.append_all_canonical(drafts);
    (records, n17, key)
}

// ── la FRONTIÈRE de persistance : exactement ce que le store écrit (JSONB) puis relit. ──

/// L'enveloppe persistée (le miroir fidèle de `StoredRun` côté passerelle).
fn persist_payload(records: &[AgentCallRecord], n17: &[LlmProvenanceRecord], leaves_hex: &[String], root_hex: &Option<String>) -> Value {
    json!({
        "records": records,        // serde (l'évidence d'essaim)
        "llm_records": n17,        // serde (la forensique N17 — ADR-016)
        "n17_leaves": leaves_hex,  // feuilles SHA-256 (hex, ordre canonique)
        "n17_root": root_hex,      // racine de la chaîne forensique scellée (hex)
    })
}

/// Relit l'enveloppe depuis les octets (process mort) : déserialise tout, redécode les
/// feuilles/`n17_root` en `[u8;32]`. C'est l'entrée EXACTE de `verify_run_offline`.
#[allow(clippy::type_complexity)]
fn reload(payload: &Value) -> (Vec<AgentCallRecord>, Vec<LlmProvenanceRecord>, Vec<[u8; 32]>, Option<[u8; 32]>) {
    let records: Vec<AgentCallRecord> = serde_json::from_value(payload["records"].clone()).unwrap();
    let n17: Vec<LlmProvenanceRecord> = serde_json::from_value(payload["llm_records"].clone()).unwrap();
    let leaves: Vec<[u8; 32]> = payload["n17_leaves"].as_array().unwrap().iter().filter_map(|v| unhex32(v.as_str().unwrap_or(""))).collect();
    let root = payload["n17_root"].as_str().and_then(unhex32);
    (records, n17, leaves, root)
}

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

/// `Uuid::from_u128` sans tirer l'horloge (déterministe).
fn uuid_from_u128(v: u128) -> uuid::Uuid {
    uuid::Uuid::from_u128(v)
}

/// **§8.1 ③ — coût à UN point de charge `n`** : octets/forensique sérialisée + latence de
/// `seal_n17` et `verify_run_offline` (RUNS itérations, WARMUP écarté). Renvoie
/// `(seal_us_per_run, verify_us_per_run, verify_us_per_record, bytes_per_record)`.
fn measure_cost(n: usize) -> (f64, f64, f64, f64) {
    let (records, n17, key) = synth_mode_c_run(n);
    let sealed = seal_n17(&n17, &key);
    // octets : la MÊME enveloppe persistée que le round-trip (records + forensique + feuilles + root).
    let payload = persist_payload(&records, &n17, &sealed.leaves_hex(), &sealed.root_hex());
    let bytes_per_record = serde_json::to_string(&payload).unwrap().len() as f64 / n.max(1) as f64;
    let mut seal_ns = Vec::with_capacity(RUNS);
    let mut verify_ns = Vec::with_capacity(RUNS);
    for i in 0..(RUNS + WARMUP) {
        let t0 = Instant::now();
        let s = seal_n17(&n17, &key);
        let d_seal = t0.elapsed().as_nanos() as f64;
        let t1 = Instant::now();
        let _ = verify_run_offline(&records, &n17, &s.leaf_hashes, s.n17_root, &key);
        let d_verify = t1.elapsed().as_nanos() as f64;
        if i >= WARMUP {
            seal_ns.push(d_seal);
            verify_ns.push(d_verify);
        }
    }
    let mean = |xs: &[f64]| xs.iter().sum::<f64>() / xs.len() as f64;
    let us = |ns: f64| ns / 1000.0;
    let seal_us = us(mean(&seal_ns));
    let verify_us = us(mean(&verify_ns));
    (seal_us, verify_us, verify_us / n.max(1) as f64, bytes_per_record)
}

fn main() {
    println!("══════════════════════════════════════════════════════════════════════════════");
    println!(" LOT N17PERSIST — gate ADR-016 : la forensique N17 survit-elle au process,");
    println!(" et se re-prouve-t-elle HORS LIGNE (essaim + chaîne N17 + pont) ?");
    println!(" crypto RÉELLE · seal_n17/verify_run_offline RÉELS · appels LLM SYNTHÉTIQUES");
    println!(" {MODE_C_CALLS} appels Mode C + 1 juge Mode A · aller-retour serde JSON · {RUNS} runs");
    println!("══════════════════════════════════════════════════════════════════════════════");

    // ── 1) un run Mode C synthétique représentatif (n = MODE_C_CALLS), scellé. ────
    let (records, n17, key) = synth_mode_c_run(MODE_C_CALLS);
    let sealed = seal_n17(&n17, &key);
    let leaves_hex = sealed.leaves_hex();
    let root_hex = sealed.root_hex();

    // Émet les DEUX exports tels quels (miroir de `GET /records` et `GET /n17`) : ce sont
    // exactement les fichiers qu'un tiers reçoit, puis re-prouve via `verify_offline`.
    let _ = fs::create_dir_all("scripts/swarm");
    let _ = fs::write(
        "scripts/swarm/n17_sample_records.json",
        serde_json::to_string_pretty(&records).unwrap(),
    );
    let _ = fs::write(
        "scripts/swarm/n17_sample_export.json",
        serde_json::to_string_pretty(&json!({
            "run_id": uuid_from_u128(0x5EED),
            "llm_records": &n17,
            "n17_leaves": &leaves_hex,
            "n17_root": &root_hex,
        }))
        .unwrap(),
    );

    // Émet AUSSI un export FALSIFIÉ (model_id muté, feuilles/racine d'ORIGINE conservées car
    // l'attaquant n'a pas la clé) : c'est l'intrant qui fait sortir le binaire `verify_offline`
    // en code 1 avec faute localisée. On CAPTURE le verdict que le binaire mappe en code de
    // sortie (0 sain / 1 falsifié) → la promesse §5.3/ADR devient reproductible ET enregistrée.
    let mut tampered_n17 = n17.clone();
    tampered_n17[0].model_identity = ModelIdentity::hosted("ollama", "MENTEUR:70b", "local");
    let _ = fs::write(
        "scripts/swarm/n17_sample_export.TAMPERED.json",
        serde_json::to_string_pretty(&json!({
            "run_id": uuid_from_u128(0x5EED),
            "llm_records": &tampered_n17,
            "n17_leaves": &leaves_hex, // feuilles d'ORIGINE (non re-scellées)
            "n17_root": &root_hex,
        }))
        .unwrap(),
    );
    let vo_healthy_proven = verify_run_offline(&records, &n17, &sealed.leaf_hashes, sealed.n17_root, &key).fully_proven;
    let vo_tampered = verify_run_offline(&records, &tampered_n17, &sealed.leaf_hashes, sealed.n17_root, &key);
    let vo_tampered_proven = vo_tampered.fully_proven;
    let vo_tampered_localized = vo_tampered.n17_chain.first_break.map(|p| p.as_uuid().to_string());
    // le verdict binaire-équivalent : sain → exit 0, falsifié → exit 1 (faute localisée).
    let verify_offline_ok = vo_healthy_proven && !vo_tampered_proven && vo_tampered_localized.is_some();

    // ── 2) ALLER-RETOUR de persistance (I-PN1) : sérialise → jette → relit → re-prouve. ──
    let payload = persist_payload(&records, &n17, &leaves_hex, &root_hex);
    let payload_str = serde_json::to_string(&payload).unwrap();
    let payload_bytes = payload_str.len();
    drop(records);
    drop(n17); // le process « meurt » : plus aucun objet vivant, rien que des octets.

    let reloaded: Value = serde_json::from_str(&payload_str).unwrap();
    let (r_records, r_n17, r_leaves, r_root) = reload(&reloaded);
    let n17_count = r_n17.len();
    let v = verify_run_offline(&r_records, &r_n17, &r_leaves, r_root, &key);
    let round_trip_proven = v.fully_proven;
    let bytes_per_record = if n17_count > 0 { payload_bytes as f64 / n17_count as f64 } else { 0.0 };

    // ── 3) MODE A : un run keyless (0 forensique) est trivialement re-prouvé. ─────
    let mode_a_proven = {
        let (recs, _n17, _k) = {
            // run Mode A pur : pas d'appel LLM, donc 0 forensique.
            let tenant = TenantId::system();
            let swarm = SwarmId::from_uuid(uuid_from_u128(0xA0A0));
            let k = tenant_key(tenant);
            let mut chain = SwarmChain::new(tenant, swarm, k.clone());
            let recs = chain.append_all(std::iter::once(AgentCallDraft::new(
                swarm, tenant, AgentId::new("judge").unwrap(), AgentRole::Judge,
                AgentKind::ModeA, Blake3Hash::from_bytes([1u8; 32]), Blake3Hash::from_bytes([2u8; 32]),
            )));
            (recs, Vec::<LlmProvenanceRecord>::new(), k)
        };
        let sealed_a = seal_n17(&[], &key);
        let va = verify_run_offline(&recs, &[], &sealed_a.leaf_hashes, sealed_a.n17_root, &key);
        va.fully_proven && va.n17_chain.intact && va.bridge.all_mode_c_consistent() && sealed_a.n17_root.is_none()
    };

    // ── 4) FALSIFICATIONS (4 vecteurs) — toutes doivent être détectées. ──────────
    // (a) §8.1 ② : muter CHAQUE champ forensique sensible (hash prompt, modèle, graine,
    //     output_hash) d'un record N17 relu → la chaîne N17 casse ET localise (first_break ==
    //     provenance muté). On rejoue un reload FRAIS par champ.
    #[allow(clippy::type_complexity)]
    let field_mutations: Vec<(&str, Box<dyn Fn(&mut LlmProvenanceRecord)>)> = vec![
        ("user_prompt_hash", Box::new(|r: &mut LlmProvenanceRecord| r.user_prompt_hash = Blake3Hash::from_bytes([0x11; 32]))),
        ("model_id", Box::new(|r| r.model_identity = ModelIdentity::hosted("ollama", "MENTEUR:70b", "local"))),
        ("seed", Box::new(|r| r.sampling.seed = Some(0xDEAD))),
        ("output_hash", Box::new(|r| r.output_hash = Some(Blake3Hash::from_bytes([0x22; 32])))),
    ];
    let mut field_results: Vec<(&str, bool, bool)> = Vec::with_capacity(field_mutations.len());
    for (label, mutate) in &field_mutations {
        let (rr, mut rn, rl, rt) = reload(&serde_json::from_str::<Value>(&payload_str).unwrap());
        let victim = rn[2].provenance_id;
        mutate(&mut rn[2]);
        let vt = verify_run_offline(&rr, &rn, &rl, rt, &key);
        field_results.push((label, !vt.fully_proven && !vt.n17_chain.intact, vt.n17_chain.first_break == Some(victim)));
    }
    let t_field_detected = field_results.iter().all(|(_, d, _)| *d);
    let t_field_localized = field_results.iter().all(|(_, _, l)| *l);
    // (b) muter un record d'ESSAIM (Mode C) relu → chaîne d'essaim casse.
    let t_swarm_detected = {
        let (mut rr, rn, rl, rt) = reload(&serde_json::from_str::<Value>(&payload_str).unwrap());
        rr[0].output_hash = Blake3Hash::from_bytes([0xff; 32]);
        let vt = verify_run_offline(&rr, &rn, &rl, rt, &key);
        !vt.fully_proven && !vt.swarm.content_intact
    };
    // (c) forger une FEUILLE scellée (sans la clé) → racine N17 ne recolle pas.
    let t_leaf_detected = {
        let (rr, rn, mut rl, rt) = reload(&serde_json::from_str::<Value>(&payload_str).unwrap());
        if let Some(first) = rl.first_mut() {
            first[0] ^= 0xff;
        }
        let vt = verify_run_offline(&rr, &rn, &rl, rt, &key);
        !vt.fully_proven && !vt.n17_chain.intact
    };
    // (d) RETIRER un record N17 (compte forensique falsifié) → CountMismatch.
    let t_drop_detected = {
        let (rr, mut rn, rl, rt) = reload(&serde_json::from_str::<Value>(&payload_str).unwrap());
        rn.pop();
        let vt = verify_run_offline(&rr, &rn, &rl, rt, &key);
        !vt.fully_proven && !vt.n17_chain.intact
    };

    // ── 5) COÛT (§8.1 ③) : BALAYAGE de charge N ∈ {1, 10, 100} appels — la COURBE, pas un
    //    point. Confirme la linéarité O(records) et que le budget tient jusqu'à N=100.
    //    Chaque point : (n, seal_us/run, verify_us/run, verify_us/record, octets/forensique).
    let sweep: Vec<(usize, f64, f64, f64, f64)> = N_SWEEP
        .iter()
        .map(|&n| {
            let (seal_us, verify_us, vpr, bpr) = measure_cost(n);
            (n, seal_us, verify_us, vpr, bpr)
        })
        .collect();
    // point de référence pour le résumé (le plus grand N = le pire cas de coût/record).
    let (_, seal_us, verify_us, verify_us_per_record, _) = *sweep.last().unwrap();

    // ── verdict. ─────────────────────────────────────────────────────────────────
    let all_tamper = t_field_detected && t_field_localized && t_swarm_detected && t_leaf_detected && t_drop_detected;
    // cost_ok : octets/forensique ET latence/record sous budget à CHAQUE point de charge.
    let cost_ok = bytes_per_record <= BYTES_PER_RECORD_BUDGET
        && sweep.iter().all(|(_, _, _, vpr, bpr)| *vpr <= VERIFY_US_PER_RECORD_BUDGET && *bpr <= BYTES_PER_RECORD_BUDGET);
    let go = round_trip_proven && mode_a_proven && all_tamper && verify_offline_ok && cost_ok;
    let verdict = if go { "GO ✅ — la forensique N17 est durable, re-prouvable hors ligne, et infalsifiable" } else { "NO-GO ❌" };

    println!("\n ALLER-RETOUR (I-PN1) — survie à la mort du process :");
    println!("   forensique persistée  : {n17_count} records N17 (Mode C) + records d'essaim");
    println!("   payload sérialisé     : {payload_bytes} o  ({bytes_per_record:.0} o / forensique, budget {BYTES_PER_RECORD_BUDGET:.0} o)");
    println!("   re-preuve COMPLÈTE    : fully_proven={round_trip_proven}  (essaim ok={} · N17 intacte={} · pont cohérent={})",
        v.swarm.ok(), v.n17_chain.intact, v.bridge.all_mode_c_consistent());
    println!("   pont : Mode C cohérents={} · non liés={} · rompus={}", v.bridge.consistent, v.bridge.unlinked, v.bridge.broken);
    println!("\n MODE A (no-op) : keyless trivialement re-prouvé = {mode_a_proven}");
    println!("\n FALSIFICATIONS (toutes doivent être détectées) :");
    println!("   (a) CHAQUE champ forensique muté (§8.1 ②) :");
    for (label, d, l) in &field_results {
        println!("        {label:<18} détecté={d} · LOCALISÉ={l}");
    }
    println!("   (b) record d'essaim muté  : détecté={t_swarm_detected}");
    println!("   (c) feuille N17 forgée    : détecté={t_leaf_detected}");
    println!("   (d) record N17 retiré     : détecté={t_drop_detected}");
    println!("\n VÉRIFICATEUR TIERS (verify_offline, verdict → code de sortie) :");
    println!("   export sain      → prouvé={vo_healthy_proven} (exit 0)");
    println!("   export falsifié  → prouvé={vo_tampered_proven} (exit 1) · localisé={}", vo_tampered_localized.as_deref().unwrap_or("∅"));
    println!("\n COÛT (§8.1 ③ — balayage N ∈ {{1,10,100}}, {RUNS} runs/point) :");
    println!("   {:<5} {:>12} {:>14} {:>16} {:>16}", "N", "seal µs/run", "verify µs/run", "verify µs/rec", "octets/rec");
    for (n, s, vr, vpr, bpr) in &sweep {
        println!("   {n:<5} {s:>12.1} {vr:>14.1} {vpr:>16.1} {bpr:>16.0}");
    }
    println!("   budgets : ≤ {VERIFY_US_PER_RECORD_BUDGET:.0} µs/record · ≤ {BYTES_PER_RECORD_BUDGET:.0} o/forensique (tenus à chaque N)");

    println!("\n────────────────────────── VERDICT LOT N17PERSIST ──────────────────────────");
    println!(" {verdict}");
    println!("────────────────────────────────────────────────────────────────────────────");

    // ── persiste l'artefact (format des Lots P4/P5). ─────────────────────────────
    // détail par-champ de la falsification forensique (§8.1 ②) et courbe de coût (§8.1 ③).
    let forensic_fields: serde_json::Map<String, Value> = field_results
        .iter()
        .map(|(label, d, l)| (label.to_string(), json!({ "detected": d, "localized": l })))
        .collect();
    let sweep_json: Vec<Value> = sweep
        .iter()
        .map(|(n, s, vr, vpr, bpr)| json!({
            "n": n,
            "seal_us_per_run": s,
            "verify_offline_us_per_run": vr,
            "verify_offline_us_per_record": vpr,
            "bytes_per_record": bpr,
        }))
        .collect();
    let payload = json!({
        "_note": "Appels LLM SYNTHÉTIQUES (forensique fabriquée) ; crypto (BLAKE3+SHA256+HMAC+HKDF) + seal_n17/verify_run_offline RÉELS. Persistance éprouvée par aller-retour serde JSON (frontière JSONB du store).",
        "mode_c_calls": MODE_C_CALLS,
        "round_trip": {
            "n17_records": n17_count,
            "payload_bytes": payload_bytes,
            "bytes_per_record": bytes_per_record,
            "fully_proven": round_trip_proven,
            "swarm_ok": v.swarm.ok(),
            "n17_chain_intact": v.n17_chain.intact,
            "bridge_consistent": v.bridge.all_mode_c_consistent(),
            "bridge": { "consistent": v.bridge.consistent, "unlinked": v.bridge.unlinked, "broken": v.bridge.broken }
        },
        "mode_a_trivially_proven": mode_a_proven,
        "tamper_detection": {
            "forensic_field_mutated_detected": t_field_detected,
            "forensic_field_mutated_localized": t_field_localized,
            "forensic_fields": Value::Object(forensic_fields),
            "swarm_record_mutated_detected": t_swarm_detected,
            "leaf_forged_detected": t_leaf_detected,
            "n17_record_dropped_detected": t_drop_detected,
            "all_detected": all_tamper
        },
        "verify_offline_cli": {
            "healthy_proven": vo_healthy_proven,
            "tampered_model_id_proven": vo_tampered_proven,
            "tampered_localized_provenance": vo_tampered_localized,
            "ok": verify_offline_ok,
            "_note": "verdict mappé par le binaire verify_offline en code de sortie : sain→0, falsifié→1 (faute localisée). Export falsifié émis : scripts/swarm/n17_sample_export.TAMPERED.json"
        },
        "cost": {
            "seal_us_per_run": seal_us,
            "verify_offline_us_per_run": verify_us,
            "verify_offline_us_per_record": verify_us_per_record,
            "sweep": sweep_json
        },
        "budgets": { "bytes_per_record": BYTES_PER_RECORD_BUDGET, "verify_us_per_record": VERIFY_US_PER_RECORD_BUDGET },
        "verdict": verdict,
        "go": go
    });
    let _ = fs::create_dir_all("scripts/swarm");
    let path = "scripts/swarm/EFE_SWARM_N17PERSIST_2026-06-15.json";
    let _ = fs::write(path, serde_json::to_string_pretty(&payload).unwrap());
    println!("\n✅ Résultats : {path}");
    assert!(go, "GATE ADR-016 : la persistance/re-preuve N17 doit tenir");
}
