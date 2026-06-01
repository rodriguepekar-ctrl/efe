//! End-to-end Phase 2 Sprint 5 (Mode C) integration tests.
//!
//! Wires together :
//!   InMemoryCatalog
//!   + HandlerRegistry        (Mode A)
//!   + PureRustRuntime        (Mode A)
//!   + InProcessModelLoader   (Mode B)
//!   + StubVmPool (Mode B + Mode C)
//!   + LocalModelRuntime      (Mode B)
//!   + BuiltinLlmClient       (Mode C)
//!   + OutputValidator        (Mode C)
//!   + InMemoryBudgetEnforcer (Mode C)
//!   + PiiRedactor            (Mode C)
//!   + LlmAugmentedRuntime    (Mode C)
//!   + MultiModeRuntime       (A + B + C)
//!   + AuditWriter            (stub)
//!   + IdempotencyCache       (in-memory)
//!   + Dispatcher
//!
//! and runs realistic mixed-mode swarms through all 20 starter agents.

use std::sync::Arc;
use uuid::Uuid;

use efe_nano::audit::{stub::InMemoryAuditWriter, AuditWriter};
use efe_nano::budget::{BudgetEnforcer, InMemoryBudgetEnforcer};
use efe_nano::catalog::InMemoryCatalog;
use efe_nano::dispatcher::Dispatcher;
use efe_nano::idempotency::IdempotencyCache;
use efe_nano::metering::{ExecutionModeKind, InMemoryMeteringClient, MeteringClient};
use efe_nano::runtime::model_loader::InProcessModelLoader;
use efe_nano::runtime::pool::stub::{StubVmPool, StubVmPoolConfig};
use efe_nano::runtime::pool::VmPool;
use efe_nano::runtime::pure_rust::{HandlerRegistry, PureRustRuntime};
use efe_nano::runtime::{
    BuiltinLlmClient, LlmAugmentedRuntime, LocalModelRuntime, MultiModeRuntime, NanoAgentExecutor,
    OutputValidator, PiiRedactor,
};
use efe_nano::starter_pack::{
    mode_c_output_schemas, register_all_starter_pack_with_mode_b_and_c,
};
use efe_nano_sdk::{
    Catalog, NanoAgentId, NanoAgentRef, OpStatus, Operation, OperationInput, RetryPolicy,
    SwarmRequest, SwarmStatus, TenantId,
};

// ─────────────────────────────────────────────────────────────────────
//  Harness
// ─────────────────────────────────────────────────────────────────────

struct Harness {
    dispatcher: Dispatcher,
    audit: Arc<InMemoryAuditWriter>,
    budget: Arc<InMemoryBudgetEnforcer>,
    llm: Arc<BuiltinLlmClient>,
    metering: Arc<InMemoryMeteringClient>,
}

async fn make_harness() -> Harness {
    make_harness_with_budget_cap(10_000.0).await
}

async fn make_harness_with_budget_cap(cap_cents: f64) -> Harness {
    let catalog = Arc::new(InMemoryCatalog::new());
    let registry = HandlerRegistry::new();
    let count = register_all_starter_pack_with_mode_b_and_c(&*catalog, &registry)
        .await
        .unwrap();
    assert_eq!(count, 20);

    // Shared in-memory metering : observable from all 3 runtimes.
    let metering = Arc::new(InMemoryMeteringClient::new());
    let metering_dyn: Arc<dyn MeteringClient> = Arc::clone(&metering) as _;

    // Pool with both Mode B model loader + Mode C LLM client.
    let llm = Arc::new(BuiltinLlmClient::new());
    let pool = Arc::new(StubVmPool::with_llm(
        StubVmPoolConfig::default(),
        Arc::new(InProcessModelLoader::with_phase_2_builtins()),
        Arc::clone(&llm) as Arc<dyn efe_nano::runtime::LlmClient>,
    ));

    // 3 mode runtimes — all wired with the shared metering client.
    let mode_a = Arc::new(PureRustRuntime::with_metering(
        registry,
        Arc::clone(&metering_dyn),
    ));
    let mode_b = Arc::new(LocalModelRuntime::with_metering(
        Arc::clone(&pool) as Arc<dyn VmPool>,
        Arc::clone(&metering_dyn),
    ));

    let budget = Arc::new(InMemoryBudgetEnforcer::with_cap(cap_cents));
    let validator = Arc::new(OutputValidator::new());
    let mode_c = Arc::new(LlmAugmentedRuntime::with_metering(
        Arc::clone(&pool) as Arc<dyn VmPool>,
        Arc::clone(&llm) as Arc<dyn efe_nano::runtime::LlmClient>,
        Arc::clone(&validator),
        Arc::clone(&budget) as Arc<dyn BudgetEnforcer>,
        Arc::new(PiiRedactor::with_default_regex_backend()),
        Arc::clone(&metering_dyn),
    ));

    // Pre-warm validator with all 5 Mode C output schemas.
    mode_c.warm_output_validator(&mode_c_output_schemas()).unwrap();

    let multi: Arc<dyn NanoAgentExecutor> = Arc::new(
        MultiModeRuntime::mode_a_b_c(mode_a, mode_b, mode_c),
    );

    let mem_audit = Arc::new(InMemoryAuditWriter::new());
    let audit_dyn: Arc<dyn AuditWriter> = mem_audit.clone();
    let idempotency = Arc::new(IdempotencyCache::new());
    let dispatcher = Dispatcher::new(
        catalog as Arc<dyn Catalog>,
        multi,
        audit_dyn,
        idempotency,
    );

    Harness {
        dispatcher,
        audit: mem_audit,
        budget,
        llm,
        metering,
    }
}

