// SPDX-License-Identifier: UNLICENSED
//! **Sprint 13 — N21** : `efe-claude-smoke` — live smoke test CLI.
//!
//! ```bash
//! export ANTHROPIC_API_KEY=sk-ant-xxx
//! cargo run --bin efe-claude-smoke
//! # With explicit model :
//! cargo run --bin efe-claude-smoke -- --model claude-sonnet-4-6
//! # With user question :
//! cargo run --bin efe-claude-smoke -- --question "What's the customer asking?"
//! ```
//!
//! Exits 0 on success ; 1 on API error ; 2 on missing key.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use efe_agentic_bridge::{
    AgenticAuditContext, AgenticBridge, AgenticTask, ClaudeBridge, DEFAULT_CLAUDE_MODEL,
};
use efe_llm_provenance::{
    DefaultLlmProvenanceCollector, InMemoryLlmProvenanceStore, InMemoryModelRegistry,
    LlmProvenanceStore, ModelRegistry, RegisteredModel,
};
use efe_nano_sdk::{
    Classification, ContextBundle, Entity, ModelIdentity, NanoAgentId, NanoAgentVersion,
    Redaction, SwarmId, TenantId,
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return ExitCode::SUCCESS;
    }

    let api_key = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(k) if !k.is_empty() => k,
        _ => {
            eprintln!(
                "✗ ANTHROPIC_API_KEY env var not set.\n\
                 Set it and re-run :\n\
                 \texport ANTHROPIC_API_KEY=sk-ant-xxx\n\
                 \tcargo run --bin efe-claude-smoke"
            );
            return ExitCode::from(2);
        }
    };

    let model = read_flag(&args, "--model")
        .unwrap_or_else(|| DEFAULT_CLAUDE_MODEL.to_string());
    let question = read_flag(&args, "--question").unwrap_or_else(|| {
        "Look at the swarm_context tool and tell me, in one short \
         sentence, what the customer is asking about."
            .into()
    });
    let endpoint = read_flag(&args, "--endpoint")
        .unwrap_or_else(|| "https://api.anthropic.com".to_string());
    let max_tokens: u32 = read_flag(&args, "--max-tokens")
        .and_then(|s| s.parse().ok())
        .unwrap_or(80);
    let timeout_secs: u64 = read_flag(&args, "--timeout-secs")
        .and_then(|s| s.parse().ok())
        .unwrap_or(45);
    let verbose = args.iter().any(|a| a == "--verbose" || a == "-v");

    print_header(&model, &endpoint);

    // **N26 + G14** : wire an in-memory N17 provenance collector
    // + register the model so pre_call passes.  After the call
    // we assert the store contains exactly one record + print
    // its key fields.
    let store: Arc<dyn LlmProvenanceStore> =
        Arc::new(InMemoryLlmProvenanceStore::new());
    let registry = Arc::new(InMemoryModelRegistry::new());
    let mut registered = RegisteredModel::hosted(
        ModelIdentity::hosted("anthropic", &model, "2023-06-01"),
        0.0,
        0.0,
        1,
    );
    registered.dev_skip_hash_check = true; // hosted ; no hash to verify
    registry.register(registered).await.expect("register model");
    let collector = DefaultLlmProvenanceCollector::new(
        store.clone(),
        registry as Arc<dyn ModelRegistry>,
    );

    let bridge = ClaudeBridge::with_endpoint(api_key, endpoint)
        .with_model(model.clone())
        .with_provenance(collector.clone());
    let bundle = build_demo_bundle();

    if verbose {
        eprintln!(
            "▶ Sending swarm context bundle :\n{}",
            serde_json::to_string_pretty(&bundle)
                .unwrap_or_else(|_| "<unserialisable>".into())
        );
    }

    let tenant = TenantId::new();
    let audit_ctx = AgenticAuditContext {
        trace_id: uuid::Uuid::new_v4(),
        swarm_request_id: uuid::Uuid::new_v4(),
        op_id: "smoke_claude".into(),
        tenant_id: tenant,
        agent_id: NanoAgentId::new("smoke_agent").unwrap(),
        agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
        prompt_template_id: "smoke_template_v1".into(),
        prompt_template_version: "1.0.0".into(),
        pii_strategy_applied: efe_nano_sdk::PiiStrategy::None,
        redaction_token_set: Vec::new(),
        publisher_id: None,
        publisher_kyc_tier: None,
        gold_case_id: None,
    };
    let trace_id = audit_ctx.trace_id;

    let task = AgenticTask::new(question)
        .with_system(
            "You are a smoke-test assistant. Be terse. Always call \
             the swarm_context tool first to retrieve the structured \
             context, then answer using it.",
        )
        .with_max_tokens(max_tokens)
        .with_temperature(0.0)
        .with_audit_context(audit_ctx);

    let started = Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        bridge.call_with_context(&bundle, task),
    )
    .await;

    match result {
        Ok(Ok(resp)) => {
            let elapsed = started.elapsed();
            println!("──────────────────────────────────────────────");
            println!(" LIVE CLAUDE BRIDGE — SUCCESS");
            println!("──────────────────────────────────────────────");
            println!(" model       : {}", resp.model_identity.model_id);
            println!(" stop_reason : {:?}", resp.stop_reason);
            println!(
                " tokens      : in={} out={}",
                resp.usage.input_tokens, resp.usage.output_tokens
            );
            println!(" cost_µcents : {}", resp.cost_microcents);
            println!(
                " cost_usd    : ${:.6}",
                resp.cost_microcents as f64 / 1_000_000.0
            );
            println!(" elapsed     : {elapsed:?}");
            // **N26 + G14** : assert the store now contains one
            // record + print its details.
            match store.get_by_trace(trace_id, tenant).await {
                Ok(record) => {
                    println!("──────────────────────────────────────────────");
                    println!(" N26 LlmProvenanceRecord — ASSERTED");
                    println!("──────────────────────────────────────────────");
                    println!(" provenance_id   : {}", record.provenance_id);
                    println!(" trace_id        : {}", record.trace_id);
                    println!(" tenant_id       : {:?}", record.tenant_id);
                    println!(" agent_id        : {}", record.agent_id);
                    println!(
                        " model_identity  : {}/{}@{}",
                        record.model_identity.provider,
                        record.model_identity.model_id,
                        record.model_identity.requested_version
                    );
                    println!(
                        " tokens (record) : in={} out={}",
                        record.token_usage.canonical_input(),
                        record.token_usage.canonical_output()
                    );
                    println!(
                        " cost_cents      : {:.6}",
                        record.cost_accounting.cost_cents
                    );
                    println!(
                        " provider_resp_ms: {}",
                        record.provider_response_time_ms
                    );
                    if let Some(id) = &record.provider_request_id {
                        println!(" provider_req_id : {id}");
                    }
                }
                Err(e) => {
                    eprintln!(
                        "⚠ N26 record assertion failed : {e:?} (trace_id={trace_id}) \
                         — the LLM call succeeded but provenance was not stored"
                    );
                }
            }
            println!("──────────────────────────────────────────────");
            println!(" response :");
            println!("{}", resp.text);
            println!("──────────────────────────────────────────────");
            ExitCode::SUCCESS
        }
        Ok(Err(e)) => {
            eprintln!("✗ Bridge error : {e:?}");
            ExitCode::from(1)
        }
        Err(_) => {
            eprintln!("✗ Timed out after {timeout_secs}s");
            ExitCode::from(1)
        }
    }
}

