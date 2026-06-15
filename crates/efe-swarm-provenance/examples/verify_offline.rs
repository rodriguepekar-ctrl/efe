// SPDX-License-Identifier: UNLICENSED
//! # `verify_offline` — le **vérificateur tiers** d'ADR-016 : re-prouver un run **hors
//! ligne**, sans serveur, sans provider, sans réseau.
//!
//! C'est l'incarnation du *moat* étendu à la forensique LLM : un tiers reçoit deux
//! fichiers — l'export `GET /v1/swarm/runs/:id/records` (les `AgentCallRecord`) et l'export
//! `GET /v1/swarm/runs/:id/n17` (la forensique `LlmProvenanceRecord` + feuilles + `n17_root`)
//! — et, avec **sa propre copie de la clé tenant**, recompose TOUT :
//!   1. la chaîne d'essaim (contenu + DAG + HMAC),
//!   2. la chaîne forensique N17 (la forensique se rescelle-t-elle exactement ?),
//!   3. le pont Mode A ↔ Mode C (chaque appel LLM résout-il en cohérent ?).
//!
//! Il ne *croit* ni le serveur ni le store : il **re-prouve**. Code de sortie `0` ⇔ tout
//! est re-prouvé ; `1` sinon (et la 1ʳᵉ faute est localisée).
//!
//! **Clé** : ici on dérive la clé via `HmacKeySource::test_key()` (la source de DEV, comme
//! la passerelle hors `EFE_NANO_AUDIT_HMAC_MASTER`). En prod, le tiers branche sa copie du
//! master HKDF ; la mécanique de re-preuve est identique — seule la source de clé change.
//!
//! Usage :
//!   cargo run --release --example verify_offline -p efe-swarm-provenance -- \
//!       scripts/swarm/n17_sample_records.json scripts/swarm/n17_sample_export.json [tenant]
//!
//! (`tenant` par défaut : `system`. Les fichiers d'exemple sont produits par `lot_n17persist`.)

use std::process::ExitCode;
use std::str::FromStr;
use std::{env, fs};

use efe_nano_sdk::{LlmProvenanceRecord, TenantId};
use efe_swarm_provenance::{verify_run_offline, AgentCallRecord, HmacKeySource};
use serde::Deserialize;

/// L'export `/n17` (le miroir de `N17Export` côté passerelle).
#[derive(Deserialize)]
struct N17Export {
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    llm_records: Vec<LlmProvenanceRecord>,
    #[serde(default)]
    n17_leaves: Vec<String>,
    #[serde(default)]
    n17_root: Option<String>,
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

/// `tenant` (chaîne) → `TenantId` : `"system"` (ou l'UUID système) → `TenantId::system()`.
fn tenant_of(s: &str) -> Result<TenantId, String> {
    if s == "system" {
        return Ok(TenantId::system());
    }
    TenantId::from_str(s).map_err(|_| format!("tenant invalide : {s}"))
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: verify_offline <records.json> <n17.json> [tenant=system]");
        eprintln!("  (produire les exemples : cargo run --release --example lot_n17persist -p efe-swarm-provenance)");
        return ExitCode::from(2);
    }
    let records_path = &args[1];
    let n17_path = &args[2];
    let tenant_str = args.get(3).map(String::as_str).unwrap_or("system");

    // ── 1) lire les deux exports (les seuls intrants : aucun serveur, aucun réseau). ──
    let records: Vec<AgentCallRecord> = match fs::read_to_string(records_path)
        .map_err(|e| e.to_string())
        .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("lecture {records_path} : {e}");
            return ExitCode::from(2);
        }
    };
    let n17: N17Export = match fs::read_to_string(n17_path)
        .map_err(|e| e.to_string())
        .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("lecture {n17_path} : {e}");
            return ExitCode::from(2);
        }
    };

    // ── 2) dériver la clé tenant (source DEV ; en prod : copie du master HKDF). ───────
    let tenant = match tenant_of(tenant_str) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let key = match HmacKeySource::test_key().derive(tenant) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("dérivation de clé : {e}");
            return ExitCode::from(2);
        }
    };

    // ── 3) décoder les feuilles/racine N17, puis RE-PROUVER tout, hors ligne. ────────
    let leaves: Vec<[u8; 32]> = n17.n17_leaves.iter().filter_map(|h| unhex32(h)).collect();
    let root = n17.n17_root.as_deref().and_then(unhex32);

    let v = verify_run_offline(&records, &n17.llm_records, &leaves, root, &key);

    println!("══════════════════════════════════════════════════════════════════════");
    println!(" VÉRIFICATION HORS LIGNE — run {}", n17.run_id.as_deref().unwrap_or("?"));
    println!(" tenant={tenant_str}  ·  records d'essaim={}  ·  forensique N17={}", records.len(), n17.llm_records.len());
    println!("══════════════════════════════════════════════════════════════════════");
    println!(" 1) ESSAIM   : ok={}  (chaîne={} · contenu={} · DAG={})  root={}",
        v.swarm.ok(), v.swarm.chain_intact, v.swarm.content_intact, v.swarm.dag_consistent,
        v.swarm.root_hex().as_deref().unwrap_or("∅"));
    println!(" 2) N17      : intacte={}  records={}  root={}",
        v.n17_chain.intact, v.n17_chain.records, v.n17_chain.root_hex().as_deref().unwrap_or("∅"));
    println!(" 3) PONT     : cohérent={}  (Mode C cohérents={} · non liés={} · rompus={})",
        v.bridge.all_mode_c_consistent(), v.bridge.consistent, v.bridge.unlinked, v.bridge.broken);
    if let Some(b) = v.swarm.first_break {
        println!("    ⚠ 1ʳᵉ faute d'essaim localisée : record {b}");
    }
    if let Some(b) = v.n17_chain.first_break {
        println!("    ⚠ 1ʳᵉ faute forensique localisée : provenance {b}");
    }
    println!("──────────────────────────────────────────────────────────────────────");
    if v.fully_proven {
        println!(" ✅ PROUVÉ — essaim + forensique N17 + pont recomposés exactement.");
        ExitCode::SUCCESS
    } else {
        println!(" ❌ NON PROUVÉ — au moins une chaîne ne recompose pas (faute localisée ci-dessus).");
        ExitCode::FAILURE
    }
}