fn lit(v: serde_json::Value) -> OperationInput {
    OperationInput::Literal { value: v }
}

fn op(id: &str, agent_id: &str, deps: &[&str], input: OperationInput) -> Operation {
    Operation {
        op_id: id.to_string(),
        agent: NanoAgentRef::new(NanoAgentId::new(agent_id).unwrap()),
        input,
        depends_on: deps.iter().map(|s| s.to_string()).collect(),
        timeout_ms: None,
        retry: RetryPolicy::default(),
    }
}

fn req(ops: Vec<Operation>) -> SwarmRequest {
    SwarmRequest {
        request_id: Uuid::new_v4(),
        tenant_id: TenantId::system(),
        caller_agent_id: Some("phase-2-mode-c-test".to_string()),
        operations: ops,
        max_parallelism: 100,
        timeout_ms: 60_000,
        idempotency_key: None,
        streaming: false,
        audit_tags: vec!["phase-2".to_string(), "mode-c".to_string()],
        cost_cap_cents: None,
        parent_envelope_trace_id: None,
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Scenarios
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn scenario_all_5_mode_c_agents_invokable_in_parallel() {
    let h = make_harness().await;
    let ops = vec![
        op(
            "summarize",
            "summarize_strict",
            &[],
            lit(serde_json::json!({
                "text": "First sentence. Second sentence. Third sentence.",
                "max_sentences": 2,
            })),
        ),
        op(
            "extract",
            "extract_structured",
            &[],
            lit(serde_json::json!({
                "text": "John bought 42 widgets",
                "fields": [
                    {"name": "buyer", "type": "string"},
                    {"name": "qty", "type": "number"},
                ],
            })),
        ),
        op(
            "topic",
            "classify_topic_llm",
            &[],
            lit(serde_json::json!({"text": "stock market analysis"})),
        ),
        op(
            "sentiment",
            "sentiment_nuanced",
            &[],
            lit(serde_json::json!({"text": "This product is great"})),
        ),
        op(
            "qa",
            "qa_grounded",
            &[],
            lit(serde_json::json!({
                "question": "What is the capital",
                "context": "Paris is the capital of France.",
            })),
        ),
    ];
    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);
    assert_eq!(resp.results.len(), 5);
    for r in resp.results.values() {
        assert_eq!(
            r.status,
            OpStatus::Success,
            "{} failed : {:?}",
            r.op_id,
            r.error
        );
    }
    assert_eq!(h.audit.op_count().await, 5);
}

#[tokio::test]
async fn scenario_mixed_mode_a_b_c_dag_in_one_swarm() {
    let h = make_harness().await;
    // Layer 0 : json_validate (A), ner_clinical (B), classify_topic_llm (C)
    // Layer 1 : summarize_strict (C, depends on B)
    let ops = vec![
        op(
            "json",
            "json_validate",
            &[],
            lit(serde_json::json!({"value": 1, "schema": {"type": "integer"}})),
        ),
        op(
            "ner",
            "ner_clinical",
            &[],
            lit(serde_json::json!({"text": "metformin for diabetes"})),
        ),
        op(
            "topic",
            "classify_topic_llm",
            &[],
            lit(serde_json::json!({"text": "tech news today"})),
        ),
        op(
            "summarize",
            "summarize_strict",
            &["ner"],
            lit(serde_json::json!({
                "text": "Detailed report. Many findings. Conclusions follow.",
                "max_sentences": 2,
            })),
        ),
    ];
    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);
    for r in resp.results.values() {
        assert_eq!(r.status, OpStatus::Success);
    }
}

#[tokio::test]
async fn scenario_mode_c_failure_propagates_to_downstream() {
    let h = make_harness().await;
    // a : summarize_strict with invalid input (missing max_sentences)
    // b : json_validate depends on a → Skipped
    let ops = vec![
        op(
            "a",
            "summarize_strict",
            &[],
            lit(serde_json::json!({"text": "x"})),    // missing max_sentences
        ),
        op(
            "b",
            "json_validate",
            &["a"],
            lit(serde_json::json!({
                "value": "x",
                "schema": {"type": "string"}
            })),
        ),
    ];
    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::PartialFailure);
    assert_eq!(resp.results["a"].status, OpStatus::Failed);
    assert_eq!(resp.results["b"].status, OpStatus::Skipped);
}

#[tokio::test]
async fn scenario_mode_c_budget_exhausted_after_n_calls() {
    // Cap at 1 cent — Mode C cost > 0.1 cent per call typically.
    let h = make_harness_with_budget_cap(1.0).await;
    // 5 calls — at least one should hit budget exhaustion.
    let mut ops = Vec::new();
    for i in 0..5 {
        ops.push(op(
            &format!("op-{i}"),
            "summarize_strict",
            &[],
            lit(serde_json::json!({
                "text": format!("text {i}. another."),
                "max_sentences": 1,
            })),
        ));
    }
    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    // PartialFailure expected : some succeeded, some hit budget limit.
    assert_eq!(resp.overall_status, SwarmStatus::PartialFailure);
    let failures: Vec<_> = resp
        .results
        .values()
        .filter(|r| r.status == OpStatus::Failed)
        .collect();
    assert!(!failures.is_empty(), "expected at least one budget failure");
    for f in &failures {
        let err = f.error.as_ref().unwrap();
        assert_eq!(
            err.kind,
            efe_nano_sdk::OperationErrorKind::BudgetExceeded
        );
    }
}

