//! Sprint **D14-bis** — `efe-verify-audit` CLI.
//!
//! Tamper-evidence verifier for the `audit_events` table.
//!
//! # Synopsis
//!
//! ```text
//! efe-verify-audit \
//!     --pg-url <postgres://...> \
//!     --keys-file <path> \
//!     [--chain <chain_id>] \
//!     [--json]
//! ```
//!
//! # Options
//!
//! * `--pg-url` (required) — Postgres DSN. Reads
//!   `EFE_PG_TEST_URL` env if absent.
//! * `--keys-file` (required) — path to the HMAC keys file (1 line
//!   = 1 base64 key, 32 raw bytes each). Same format as
//!   `EFE_AUDIT_HMAC_SECRETS_FILE`.
//! * `--chain <chain_id>` — verify only this chain
//!   (e.g. `tenant:acme`). Default : verify every distinct
//!   `chain_id` in the table.
//! * `--json` — emit a JSON report instead of TTY-colourised output.
//!   Used by CI / regulator tooling.
//!
//! # Exit codes
//!
//! * `0` — every chain verified intact end-to-end.
//! * `1` — at least one chain has a break (tampering / deletion /
//!   reordering detected) ; report contains the position.
//! * `2` — operational error (DB unreachable, malformed keys file,
//!   etc.). Distinct from `1` so CI can differentiate "audit
//!   compromised" from "verifier malfunction".
//!
//! # Output formats
//!
//! Default TTY :
//!
//! ```text
//! Verifying chain tenant:acme...
//! ✓ 12 894 events verified
//! ✓ chain intact through seq 89 422
//! ✓ keys used : abcdef01, 12345678
//! ```
//!
//! `--json` :
//!
//! ```jsonc
//! {
//!   "chains": [
//!     { "chain_id": "tenant:acme", "n_events": 12894,
//!       "first_break_seq": null, "last_seq_verified": 89422,
//!       "keys_used": ["abcdef01", "12345678"] }
//!   ],
//!   "summary": { "intact": true, "n_chains": 1 }
//! }
//! ```

use std::path::Path;
use std::process::ExitCode;

use efe_server::catalog::{audit_hmac::verify_chain, audit_keys::load_keys_file};
use sqlx::postgres::PgPoolOptions;
use sqlx::Row;

