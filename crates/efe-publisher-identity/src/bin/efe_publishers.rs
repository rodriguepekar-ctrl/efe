// SPDX-License-Identifier: UNLICENSED
//! **Sprint 10 — N18** : `efe-publishers` CLI.
//!
//! Architecture A4.1, A4.3.
//!
//! A minimal command-line interface for operator workflows that don't
//! warrant the full HTTP API surface.  Sprint 10 ships sub-commands :
//!
//! ```text
//! efe-publishers register --display-name NAME --tier {0|1|2|3} [--pseudonymous]
//! efe-publishers list [--tier-floor {0|1|2|3}]
//! efe-publishers lookup <publisher_id>
//! efe-publishers revoke <publisher_id> --reason {key_compromise|sanctions_violation|policy_violation|publisher_request|operator} [--suspected-compromise-at RFC3339] [--rationale RATIONALE]
//! efe-publishers sanctions-check <publisher_id>
//! efe-publishers promote <publisher_id> --to-tier {2|3} --approvers OPERATOR_ID,...
//! efe-publishers history <publisher_id> [--limit 50]
//! efe-publishers help
//! ```
//!
//! The CLI uses an **in-memory** registry / ledger by default for
//! Sprint 10 — production deployments (Sprint 11) will inject a
//! Postgres-backed pair via environment variables.  The output is
//! always JSON ; downstream tooling can `jq` the result.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::SystemTime;

use efe_nano_sdk::{
    Iso3166, KycTier, NewPublisherCandidate, OperatorSignature, PublisherId, RevocationReason,
    SignatureAlgorithm,
};
use efe_publisher_identity::{
    DefaultPublisherApi, Ed25519Verifier, HistoryQuery, InMemoryPublishLedger,
    InMemoryPublisherRegistry, ListQuery, N18Metrics, PromoteRequest, PublisherApi,
    RegisterRequest, RevokeRequest, StubKycProvider, StubSanctionsScreener,
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        print_help();
        return ExitCode::from(1);
    }
    match args[1].as_str() {
        "help" | "--help" | "-h" => {
            print_help();
            ExitCode::SUCCESS
        }
        "register" => match cmd_register(&args[2..]).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(&e),
        },
        "list" => match cmd_list(&args[2..]).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(&e),
        },
        "lookup" => match cmd_lookup(&args[2..]).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(&e),
        },
        "revoke" => match cmd_revoke(&args[2..]).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(&e),
        },
        "sanctions-check" => match cmd_sanctions_check(&args[2..]).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(&e),
        },
        "promote" => match cmd_promote(&args[2..]).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(&e),
        },
        "history" => match cmd_history(&args[2..]).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(&e),
        },
        other => {
            eprintln!("efe-publishers : unknown subcommand `{other}` ; see --help");
            ExitCode::from(2)
        }
    }
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("efe-publishers : {msg}");
    ExitCode::from(1)
}

fn print_help() {
    println!(
        "efe-publishers — operator CLI for the N18 publisher registry.\n\n\
        USAGE:\n  efe-publishers <subcommand> [options]\n\n\
        SUBCOMMANDS:\n\
        \tregister --display-name NAME --tier {{0|1|2|3}} [--pseudonymous]\n\
        \tlist [--tier-floor {{0|1|2|3}}]\n\
        \tlookup <publisher_id>\n\
        \trevoke <publisher_id> --reason REASON [--suspected-compromise-at RFC3339] [--rationale TEXT]\n\
        \tsanctions-check <publisher_id>\n\
        \tpromote <publisher_id> --to-tier {{1|2|3}} [--approvers N]\n\
        \thistory <publisher_id> [--limit 50]\n\
        \thelp\n\n\
        OUTPUT: JSON on stdout, errors on stderr.\n"
    );
}

fn build_api() -> DefaultPublisherApi {
    DefaultPublisherApi::new(
        Arc::new(InMemoryPublisherRegistry::new()),
        Arc::new(InMemoryPublishLedger::new([0x77u8; 32])),
        Arc::new(Ed25519Verifier::new()),
        Arc::new(StubKycProvider::permissive()),
        Arc::new(StubSanctionsScreener::clear()),
        N18Metrics::new(),
    )
}

fn parse_tier(s: &str) -> Result<KycTier, String> {
    match s {
        "0" | "tier_0" | "tier0" => Ok(KycTier::Tier0),
        "1" | "tier_1" | "tier1" => Ok(KycTier::Tier1),
        "2" | "tier_2" | "tier2" => Ok(KycTier::Tier2),
        "3" | "tier_3" | "tier3" => Ok(KycTier::Tier3),
        other => Err(format!("unknown tier `{other}`")),
    }
}

fn parse_reason(s: &str) -> Result<RevocationReason, String> {
    match s {
        "key_compromise" => Ok(RevocationReason::KeyCompromise),
        "sanctions_violation" => Ok(RevocationReason::SanctionsViolation),
        "policy_violation" => Ok(RevocationReason::PolicyViolation),
        "publisher_request" => Ok(RevocationReason::PublisherRequest),
        "operator" => Ok(RevocationReason::Operator {
            rationale: "cli".into(),
        }),
        other => Err(format!("unknown revocation reason `{other}`")),
    }
}