#[tokio::test]
async fn scenario_mode_c_concurrent_swarms_share_budget() {
    use std::sync::Arc as StdArc;
    let h = StdArc::new(make_harness().await);
    let mut handles = Vec::new();
    for i in 0..4 {
        let h = StdArc::clone(&h);
        handles.push(tokio::spawn(async move {
            let ops = vec![op(
                "topic",
                "classify_topic_llm",
                &[],
                lit(serde_json::json!({"text": format!("text {i}")})),
            )];
            h.dispatcher.dispatch(req(ops)).await
        }));
    }
    for h in handles {
        let resp = h.await.unwrap().unwrap();
        assert_eq!(resp.overall_status, SwarmStatus::Success);
    }
}

#[tokio::test]
async fn scenario_mode_c_pii_redaction_called_in_pipeline() {
    // We can't easily inspect the pre-LLM redaction from the dispatcher
    // path (the BuiltinLlmClient sees redacted strings).  Instead we
    // check that an input with an email + phone + SSN doesn't crash AND
    // that the LLM call count increments (i.e. pipeline completed).
    let h = make_harness().await;
    let prev_calls = h.llm.call_count_for("m50.summarize_strict.v1").await;
    let ops = vec![op(
        "s",
        "summarize_strict",
        &[],
        lit(serde_json::json!({
            "text": "Email john@x.com, phone +1-555-1234567, SSN 123-45-6789",
            "max_sentences": 1,
        })),
    )];
    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);
    let new_calls = h.llm.call_count_for("m50.summarize_strict.v1").await;
    assert!(new_calls > prev_calls);
}

#[tokio::test]
async fn scenario_mode_c_total_cost_aggregates_provider_costs() {
    let h = make_harness().await;
    let ops = vec![
        op(
            "s",
            "summarize_strict",
            &[],
            lit(serde_json::json!({"text": "x. y. z.", "max_sentences": 2})),
        ),
        op(
            "t",
            "classify_topic_llm",
            &[],
            lit(serde_json::json!({"text": "stock market"})),
        ),
    ];
    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    let summed: f64 = resp.results.values().map(|r| r.cost_cents).sum();
    assert!((resp.total_cost_cents - summed).abs() < 1e-9);
    // Both ops > 0 cents (Mode C is non-free).
    for r in resp.results.values() {
        assert!(r.cost_cents > 0.0);
    }
}

#[tokio::test]
async fn scenario_mode_c_audit_trace_records_llm_metadata() {
    let h = make_harness().await;
    let ops = vec![op(
        "s",
        "summarize_strict",
        &[],
        lit(serde_json::json!({"text": "Hello.", "max_sentences": 1})),
    )];
    let _ = h.dispatcher.dispatch(req(ops)).await.unwrap();
    // N14 : 1 op + 1 envelope = 2 ; filter to op.
    let traces = h.audit.traces().await;
    let op_traces: Vec<_> = traces.iter().filter(|t| t.error_kind.is_none()).collect();
    assert_eq!(op_traces.len(), 1);
    let t = op_traces[0];
    assert_eq!(t.execution_mode, "llm_augmented");
    assert!(t.llm_tokens_used.is_some());
    let tokens = t.llm_tokens_used.unwrap();
    assert!(tokens > 0);
    assert!(t.cost_cents > 0.0);
}

#[tokio::test]
async fn scenario_mode_c_only_when_runtime_lacks_mode_c() {
    // Construct a harness with ONLY Mode A + B wired (no Mode C).
    let catalog = Arc::new(InMemoryCatalog::new());
    let registry = HandlerRegistry::new();
    register_all_starter_pack_with_mode_b_and_c(&*catalog, &registry)
        .await
        .unwrap();
    let pool = Arc::new(StubVmPool::with_phase_2_defaults());
    let mode_a = Arc::new(PureRustRuntime::new(registry));
    let mode_b = Arc::new(LocalModelRuntime::new(Arc::clone(&pool) as Arc<dyn VmPool>));
    let multi: Arc<dyn NanoAgentExecutor> =
        Arc::new(MultiModeRuntime::mode_a_and_b(mode_a, mode_b));
    let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
    let idempotency = Arc::new(IdempotencyCache::new());
    let dispatcher = Dispatcher::new(
        catalog as Arc<dyn Catalog>,
        multi,
        audit,
        idempotency,
    );

    let ops = vec![op(
        "s",
        "summarize_strict",
        &[],
        lit(serde_json::json!({"text": "x", "max_sentences": 1})),
    )];
    let resp = dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::PartialFailure);
    let err = resp.results["s"].error.as_ref().unwrap();
    assert_eq!(err.kind, efe_nano_sdk::OperationErrorKind::InternalError);
    assert!(err.message.contains("llm_augmented"));
}

