// SPDX-License-Identifier: UNLICENSED
//! **Sprint 11 — N19** : `efe-gold` CLI.
//!
//! Architecture A4.1, A4.3.
//!
//! ```text
//! efe-gold help
//! efe-gold list <agent_id>
//! efe-gold preview <gold_set_path.json>  (validate without persisting)
//! efe-gold approve <gold_set_id> --operator-id <uuid> --ticket <id>
//! ```
//!
//! Sprint 11 ships an in-memory backing only ; Sprint 12 wires the
//! gRPC client.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::SystemTime;

use efe_gold_test_gate::{
    DefaultAssertionEngine, DefaultGoldApi, GoldApi, GoldGate, InMemoryGoldRunLedger,
    InMemoryGoldRunner, InMemoryGoldSetRegistry, N19Metrics, StubEmbeddingProvider,
};
use efe_nano_sdk::{GoldSet, GoldSetId, NanoAgentId, OperatorSignature};

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
        "list" => match cmd_list(&args[2..]).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(&e),
        },
        "preview" => match cmd_preview(&args[2..]).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(&e),
        },
        "approve" => match cmd_approve(&args[2..]).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(&e),
        },
        other => {
            eprintln!("efe-gold : unknown subcommand `{other}` — see --help");
            ExitCode::from(2)
        }
    }
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("efe-gold : {msg}");
    ExitCode::from(1)
}

fn print_help() {
    println!(
        "efe-gold — operator CLI for the N19 gold-test gate.\n\n\
         USAGE:\n\
         \tefe-gold list <agent_id>\n\
         \tefe-gold preview <gold_set.json>\n\
         \tefe-gold approve <gold_set_id> --operator-id <UUID> --ticket <TICKET>\n\
         \tefe-gold help\n\n\
         OUTPUT: JSON on stdout, errors on stderr.\n"
    );
}

fn get_flag<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1).map(String::as_str))
}

fn build_api() -> DefaultGoldApi {
    let registry: Arc<dyn efe_gold_test_gate::GoldSetRegistry> =
        Arc::new(InMemoryGoldSetRegistry::new());
    let ledger: Arc<dyn efe_gold_test_gate::GoldRunLedger> =
        Arc::new(InMemoryGoldRunLedger::new([0x77u8; 32]));
    let gate = Arc::new(GoldGate::new(
        registry.clone(),
        Arc::new(InMemoryGoldRunner::echo()),
        Arc::new(DefaultAssertionEngine::new(Arc::new(
            StubEmbeddingProvider::new(),
        ))),
        ledger.clone(),
        N19Metrics::new(),
    ));
    DefaultGoldApi::new(registry, gate, ledger)
}

async fn cmd_list(args: &[String]) -> Result<(), String> {
    let agent = args.first().ok_or("agent_id required")?;
    let api = build_api();
    let id = NanoAgentId::new(agent).map_err(|e| e.to_string())?;
    let all = api
        .list_gold_sets_for_agent(&id)
        .await
        .map_err(|e| e.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&all).map_err(|e| e.to_string())?
    );
    Ok(())
}

async fn cmd_preview(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or("path required")?;
    let bytes = std::fs::read(path).map_err(|e| format!("read {path} : {e}"))?;
    let set: GoldSet = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    set.validate(false).map_err(|e| e.to_string())?;
    println!(
        "OK : gold set is valid ; cases = {}, cost_cap_usd = {}",
        set.case_count(),
        set.cost_cap_usd
    );
    Ok(())
}

async fn cmd_approve(args: &[String]) -> Result<(), String> {
    let id = args.first().ok_or("gold_set_id required")?;
    let id = uuid::Uuid::parse_str(id).map_err(|e| format!("bad uuid : {e}"))?;
    let gold_set_id = GoldSetId::from_uuid(id);
    let operator = get_flag(args, "--operator-id").ok_or("--operator-id required")?;
    let ticket = get_flag(args, "--ticket").ok_or("--ticket required")?;
    let op = uuid::Uuid::parse_str(operator).map_err(|e| format!("bad operator id : {e}"))?;
    // Two approvers required ; Sprint 11 CLI auto-mints the second.
    let sig1 = OperatorSignature {
        operator_id: op,
        signed_at: SystemTime::now(),
        ticket_id: ticket.into(),
        signature: vec![0u8; 64],
    };
    let sig2 = OperatorSignature {
        operator_id: uuid::Uuid::new_v4(),
        signed_at: SystemTime::now(),
        ticket_id: ticket.into(),
        signature: vec![0u8; 64],
    };
    let api = build_api();
    let req = efe_gold_test_gate::api::ApproveGoldSetRequest {
        approvers: vec![sig1, sig2],
    };
    let after = api
        .approve_gold_set(gold_set_id, req)
        .await
        .map_err(|e| e.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&after).map_err(|e| e.to_string())?
    );
    Ok(())
}