fn get_flag<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1).map(String::as_str))
}

fn flag_present(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

fn parse_publisher_id(s: &str) -> Result<PublisherId, String> {
    let u = uuid::Uuid::parse_str(s).map_err(|e| format!("bad publisher_id : {e}"))?;
    Ok(PublisherId::from_uuid(u))
}

// ─────────────────────────────────────────────────────────────────────
//  Subcommands
// ─────────────────────────────────────────────────────────────────────

async fn cmd_register(args: &[String]) -> Result<(), String> {
    let name = get_flag(args, "--display-name").ok_or("--display-name required")?;
    let tier = parse_tier(get_flag(args, "--tier").ok_or("--tier required")?)?;
    let pseudo = flag_present(args, "--pseudonymous");
    if pseudo && tier != KycTier::Tier0 {
        return Err("pseudonymous requires --tier 0".into());
    }
    let api = build_api();
    let candidate = NewPublisherCandidate {
        display_name: name.into(),
        target_tier: tier,
        jurisdiction: vec![Iso3166::new("US").map_err(|e| e.to_string())?],
        // CLI key is a deterministic dummy ; real flows wire Vault.
        initial_public_key_pem:
            "-----BEGIN PUBLIC KEY-----\nAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=\n-----END PUBLIC KEY-----"
                .into(),
        algorithm: SignatureAlgorithm::Ed25519,
        pseudonymous: pseudo,
        kyc_dossier_reference: if tier >= KycTier::Tier2 {
            Some("cli-dossier".into())
        } else {
            None
        },
    };
    let resp = api
        .register(RegisterRequest { candidate })
        .await
        .map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(&resp).map_err(|e| e.to_string())?;
    println!("{json}");
    Ok(())
}

async fn cmd_list(args: &[String]) -> Result<(), String> {
    let tier_floor = if let Some(s) = get_flag(args, "--tier-floor") {
        parse_tier(s)?
    } else {
        KycTier::Tier0
    };
    let api = build_api();
    let list = api
        .list(ListQuery { tier_floor })
        .await
        .map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(&list).map_err(|e| e.to_string())?;
    println!("{json}");
    Ok(())
}

async fn cmd_lookup(args: &[String]) -> Result<(), String> {
    let id = args.first().ok_or("publisher_id required")?;
    let id = parse_publisher_id(id)?;
    let api = build_api();
    let identity = api.lookup(id).await.map_err(|e| e.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&identity).map_err(|e| e.to_string())?
    );
    Ok(())
}

async fn cmd_revoke(args: &[String]) -> Result<(), String> {
    let id = args.first().ok_or("publisher_id required")?;
    let id = parse_publisher_id(id)?;
    let reason = parse_reason(get_flag(args, "--reason").ok_or("--reason required")?)?;
    let suspected_at = if let Some(s) = get_flag(args, "--suspected-compromise-at") {
        Some(
            chrono::DateTime::parse_from_rfc3339(s)
                .map_err(|e| format!("--suspected-compromise-at : {e}"))?
                .with_timezone(&chrono::Utc)
                .into(),
        )
    } else {
        None
    };
    let api = build_api();
    let cascade = api
        .revoke(
            id,
            RevokeRequest {
                reason,
                suspected_compromise_at: suspected_at,
                revoking_operator_id: uuid::Uuid::new_v4(),
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&cascade).map_err(|e| e.to_string())?
    );
    Ok(())
}

async fn cmd_sanctions_check(args: &[String]) -> Result<(), String> {
    let id = args.first().ok_or("publisher_id required")?;
    let id = parse_publisher_id(id)?;
    let api = build_api();
    let verdict = api.refresh_sanctions(id).await.map_err(|e| e.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&verdict).map_err(|e| e.to_string())?
    );
    Ok(())
}

async fn cmd_promote(args: &[String]) -> Result<(), String> {
    let id = args.first().ok_or("publisher_id required")?;
    let id = parse_publisher_id(id)?;
    let target = parse_tier(get_flag(args, "--to-tier").ok_or("--to-tier required")?)?;
    let approver_count: u32 = get_flag(args, "--approvers")
        .unwrap_or("2")
        .parse()
        .map_err(|e| format!("--approvers : {e}"))?;
    let approvers = (0..approver_count)
        .map(|_| OperatorSignature {
            operator_id: uuid::Uuid::new_v4(),
            signed_at: SystemTime::now(),
            ticket_id: "cli-approval".into(),
            signature: vec![0u8; 64],
        })
        .collect();
    let api = build_api();
    let after = api
        .promote(
            id,
            PromoteRequest {
                target_tier: target,
                approvers,
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&after).map_err(|e| e.to_string())?
    );
    Ok(())
}

async fn cmd_history(args: &[String]) -> Result<(), String> {
    let id = args.first().ok_or("publisher_id required")?;
    let id = parse_publisher_id(id)?;
    let limit: usize = get_flag(args, "--limit")
        .unwrap_or("50")
        .parse()
        .map_err(|e| format!("--limit : {e}"))?;
    let api = build_api();
    let history = api
        .history(id, HistoryQuery { limit, offset: 0 })
        .await
        .map_err(|e| e.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&history).map_err(|e| e.to_string())?
    );
    Ok(())
}