#[tokio::test]
async fn scenario_20_starter_agents_all_invokable() {
    let h = make_harness().await;
    // 10 Mode A + 5 Mode B + 5 Mode C.
    let ops = vec![
        // Mode A.
        op("json", "json_validate", &[],
            lit(serde_json::json!({"value": 1, "schema": {"type": "integer"}}))),
        op("xml", "xml_validate", &[],
            lit(serde_json::json!({"xml": "<root/>"}))),
        op("regex", "regex_extract", &[],
            lit(serde_json::json!({"pattern": "\\d+", "text": "abc 1 def"}))),
        op("date", "date_normalize", &[],
            lit(serde_json::json!({"input": "2026-05-18"}))),
        op("currency", "currency_normalize", &[],
            lit(serde_json::json!({"input": "$10"}))),
        op("md", "markdown_extract_headings", &[],
            lit(serde_json::json!({"markdown": "# H1"}))),
        op("csv", "csv_normalize", &[],
            lit(serde_json::json!({"csv": "a,b\n1,2"}))),
        op("sql", "sql_parse_safe", &[],
            lit(serde_json::json!({"sql": "SELECT 1"}))),
        op("html", "html_extract_main_content", &[],
            lit(serde_json::json!({"html": "<p>p</p>"}))),
        op("pdf", "pdf_extract_text", &[],
            lit(serde_json::json!({"pdf_base64": ""}))),
        // Mode B.
        op("ner_clinical", "ner_clinical", &[],
            lit(serde_json::json!({"text": "aspirin"}))),
        op("ner_legal", "ner_legal", &[],
            lit(serde_json::json!({"text": "plaintiff"}))),
        op("intent", "intent_classify_generic", &[],
            lit(serde_json::json!({"text": "buy"}))),
        op("lang", "lang_detect", &[],
            lit(serde_json::json!({"text": "the and is of"}))),
        op("embed", "embed_text", &[],
            lit(serde_json::json!({"text": "vec me"}))),
        // Mode C.
        op("summarize", "summarize_strict", &[],
            lit(serde_json::json!({"text": "X. Y.", "max_sentences": 1}))),
        op("extract", "extract_structured", &[],
            lit(serde_json::json!({"text": "1 widget", "fields": [{"name": "qty", "type": "number"}]}))),
        op("topic", "classify_topic_llm", &[],
            lit(serde_json::json!({"text": "stock"}))),
        op("sentiment", "sentiment_nuanced", &[],
            lit(serde_json::json!({"text": "great"}))),
        op("qa", "qa_grounded", &[],
            lit(serde_json::json!({"question": "what", "context": "Paris is the capital"}))),
    ];
    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);
    assert_eq!(resp.results.len(), 20);
    for r in resp.results.values() {
        assert_eq!(r.status, OpStatus::Success, "{} : {:?}", r.op_id, r.error);
    }
    assert_eq!(h.audit.op_count().await, 20);
}

#[tokio::test]
async fn scenario_mode_c_idempotency_second_call_caches() {
    let h = make_harness().await;
    let ops = vec![op(
        "t",
        "classify_topic_llm",
        &[],
        lit(serde_json::json!({"text": "stock market"})),
    )];
    let r1 = h.dispatcher.dispatch(req(ops.clone())).await.unwrap();
    let r2 = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(r1.overall_status, SwarmStatus::Success);
    assert_eq!(r2.overall_status, SwarmStatus::Success);
    // **N12 + N14** : 1 cold + 1 cache-hit + 2 envelopes (per swarm) = 4.
    let traces = h.audit.traces().await;
    assert_eq!(traces.len(), 4, "1 cold + 1 cache-hit + 2 envelopes");
    let cold = traces.iter().filter(|t| t.error_kind.is_none()).count();
    let hit = traces
        .iter()
        .filter(|t| t.error_kind.as_deref() == Some("CacheHit"))
        .count();
    assert_eq!(cold, 1);
    assert_eq!(hit, 1);
}

#[tokio::test]
async fn scenario_mode_c_budget_reservation_released_on_error() {
    let h = make_harness().await;
    // Force LLM error → reservation must be released.
    h.llm.force_error("simulated provider outage").await;
    let ops = vec![op(
        "s",
        "summarize_strict",
        &[],
        lit(serde_json::json!({"text": "x", "max_sentences": 1})),
    )];
    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::PartialFailure);
    // Reservation must be released (snapshot reserved == 0).
    let snap = h.budget.snapshot(TenantId::system()).await.unwrap();
    assert_eq!(snap.reserved_cents, 0.0);
    assert_eq!(snap.used_cents, 0.0);
}

#[tokio::test]
async fn scenario_mode_c_caller_agent_id_preserved() {
    let h = make_harness().await;
    let mut request = req(vec![op(
        "s",
        "summarize_strict",
        &[],
        lit(serde_json::json!({"text": "x.", "max_sentences": 1})),
    )]);
    request.caller_agent_id = Some("claude-sonnet-4-5".to_string());
    let _ = h.dispatcher.dispatch(request).await.unwrap();
    // N14 : 1 op + 1 envelope = 2.
    let traces = h.audit.traces().await;
    assert_eq!(traces.len(), 2);
}