#[tokio::main]
async fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("efe-verify-audit: {}", e);
            print_usage();
            return ExitCode::from(2);
        }
    };

    // Load keys.
    let loaded = match load_keys_file(Path::new(&args.keys_file)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error loading keys file: {e}");
            return ExitCode::from(2);
        }
    };

    // Connect to PG.
    let pool = match PgPoolOptions::new()
        .max_connections(2)
        .connect(&args.pg_url)
        .await
    {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error connecting to PG: {e}");
            return ExitCode::from(2);
        }
    };

    // Determine which chains to verify.
    let chains: Vec<String> = if let Some(c) = &args.chain {
        vec![c.clone()]
    } else {
        match list_distinct_chains(&pool).await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("error listing chains: {e}");
                return ExitCode::from(2);
            }
        }
    };

    if chains.is_empty() {
        if args.json {
            println!(r#"{{"chains":[],"summary":{{"intact":true,"n_chains":0}}}}"#);
        } else {
            println!("no chains found in audit_events ; nothing to verify");
        }
        return ExitCode::SUCCESS;
    }

    let mut reports: Vec<efe_server::catalog::VerifyReport> = Vec::new();
    let mut any_break = false;

    for chain_id in &chains {
        match verify_chain(&pool, chain_id, &loaded.all).await {
            Ok(rep) => {
                if rep.first_break_seq.is_some() {
                    any_break = true;
                }
                reports.push(rep);
            }
            Err(e) => {
                eprintln!("verify error on chain {chain_id}: {e}");
                return ExitCode::from(2);
            }
        }
    }

    if args.json {
        emit_json(&reports);
    } else {
        emit_tty(&reports);
    }
    if any_break {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

// =====================================================================
// Args parsing — minimal (no clap dep)
// =====================================================================

struct Args {
    pg_url: String,
    keys_file: String,
    chain: Option<String>,
    json: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut pg_url: Option<String> = std::env::var("EFE_PG_TEST_URL").ok();
    let mut keys_file: Option<String> = None;
    let mut chain: Option<String> = None;
    let mut json = false;
    let mut iter = std::env::args().skip(1);
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--pg-url" => {
                pg_url = Some(iter.next().ok_or("--pg-url requires a value".to_string())?);
            }
            "--keys-file" => {
                keys_file = Some(
                    iter.next()
                        .ok_or("--keys-file requires a value".to_string())?,
                );
            }
            "--chain" => {
                chain = Some(iter.next().ok_or("--chain requires a value".to_string())?);
            }
            "--json" => json = true,
            "-h" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    let pg_url = pg_url.ok_or("--pg-url is required (or set EFE_PG_TEST_URL)")?;
    let keys_file = keys_file.ok_or("--keys-file is required")?;
    Ok(Args {
        pg_url,
        keys_file,
        chain,
        json,
    })
}

fn print_usage() {
    eprintln!(
        "Usage: efe-verify-audit \
         --pg-url <DSN> \
         --keys-file <PATH> \
         [--chain <chain_id>] \
         [--json]"
    );
    eprintln!();
    eprintln!("Exit codes:");
    eprintln!("  0  every chain intact");
    eprintln!("  1  tampering detected (one or more chains broken)");
    eprintln!("  2  operational error (DB unreachable, bad keys file, …)");
}

async fn list_distinct_chains(pool: &sqlx::PgPool) -> Result<Vec<String>, sqlx::Error> {
    let rows = sqlx::query("SELECT DISTINCT chain_id FROM audit_events ORDER BY chain_id")
        .fetch_all(pool)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let id: String = row.try_get("chain_id")?;
        out.push(id);
    }
    Ok(out)
}

// =====================================================================
// Output formats
// =====================================================================

fn emit_tty(reports: &[efe_server::catalog::VerifyReport]) {
    for r in reports {
        println!("Verifying chain {}…", r.chain_id);
        if let Some(brk) = r.first_break_seq {
            println!("  ✗ chain BREAK at seq {brk}");
            println!("    audit integrity compromised at this position");
        } else {
            println!("  ✓ {} events verified", r.n_events);
            if let Some(last) = r.last_seq_verified {
                println!("  ✓ chain intact through seq {last}");
            }
            if !r.keys_used.is_empty() {
                println!("  ✓ keys used : {}", r.keys_used.join(", "));
            }
        }
    }
}

fn emit_json(reports: &[efe_server::catalog::VerifyReport]) {
    let any_break = reports.iter().any(|r| r.first_break_seq.is_some());
    let n = reports.len();
    print!(r#"{{"chains":["#);
    for (i, r) in reports.iter().enumerate() {
        if i > 0 {
            print!(",");
        }
        let break_str = r
            .first_break_seq
            .map(|s| s.to_string())
            .unwrap_or_else(|| "null".into());
        let last_str = r
            .last_seq_verified
            .map(|s| s.to_string())
            .unwrap_or_else(|| "null".into());
        let keys_str = r
            .keys_used
            .iter()
            .map(|k| format!("\"{k}\""))
            .collect::<Vec<_>>()
            .join(",");
        print!(
            r#"{{"chain_id":"{}","n_events":{},"first_break_seq":{},"last_seq_verified":{},"keys_used":[{}]}}"#,
            escape_json(&r.chain_id),
            r.n_events,
            break_str,
            last_str,
            keys_str
        );
    }
    print!(
        r#"],"summary":{{"intact":{intact},"n_chains":{n}}}}}"#,
        intact = !any_break,
        n = n,
    );
    println!();
}

fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}