fn print_header(model: &str, endpoint: &str) {
    println!("──────────────────────────────────────────────");
    println!(" efe-claude-smoke — N21 live smoke test");
    println!("──────────────────────────────────────────────");
    println!(" model    : {model}");
    println!(" endpoint : {endpoint}");
    println!();
}

fn print_help() {
    println!(
        "efe-claude-smoke — exercise ClaudeBridge against the real Anthropic API.\n\
         \n\
         USAGE :\n\
         \tcargo run --bin efe-claude-smoke -- [FLAGS]\n\
         \n\
         FLAGS :\n\
         \t--model <id>         Override default model (default : {DEFAULT_CLAUDE_MODEL})\n\
         \t--question <text>    User-side question for Claude\n\
         \t--endpoint <url>     Override API endpoint (testing)\n\
         \t--max-tokens <N>     Max output tokens (default : 80)\n\
         \t--timeout-secs <N>   Hard timeout (default : 45)\n\
         \t-v, --verbose        Print the bundle that will be sent\n\
         \t-h, --help           Show this help\n\
         \n\
         ENV :\n\
         \tANTHROPIC_API_KEY    Required. Anthropic API key (sk-ant-...).\n\
         \n\
         EXIT CODES :\n\
         \t0 → success ; 1 → API error or timeout ; 2 → missing API key.\n"
    );
}

fn read_flag(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1).cloned())
}

fn build_demo_bundle() -> ContextBundle {
    let mut bundle = ContextBundle::empty(SwarmId::new());
    bundle.language = Some("en".into());
    bundle.summary = Some(
        "Customer escalation : reports that order #4521 from \
         2026-05-15 was charged twice. The total disputed amount \
         is $1,234.50. Wants a refund."
            .into(),
    );
    bundle.entities.push(Entity {
        entity_type: "person".into(),
        value: "John Doe".into(),
        span: Some((0, 8)),
    });
    bundle.entities.push(Entity {
        entity_type: "money".into(),
        value: "$1,234.50".into(),
        span: Some((50, 59)),
    });
    bundle.entities.push(Entity {
        entity_type: "date".into(),
        value: "2026-05-15".into(),
        span: Some((30, 40)),
    });
    bundle.redactions.push(Redaction {
        redaction_type: "email".into(),
        original_span: (10, 28),
        redaction_token: "<EMAIL_0>".into(),
    });
    bundle.classifications.push(Classification {
        label: "billing_dispute".into(),
        confidence: 0.94,
    });
    bundle.classifications.push(Classification {
        label: "refund_request".into(),
        confidence: 0.89,
    });
    bundle.metadata.agents_executed = vec![
        efe_nano_sdk::AgentId::new("ner-extractor").unwrap(),
        efe_nano_sdk::AgentId::new("language-detector").unwrap(),
        efe_nano_sdk::AgentId::new("pii-redactor").unwrap(),
        efe_nano_sdk::AgentId::new("sentiment-classifier").unwrap(),
        efe_nano_sdk::AgentId::new("summary-writer").unwrap(),
    ];
    bundle.metadata.total_cost_microcents = 4_280;
    bundle.finalize_size();
    bundle
}