#[tokio::test]
async fn scenario_mode_c_long_input_succeeds() {
    let h = make_harness().await;
    let long_text = "Sentence one. ".repeat(500);    // ~ 7 KB
    let ops = vec![op(
        "s",
        "summarize_strict",
        &[],
        lit(serde_json::json!({"text": long_text, "max_sentences": 3})),
    )];
    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);
    let summary = resp.results["s"].output.as_ref().unwrap()["summary"]
        .as_str()
        .unwrap();
    assert!(!summary.is_empty());
}

#[tokio::test]
async fn scenario_mode_c_extract_structured_returns_all_fields() {
    let h = make_harness().await;
    let ops = vec![op(
        "e",
        "extract_structured",
        &[],
        lit(serde_json::json!({
            "text": "John bought 42 widgets on 2026-01-15.",
            "fields": [
                {"name": "buyer", "type": "string"},
                {"name": "qty", "type": "number"},
                {"name": "purchased", "type": "boolean"},
                {"name": "date", "type": "date"}
            ]
        })),
    )];
    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    let extracted = resp.results["e"].output.as_ref().unwrap()["extracted"]
        .as_object()
        .unwrap();
    assert!(extracted.contains_key("buyer"));
    assert!(extracted.contains_key("qty"));
    assert!(extracted.contains_key("purchased"));
    assert!(extracted.contains_key("date"));
}

#[tokio::test]
async fn scenario_mode_c_qa_grounded_returns_citations() {
    let h = make_harness().await;
    let ops = vec![op(
        "q",
        "qa_grounded",
        &[],
        lit(serde_json::json!({
            "question": "What is the capital",
            "context": "Paris is the capital of France. Berlin is the capital of Germany."
        })),
    )];
    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    let citations = resp.results["q"].output.as_ref().unwrap()["citations"]
        .as_array()
        .unwrap();
    assert!(!citations.is_empty());
    let c = &citations[0];
    assert!(c["span_start"].as_u64().unwrap() < c["span_end"].as_u64().unwrap());
}

#[tokio::test]
async fn scenario_mode_c_sentiment_nuanced_detects_positive() {
    let h = make_harness().await;
    let ops = vec![op(
        "s",
        "sentiment_nuanced",
        &[],
        lit(serde_json::json!({
            "text": "This is great and excellent product"
        })),
    )];
    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    let out = resp.results["s"].output.as_ref().unwrap();
    assert_eq!(out["overall_sentiment"], "positive");
    assert!(out["intensity"].as_f64().unwrap() > 0.0);
}

#[tokio::test]
async fn scenario_mode_c_classify_topic_picks_topic() {
    let h = make_harness().await;
    let ops = vec![op(
        "t",
        "classify_topic_llm",
        &[],
        lit(serde_json::json!({"text": "stock market and trading"})),
    )];
    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(
        resp.results["t"].output.as_ref().unwrap()["topic"],
        "finance"
    );
}

// ─────────────────────────────────────────────────────────────────────
//  Metering integration scenarios (Sprint 5 § 8 step 14 + decision G.5)
// ─────────────────────────────────────────────────────────────────────

/// Mode C success path emits all 3 meters with correct values.
#[tokio::test]
async fn metering_mode_c_success_emits_calls_tokens_vmseconds() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    let ops = vec![op(
        "s",
        "summarize_strict",
        &[],
        lit(serde_json::json!({
            "text": "Hello. World.",
            "max_sentences": 1
        })),
    )];
    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);

    // 1 nano_agent_call recorded.
    assert_eq!(h.metering.nano_agent_calls(tenant), 1);
    assert_eq!(h.metering.calls_for_mode(tenant, ExecutionModeKind::ModeC), 1);
    // LLM tokens : at least 1 (BuiltinLlmClient computes from input/output bytes).
    assert!(h.metering.nano_agent_llm_tokens(tenant) > 0);
    // VM seconds : at least 0 (sub-millisecond on the in-process builtin).
    let s = h.metering.nano_agent_vm_seconds(tenant);
    assert!(s >= 0.0);
}

/// Mode C failure path does NOT emit any meter (no double-billing on errors).
#[tokio::test]
async fn metering_mode_c_failure_does_not_emit() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    // Force LLM error.
    h.llm.force_error("simulated outage").await;
    let ops = vec![op(
        "s",
        "summarize_strict",
        &[],
        lit(serde_json::json!({
            "text": "x",
            "max_sentences": 1
        })),
    )];
    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::PartialFailure);
    // Failure → no metering emitted.
    assert_eq!(h.metering.nano_agent_calls(tenant), 0);
    assert_eq!(h.metering.nano_agent_llm_tokens(tenant), 0);
}

/// Mode A success path emits NanoAgentCalls (1 mode_a label, 0 mode_c, 0 vmseconds, 0 tokens).
#[tokio::test]
async fn metering_mode_a_emits_call_only_no_vmseconds_no_tokens() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    let ops = vec![op(
        "j",
        "json_validate",
        &[],
        lit(serde_json::json!({
            "value": 42,
            "schema": {"type": "integer"}
        })),
    )];
    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);

    assert_eq!(h.metering.nano_agent_calls(tenant), 1);
    assert_eq!(h.metering.calls_for_mode(tenant, ExecutionModeKind::ModeA), 1);
    assert_eq!(h.metering.calls_for_mode(tenant, ExecutionModeKind::ModeC), 0);
    // No LLM tokens or VM seconds for Mode A.
    assert_eq!(h.metering.nano_agent_llm_tokens(tenant), 0);
    assert!((h.metering.nano_agent_vm_seconds(tenant) - 0.0).abs() < 1e-9);
}

