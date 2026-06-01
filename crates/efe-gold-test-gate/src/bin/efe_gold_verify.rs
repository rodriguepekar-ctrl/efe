// SPDX-License-Identifier: UNLICENSED
//! **Sprint 11 — N19** : `efe-gold-verify` CLI.
//!
//! Architecture A4.4 — walks a gold-run ledger for a given
//! `agent_id` and verifies the HMAC chain.

use std::process::ExitCode;

use efe_gold_test_gate::{
    GoldRunLedger, GoldRunLedgerRow, InMemoryGoldRunLedger, CRATE_VERSION,
};
use efe_nano_sdk::NanoAgentId;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct LedgerDump {
    rows: Vec<GoldRunLedgerRow>,
    hmac_key_hex: String,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        print_help();
        return ExitCode::from(1);
    }
    match args[1].as_str() {
        "--version" | "-V" => {
            println!("efe-gold-verify {CRATE_VERSION}");
            ExitCode::SUCCESS
        }
        "--help" | "-h" | "help" => {
            print_help();
            ExitCode::SUCCESS
        }
        "ledger" => match cmd_ledger_inmemory(&args[2..]).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(&e, 2),
        },
        "ledger-from-file" => match cmd_ledger_from_file(&args[2..]).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(&e, 2),
        },
        other => {
            eprintln!("efe-gold-verify : unknown subcommand `{other}` — see --help");
            ExitCode::from(1)
        }
    }
}

fn fail(msg: &str, code: u8) -> ExitCode {
    eprintln!("efe-gold-verify : {msg}");
    ExitCode::from(code)
}

fn print_help() {
    println!(
        "efe-gold-verify — N19 gold-run ledger chain integrity verifier.\n\n\
         USAGE:\n\
         \tefe-gold-verify ledger <agent_id>\n\
         \tefe-gold-verify ledger-from-file PATH.json\n\
         \tefe-gold-verify --version\n\
         \tefe-gold-verify help\n\n\
         EXIT CODES:\n\
         \t0  : chain verifies clean.\n\
         \t1  : usage error.\n\
         \t2  : chain broken or verification failed.\n"
    );
}

async fn cmd_ledger_inmemory(args: &[String]) -> Result<(), String> {
    let id_str = args.first().ok_or("agent_id required")?;
    let id = NanoAgentId::new(id_str).map_err(|e| e.to_string())?;
    let ledger = InMemoryGoldRunLedger::new([0u8; 32]);
    let report = ledger.verify_chain(&id).await.map_err(|e| e.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
    );
    if !report.ok {
        return Err("chain broken".into());
    }
    Ok(())
}

async fn cmd_ledger_from_file(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or("PATH.json required")?;
    let bytes = std::fs::read(path).map_err(|e| format!("read {path} : {e}"))?;
    let dump: LedgerDump = serde_json::from_slice(&bytes)
        .map_err(|e| format!("parse {path} : {e}"))?;
    let key = hex_decode(&dump.hmac_key_hex)?;
    if key.len() < 32 {
        return Err(format!(
            "HMAC key must be ≥ 32 bytes (got {})",
            key.len()
        ));
    }
    let ledger = InMemoryGoldRunLedger::with_seed_rows(key, dump.rows.clone())?;
    let agents: std::collections::HashSet<NanoAgentId> =
        dump.rows.iter().map(|r| r.agent_id.clone()).collect();
    let mut overall_ok = true;
    for a in agents {
        let report = ledger.verify_chain(&a).await.map_err(|e| e.to_string())?;
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
        overall_ok &= report.ok;
    }
    if !overall_ok {
        return Err("at least one chain failed verification".into());
    }
    Ok(())
}

fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    if s.len() % 2 != 0 {
        return Err(format!("odd hex length {}", s.len()));
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi = hex_digit(bytes[i])?;
        let lo = hex_digit(bytes[i + 1])?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Ok(out)
}

fn hex_digit(c: u8) -> Result<u8, String> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        _ => Err(format!("invalid hex char : {c}")),
    }
}
