// SPDX-License-Identifier: UNLICENSED
//! **Sprint 10 — N18** : `efe-publisher-verify` CLI.
//!
//! Architecture A4.2.
//!
//! Walks a publish ledger for a given `publisher_id` and verifies the
//! HMAC chain integrity end-to-end.  Designed to be runnable
//! offline against a JSON dump of the ledger (Sprint 11 wires a
//! Postgres source).
//!
//! ```text
//! efe-publisher-verify ledger <publisher_id>           # in-memory demo
//! efe-publisher-verify ledger-from-file PATH.json      # offline forensic
//! efe-publisher-verify --version
//! ```
//!
//! Exit codes :
//!   - 0  : chain verifies clean.
//!   - 1  : usage error.
//!   - 2  : chain broken / verification failed.

use std::process::ExitCode;

use efe_nano_sdk::PublisherId;
use efe_publisher_identity::{
    InMemoryPublishLedger, LedgerRow, PublishLedger, CRATE_VERSION,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct LedgerDump {
    rows: Vec<LedgerRow>,
    /// HMAC key for the chain (hex-encoded ; must match the producer).
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
            println!("efe-publisher-verify {CRATE_VERSION}");
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
            eprintln!("efe-publisher-verify : unknown subcommand `{other}` ; see --help");
            ExitCode::from(1)
        }
    }
}

fn fail(msg: &str, code: u8) -> ExitCode {
    eprintln!("efe-publisher-verify : {msg}");
    ExitCode::from(code)
}

fn print_help() {
    println!(
        "efe-publisher-verify — N18 publish-ledger chain integrity verifier.\n\n\
         USAGE:\n\
         \tefe-publisher-verify ledger <publisher_id>\n\
         \tefe-publisher-verify ledger-from-file PATH.json\n\
         \tefe-publisher-verify --version\n\
         \tefe-publisher-verify help\n\n\
         EXIT CODES:\n\
         \t0  : chain verifies clean.\n\
         \t1  : usage error.\n\
         \t2  : chain broken or verification failed.\n"
    );
}

async fn cmd_ledger_inmemory(args: &[String]) -> Result<(), String> {
    let id_str = args.first().ok_or("publisher_id required")?;
    let uuid = uuid::Uuid::parse_str(id_str).map_err(|e| format!("bad publisher_id : {e}"))?;
    let publisher_id = PublisherId::from_uuid(uuid);
    // In-memory ledger is empty here — useful for smoke-testing the
    // tool's plumbing.  Real forensic runs use `ledger-from-file`.
    let ledger = InMemoryPublishLedger::new([0u8; 32]);
    let report = ledger
        .verify_chain(publisher_id)
        .await
        .map_err(|e| e.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
    );
    if !report.ok {
        return Err("chain integrity broken".into());
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
            "HMAC key must be ≥ 32 bytes (got {} bytes)",
            key.len()
        ));
    }
    // Rebuild an in-memory ledger seeded with the dumped rows.
    let ledger = InMemoryPublishLedger::with_seed_rows(key, dump.rows.clone())?;
    let publisher_ids: std::collections::HashSet<_> =
        dump.rows.iter().map(|r| r.publisher_id).collect();
    let mut overall_ok = true;
    for pid in publisher_ids {
        let report = ledger
            .verify_chain(pid)
            .await
            .map_err(|e| e.to_string())?;
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
        let h = bytes[i];
        let l = bytes[i + 1];
        let hi = hex_digit(h)?;
        let lo = hex_digit(l)?;
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