/// Mode B emits NanoAgentCalls + VmSeconds but NOT LlmTokens.
#[tokio::test]
async fn metering_mode_b_emits_call_and_vmseconds_no_tokens() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    let ops = vec![op(
        "ner",
        "ner_clinical",
        &[],
        lit(serde_json::json!({"text": "metformin and aspirin"})),
    )];
    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);

    assert_eq!(h.metering.nano_agent_calls(tenant), 1);
    assert_eq!(h.metering.calls_for_mode(tenant, ExecutionModeKind::ModeB), 1);
    assert_eq!(h.metering.nano_agent_llm_tokens(tenant), 0);
    let s = h.metering.nano_agent_vm_seconds(tenant);
    assert!(s >= 0.0);
}

/// Mixed-mode DAG aggregates correctly across all 3 modes.
#[tokio::test]
async fn metering_mixed_mode_dag_aggregates_correctly() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    let ops = vec![
        op(
            "a",
            "json_validate",
            &[],
            lit(serde_json::json!({"value": 1, "schema": {"type": "integer"}})),
        ),
        op(
            "b",
            "ner_clinical",
            &[],
            lit(serde_json::json!({"text": "aspirin"})),
        ),
        op(
            "c",
            "classify_topic_llm",
            &[],
            lit(serde_json::json!({"text": "stock market"})),
        ),
    ];
    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);

    // Total 3 calls, 1 per mode.
    assert_eq!(h.metering.nano_agent_calls(tenant), 3);
    assert_eq!(h.metering.calls_for_mode(tenant, ExecutionModeKind::ModeA), 1);
    assert_eq!(h.metering.calls_for_mode(tenant, ExecutionModeKind::ModeB), 1);
    assert_eq!(h.metering.calls_for_mode(tenant, ExecutionModeKind::ModeC), 1);
    // Only Mode C produces LLM tokens.
    assert!(h.metering.nano_agent_llm_tokens(tenant) > 0);
}

/// Multi-tenant metering isolation : tenants don't see each other's meters.
#[tokio::test]
async fn metering_multi_tenant_isolation() {
    let h = make_harness().await;
    let t1 = TenantId::new();
    let t2 = TenantId::new();

    let mut req_a = req(vec![op(
        "s",
        "summarize_strict",
        &[],
        lit(serde_json::json!({"text": "A.", "max_sentences": 1})),
    )]);
    req_a.tenant_id = t1;
    let _ = h.dispatcher.dispatch(req_a).await.unwrap();

    let mut req_b = req(vec![op(
        "s",
        "summarize_strict",
        &[],
        lit(serde_json::json!({"text": "B.", "max_sentences": 1})),
    )]);
    req_b.tenant_id = t2;
    let _ = h.dispatcher.dispatch(req_b.clone()).await.unwrap();
    let _ = h.dispatcher.dispatch(req_b).await.unwrap();              // 2nd t2 call
    // Note : 2nd t2 call may hit idempotency cache → no metering.
    // Either way, t1 has exactly 1 call regardless.

    assert_eq!(h.metering.nano_agent_calls(t1), 1);
    // t2 has 1 (first dispatch) ; second dispatch hit idempotency cache.
    assert_eq!(h.metering.nano_agent_calls(t2), 1);
}

/// 50-op stress test : metering should still be deterministic + accurate
/// under concurrent load.
#[tokio::test]
async fn stress_50_parallel_mode_c_calls_metering_accurate() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    let mut ops = Vec::with_capacity(50);
    for i in 0..50 {
        ops.push(op(
            &format!("op-{i}"),
            "classify_topic_llm",
            &[],
            // Unique input per op so idempotency cache doesn't hit.
            lit(serde_json::json!({"text": format!("stock market run {i}")})),
        ));
    }
    let mut request = req(ops);
    // Raise max_parallelism + budget to accommodate.
    request.max_parallelism = 50;
    request.timeout_ms = 60_000;
    let resp = h.dispatcher.dispatch(request).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);
    assert_eq!(resp.results.len(), 50);
    for r in resp.results.values() {
        assert_eq!(r.status, OpStatus::Success);
    }

    // All 50 calls metered.
    assert_eq!(h.metering.nano_agent_calls(tenant), 50);
    assert_eq!(h.metering.calls_for_mode(tenant, ExecutionModeKind::ModeC), 50);
    // Tokens : strictly positive cumulative.
    assert!(h.metering.nano_agent_llm_tokens(tenant) >= 50);
}

/// 50-op parallel mixed-mode (Mode A + B + C) — final integration stress.
#[tokio::test]
async fn stress_50_parallel_mixed_modes_succeed() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    let mut ops = Vec::with_capacity(60);
    // 20 × Mode A.
    for i in 0..20 {
        ops.push(op(
            &format!("a-{i}"),
            "regex_extract",
            &[],
            lit(serde_json::json!({"pattern": "\\d+", "text": format!("v={i}")})),
        ));
    }
    // 20 × Mode B.
    for i in 0..20 {
        ops.push(op(
            &format!("b-{i}"),
            "lang_detect",
            &[],
            lit(serde_json::json!({"text": format!("the and is {i}")})),
        ));
    }
    // 20 × Mode C (unique inputs to avoid cache).
    for i in 0..20 {
        ops.push(op(
            &format!("c-{i}"),
            "classify_topic_llm",
            &[],
            lit(serde_json::json!({"text": format!("stock market run {i}")})),
        ));
    }

    let mut request = req(ops);
    request.max_parallelism = 60;
    request.timeout_ms = 60_000;
    let resp = h.dispatcher.dispatch(request).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);
    assert_eq!(resp.results.len(), 60);

    // Metering totals.
    assert_eq!(h.metering.nano_agent_calls(tenant), 60);
    assert_eq!(h.metering.calls_for_mode(tenant, ExecutionModeKind::ModeA), 20);
    assert_eq!(h.metering.calls_for_mode(tenant, ExecutionModeKind::ModeB), 20);
    assert_eq!(h.metering.calls_for_mode(tenant, ExecutionModeKind::ModeC), 20);
}

/// Idempotency cache hit must NOT emit metering (call is short-circuited
/// before the runtime is invoked).
#[tokio::test]
async fn metering_idempotency_cache_hit_not_emitted() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    let ops = vec![op(
        "t",
        "classify_topic_llm",
        &[],
        lit(serde_json::json!({"text": "stock market"})),
    )];
    // First dispatch : warms cache + emits metering.
    let _ = h.dispatcher.dispatch(req(ops.clone())).await.unwrap();
    assert_eq!(h.metering.nano_agent_calls(tenant), 1);
    // Second dispatch : cache hit, no metering emission.
    let _ = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(h.metering.nano_agent_calls(tenant), 1);
}

// ─────────────────────────────────────────────────────────────────────
//  Retry-aware accounting (Sprint 5 audit — semantic correctness)
// ─────────────────────────────────────────────────────────────────────

/// When the OutputValidator retry-once fires (first attempt produces
/// invalid JSON shape, second attempt succeeds), the LlmAugmentedRuntime
/// MUST charge the budget for BOTH calls and meter tokens from both.
///
/// We use a custom LlmClient that returns malformed JSON on first call,
/// proper JSON on second call.  Each call produces deterministic tokens
/// so we can assert the totals exactly.
#[tokio::test]
async fn metering_retry_accumulates_tokens_and_cost_across_attempts() {
    use async_trait::async_trait;
    use efe_nano::error::LlmCallError;
    use efe_nano::runtime::{LlmCallRequest, LlmCallResponse, LlmClient};

    struct RetryAwareLlm {
        call_count: tokio::sync::Mutex<u32>,
    }

    #[async_trait]
    impl LlmClient for RetryAwareLlm {
        async fn call(
            &self,
            req: LlmCallRequest,
        ) -> Result<LlmCallResponse, LlmCallError> {
            let mut c = self.call_count.lock().await;
            *c += 1;
            let attempt = *c;
            let content = if attempt == 1 {
                // Bad shape : missing required field.
                serde_json::json!({"bogus": "field"})
            } else {
                // Stronger-prompt retry should see _retry_reminder.
                assert!(
                    req.template_inputs.get("_retry_reminder").is_some(),
                    "retry didn't inject reminder"
                );
                serde_json::json!({"summary": "ok", "sentence_count": 1})
            };
            Ok(LlmCallResponse {
                content,
                tokens_in: 100,
                tokens_out: 50,
                cost_cents: 2.0,
                provider_name: "mock".into(),
                model_name: "retry-aware".into(),
                cache_hit: false,
                duration_ms: 1,
                retry_attempts: 0,
            })
        }
        fn estimate_cost_cents(&self, _: &LlmCallRequest) -> f64 {
            5.0
        }
    }

    // Construct a custom harness with the RetryAwareLlm.
    let catalog = Arc::new(InMemoryCatalog::new());
    let registry = HandlerRegistry::new();
    register_all_starter_pack_with_mode_b_and_c(&*catalog, &registry)
        .await
        .unwrap();
    let llm: Arc<dyn LlmClient> = Arc::new(RetryAwareLlm {
        call_count: tokio::sync::Mutex::new(0),
    });
    let metering = Arc::new(InMemoryMeteringClient::new());
    let metering_dyn: Arc<dyn MeteringClient> = Arc::clone(&metering) as _;
    let pool = Arc::new(StubVmPool::with_llm(
        StubVmPoolConfig::default(),
        Arc::new(InProcessModelLoader::with_phase_2_builtins()),
        Arc::clone(&llm),
    ));
    let mode_a = Arc::new(PureRustRuntime::with_metering(
        registry,
        Arc::clone(&metering_dyn),
    ));
    let mode_b = Arc::new(LocalModelRuntime::with_metering(
        Arc::clone(&pool) as Arc<dyn VmPool>,
        Arc::clone(&metering_dyn),
    ));
    let budget = Arc::new(InMemoryBudgetEnforcer::with_cap(1_000.0));
    let mode_c = Arc::new(LlmAugmentedRuntime::with_metering(
        Arc::clone(&pool) as Arc<dyn VmPool>,
        Arc::clone(&llm),
        Arc::new(OutputValidator::new()),
        Arc::clone(&budget) as Arc<dyn BudgetEnforcer>,
        Arc::new(PiiRedactor::with_default_regex_backend()),
        Arc::clone(&metering_dyn),
    ));
    let multi: Arc<dyn NanoAgentExecutor> = Arc::new(
        MultiModeRuntime::mode_a_b_c(mode_a, mode_b, mode_c),
    );
    let audit_writer = Arc::new(InMemoryAuditWriter::new());
    let audit: Arc<dyn AuditWriter> = Arc::clone(&audit_writer) as _;
    let dispatcher = Dispatcher::new(
        catalog as Arc<dyn Catalog>,
        multi,
        audit,
        Arc::new(IdempotencyCache::new()),
    );

    let ops = vec![op(
        "s",
        "summarize_strict",
        &[],
        lit(serde_json::json!({"text": "Hello.", "max_sentences": 1})),
    )];
    let resp = dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);

    // 2 LLM calls → 2 × 100 in + 2 × 50 out = 300 tokens total.
    let tenant = TenantId::system();
    assert_eq!(metering.nano_agent_llm_tokens(tenant), 300);
    // 1 metered nano agent call (the customer sees 1 op, not 2).
    assert_eq!(metering.nano_agent_calls(tenant), 1);
    // Budget : 2 × 2.0 = 4.0 cents committed.
    let snap = budget.snapshot(tenant).await.unwrap();
    assert!(
        (snap.used_cents - 4.0).abs() < 1e-6,
        "expected 4.0 cents committed (2 attempts × 2.0), got {}",
        snap.used_cents
    );
}

/// When retry exhausts (second attempt also fails), reservation is
/// released — no metering emitted, no budget charged.
#[tokio::test]
async fn metering_retry_exhausted_releases_reservation() {
    use async_trait::async_trait;
    use efe_nano::error::LlmCallError;
    use efe_nano::runtime::{LlmCallRequest, LlmCallResponse, LlmClient};

    struct AlwaysBadLlm;

    #[async_trait]
    impl LlmClient for AlwaysBadLlm {
        async fn call(
            &self,
            _req: LlmCallRequest,
        ) -> Result<LlmCallResponse, LlmCallError> {
            Ok(LlmCallResponse {
                content: serde_json::json!({"missing_required_fields": true}),
                tokens_in: 50,
                tokens_out: 25,
                cost_cents: 1.0,
                provider_name: "mock".into(),
                model_name: "always-bad".into(),
                cache_hit: false,
                duration_ms: 1,
                retry_attempts: 0,
            })
        }
        fn estimate_cost_cents(&self, _: &LlmCallRequest) -> f64 {
            3.0
        }
    }

    let catalog = Arc::new(InMemoryCatalog::new());
    let registry = HandlerRegistry::new();
    register_all_starter_pack_with_mode_b_and_c(&*catalog, &registry)
        .await
        .unwrap();
    let llm: Arc<dyn LlmClient> = Arc::new(AlwaysBadLlm);
    let metering = Arc::new(InMemoryMeteringClient::new());
    let metering_dyn: Arc<dyn MeteringClient> = Arc::clone(&metering) as _;
    let pool = Arc::new(StubVmPool::with_llm(
        StubVmPoolConfig::default(),
        Arc::new(InProcessModelLoader::with_phase_2_builtins()),
        Arc::clone(&llm),
    ));
    let mode_a = Arc::new(PureRustRuntime::with_metering(
        registry,
        Arc::clone(&metering_dyn),
    ));
    let mode_b = Arc::new(LocalModelRuntime::with_metering(
        Arc::clone(&pool) as Arc<dyn VmPool>,
        Arc::clone(&metering_dyn),
    ));
    let budget = Arc::new(InMemoryBudgetEnforcer::with_cap(1_000.0));
    let mode_c = Arc::new(LlmAugmentedRuntime::with_metering(
        Arc::clone(&pool) as Arc<dyn VmPool>,
        Arc::clone(&llm),
        Arc::new(OutputValidator::new()),
        Arc::clone(&budget) as Arc<dyn BudgetEnforcer>,
        Arc::new(PiiRedactor::with_default_regex_backend()),
        Arc::clone(&metering_dyn),
    ));
    let multi: Arc<dyn NanoAgentExecutor> = Arc::new(
        MultiModeRuntime::mode_a_b_c(mode_a, mode_b, mode_c),
    );
    let audit_writer = Arc::new(InMemoryAuditWriter::new());
    let audit: Arc<dyn AuditWriter> = Arc::clone(&audit_writer) as _;
    let dispatcher = Dispatcher::new(
        catalog as Arc<dyn Catalog>,
        multi,
        audit,
        Arc::new(IdempotencyCache::new()),
    );

    let ops = vec![op(
        "s",
        "summarize_strict",
        &[],
        lit(serde_json::json!({"text": "x", "max_sentences": 1})),
    )];
    let resp = dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::PartialFailure);

    let tenant = TenantId::system();
    // 0 metered events on retry-exhaustion (matches failure semantic).
    assert_eq!(metering.nano_agent_calls(tenant), 0);
    assert_eq!(metering.nano_agent_llm_tokens(tenant), 0);
    // Reservation released → 0 cents charged.
    let snap = budget.snapshot(tenant).await.unwrap();
    assert_eq!(snap.reserved_cents, 0.0);
    assert_eq!(snap.used_cents, 0.0);
}
