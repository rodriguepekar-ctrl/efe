//! **Sprint 7 — N9** end-to-end integration tests.
//!
//! Validates the full **3-level audit-grade verification** pipeline :
//!
//! ```text
//! Dispatcher  →  HMAC chain  →  Merkle epoch sealer  →  Notary receipt
//!     ↑                                                       ↓
//!     └──────────────  Auditor verification path  ────────────┘
//!                  (chain HMAC + Merkle root + notary sig)
//! ```
//!
//! Coverage matrix (8 tests) :
//!
//! | # | Test                                              | Property under test                                    |
//! |---|---------------------------------------------------|--------------------------------------------------------|
//! | 1 | `end_to_end_seal_and_inclusion_proof`             | dispatch → seal → inclusion proof → audit-grade verify |
//! | 2 | `seal_after_real_swarm_dispatch_works`            | real Dispatcher-produced traces seal cleanly           |
//! | 3 | `auditor_can_verify_without_chain_access`         | auditor with only proof bytes + root verifies          |
//! | 4 | `notary_round_trip_attests_epoch_root`            | InMemoryNotary issues + verifies receipts              |
//! | 5 | `tampered_trace_breaks_inclusion_proof`           | mutating a trace after sealing breaks its proof        |
//! | 6 | `verify_epoch_chain_for_real_dispatch`            | the entire chain-of-roots verifies for real traces     |
//! | 7 | `resume_sealing_after_more_traces_arrive`         | sealing state persists across calls                    |
//! | 8 | `audit_grade_log_n_inclusion_proof_size`          | proof size grows as ceil(log2(epoch_size))             |

use std::sync::Arc;
use uuid::Uuid;

use efe_nano::audit::chain_store::{ChainStore, InMemoryChainStore};
use efe_nano::audit::hkdf::HmacKeySource;
use efe_nano::audit::hmac_chain::InMemoryHmacChainAuditWriter;
use efe_nano::audit::merkle::{
    verify_inclusion, verify_trace_in_epoch, EpochSealer, EpochSealerConfig, InMemoryNotary,
    InMemoryEpochRootStore, Notary,
};
use efe_nano::audit::AuditWriter;
use efe_nano::catalog::InMemoryCatalog;
use efe_nano::dispatcher::Dispatcher;
use efe_nano::idempotency::IdempotencyCache;
use efe_nano::runtime::pure_rust::{HandlerRegistry, PureRustRuntime};
use efe_nano::runtime::NanoAgentExecutor;
use efe_nano::starter_pack::register_all_starter_pack;
use efe_nano_sdk::{
    Catalog, NanoAgentId, NanoAgentRef, Operation, OperationInput, RetryPolicy, SwarmRequest,
    SwarmStatus, TenantId,
};

// ─────────────────────────────────────────────────────────────────────
//  Harness — wires the full Sprint 7 audit-grade chain
// ─────────────────────────────────────────────────────────────────────

struct MerkleHarness {
    dispatcher: Arc<Dispatcher>,
    chain_store: Arc<InMemoryChainStore>,
    sealer: EpochSealer<InMemoryChainStore, InMemoryEpochRootStore, HmacKeySource>,
    notary: InMemoryNotary,
}

async fn make_harness(epoch_size: usize) -> MerkleHarness {
    let catalog = Arc::new(InMemoryCatalog::new());
    let registry = HandlerRegistry::new();
    register_all_starter_pack(&*catalog, &registry).await.unwrap();
    let runtime: Arc<dyn NanoAgentExecutor> = Arc::new(PureRustRuntime::new(registry));

    let chain_store = Arc::new(InMemoryChainStore::new());
    let key_source = HmacKeySource::test_key();
    let writer = Arc::new(InMemoryHmacChainAuditWriter::new(
        Arc::clone(&chain_store),
        key_source.clone(),
    ));
    let dispatcher = Arc::new(Dispatcher::new(
        catalog as Arc<dyn Catalog>,
        runtime,
        Arc::clone(&writer) as Arc<dyn AuditWriter>,
        Arc::new(IdempotencyCache::new()),
    ));
    let epoch_store = Arc::new(InMemoryEpochRootStore::new());
    let sealer = EpochSealer::new(
        Arc::clone(&chain_store),
        epoch_store,
        Arc::new(key_source),
        EpochSealerConfig {
            epoch_size_traces: epoch_size,
        },
    );
    let notary = InMemoryNotary::test();
    MerkleHarness {
        dispatcher,
        chain_store,
        sealer,
        notary,
    }
}

fn op_validate(op_id: &str) -> Operation {
    Operation {
        op_id: op_id.to_string(),
        agent: NanoAgentRef::new(NanoAgentId::new("json_validate").unwrap()),
        input: OperationInput::Literal {
            value: serde_json::json!({
                "value": {"x": 1, "_uniq": op_id},
                "schema": {"type": "object"}
            }),
        },
        depends_on: vec![],
        timeout_ms: None,
        retry: RetryPolicy::default(),
    }
}

fn make_request(tenant: TenantId, ops: Vec<Operation>) -> SwarmRequest {
    SwarmRequest {
        request_id: Uuid::new_v4(),
        tenant_id: tenant,
        caller_agent_id: Some("phase-2-merkle-epoch".to_string()),
        operations: ops,
        max_parallelism: 64,
        timeout_ms: 60_000,
        idempotency_key: None,
        streaming: false,
        audit_tags: vec!["merkle".to_string()],
        cost_cap_cents: None,
        parent_envelope_trace_id: None,
    }
}

// ─────────────────────────────────────────────────────────────────────
//  1. End-to-end : dispatch → seal → inclusion proof → audit verify
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn end_to_end_seal_and_inclusion_proof() {
    let h = make_harness(4).await;
    let tenant = TenantId::system();

    // Dispatch 4 ops → produces 4 traces in the HMAC chain.
    let request = make_request(
        tenant,
        (0..4).map(|i| op_validate(&format!("op-{i}"))).collect(),
    );
    let resp = h.dispatcher.dispatch(request).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);

    // Seal the epoch.
    let sealed = h.sealer.seal_pending_epochs(tenant).await.unwrap();
    assert_eq!(sealed.len(), 1);
    assert_eq!(sealed[0].leaf_count, 4);

    // Pick a trace, build the inclusion proof, audit-verify.
    let chain = h.chain_store.chain_for_tenant(tenant, None).await.unwrap().into_iter().filter(|e| !e.trace.is_swarm_envelope()).collect::<Vec<_>>();
    let target = &chain[2];
    let (proof, epoch) = h
        .sealer
        .inclusion_proof_for_trace(tenant, target.trace_id())
        .await
        .unwrap();
    verify_trace_in_epoch(&target.payload_hash, &proof, &epoch.root_hash).unwrap();
}

// ─────────────────────────────────────────────────────────────────────
//  2. Real-dispatch traces seal cleanly
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn seal_after_real_swarm_dispatch_works() {
    let h = make_harness(8).await;
    let tenant = TenantId::system();
    // Two swarms of 4 ops each = 8 traces total.
    for i in 0..2 {
        let r = make_request(
            tenant,
            (0..4)
                .map(|j| op_validate(&format!("op-{i}-{j}")))
                .collect(),
        );
        h.dispatcher.dispatch(r).await.unwrap();
    }
    let sealed = h.sealer.seal_pending_epochs(tenant).await.unwrap();
    assert_eq!(sealed.len(), 1);
    assert_eq!(sealed[0].leaf_count, 8);
}

// ─────────────────────────────────────────────────────────────────────
//  3. Auditor with only proof bytes + root verifies (no chain access)
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn auditor_can_verify_without_chain_access() {
    let h = make_harness(4).await;
    let tenant = TenantId::system();
    let request = make_request(
        tenant,
        (0..4).map(|i| op_validate(&format!("op-{i}"))).collect(),
    );
    h.dispatcher.dispatch(request).await.unwrap();
    h.sealer.seal_pending_epochs(tenant).await.unwrap();

    let chain = h.chain_store.chain_for_tenant(tenant, None).await.unwrap().into_iter().filter(|e| !e.trace.is_swarm_envelope()).collect::<Vec<_>>();
    let target = &chain[1];
    let (proof, epoch) = h
        .sealer
        .inclusion_proof_for_trace(tenant, target.trace_id())
        .await
        .unwrap();

    // *** Auditor scenario *** : they have ONLY :
    //   - The proof bytes (serializable to JSON).
    //   - The trace's `payload_hash`.
    //   - The epoch's `root_hash` (signed by the notary).
    // No SQL access, no key access.
    let payload_hash_copy = target.payload_hash;
    let proof_copy = proof.clone();
    let root_copy = epoch.root_hash;

    // Drop our references to prove we're not depending on internal state.
    let _ = target;
    let _ = epoch;

    // The auditor's verification : pure function call.
    verify_inclusion(&proof_copy).unwrap();
    verify_trace_in_epoch(&payload_hash_copy, &proof_copy, &root_copy).unwrap();
}

// ─────────────────────────────────────────────────────────────────────
//  4. Notary round-trip : attest an epoch root
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn notary_round_trip_attests_epoch_root() {
    let h = make_harness(4).await;
    let tenant = TenantId::system();
    let request = make_request(
        tenant,
        (0..4).map(|i| op_validate(&format!("op-{i}"))).collect(),
    );
    h.dispatcher.dispatch(request).await.unwrap();
    let sealed = h.sealer.seal_pending_epochs(tenant).await.unwrap();

    // Notarize the epoch.
    let receipt = h.notary.notarize(&sealed[0]).await.unwrap();
    // Auditor reverifies later (potentially after EFE is compromised).
    h.notary.verify(&sealed[0], &receipt).await.unwrap();

    // Tampering the root after the receipt was issued must fail.
    let mut tampered = sealed[0].clone();
    tampered.root_hash[0] ^= 0xff;
    let err = h.notary.verify(&tampered, &receipt).await.unwrap_err();
    assert!(err.to_string().contains("commitment_hash"));
}

// ─────────────────────────────────────────────────────────────────────
//  5. Tampering a trace breaks its inclusion proof
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn tampered_trace_breaks_inclusion_proof() {
    let h = make_harness(4).await;
    let tenant = TenantId::system();
    let request = make_request(
        tenant,
        (0..4).map(|i| op_validate(&format!("op-{i}"))).collect(),
    );
    h.dispatcher.dispatch(request).await.unwrap();
    h.sealer.seal_pending_epochs(tenant).await.unwrap();

    let chain = h.chain_store.chain_for_tenant(tenant, None).await.unwrap().into_iter().filter(|e| !e.trace.is_swarm_envelope()).collect::<Vec<_>>();
    let target = &chain[0];
    let (proof, epoch) = h
        .sealer
        .inclusion_proof_for_trace(tenant, target.trace_id())
        .await
        .unwrap();

    // Auditor receives a DIFFERENT (tampered) payload_hash than the
    // one the proof was built for → must reject.
    let mut tampered_payload = target.payload_hash;
    tampered_payload[0] ^= 0xff;
    let err = verify_trace_in_epoch(&tampered_payload, &proof, &epoch.root_hash)
        .unwrap_err();
    assert!(err.to_string().contains("payload"));
}

// ─────────────────────────────────────────────────────────────────────
//  6. Verify the whole chain-of-roots for a real dispatch flow
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn verify_epoch_chain_for_real_dispatch() {
    let h = make_harness(4).await;
    let tenant = TenantId::system();
    // 12 traces = 3 epochs.
    for batch in 0..3 {
        let r = make_request(
            tenant,
            (0..4)
                .map(|i| op_validate(&format!("batch-{batch}-{i}")))
                .collect(),
        );
        h.dispatcher.dispatch(r).await.unwrap();
    }
    let sealed = h.sealer.seal_pending_epochs(tenant).await.unwrap();
    assert_eq!(sealed.len(), 3);
    // Each epoch links to the previous.
    assert!(sealed[0].previous_epoch_root.is_none());
    assert_eq!(sealed[1].previous_epoch_root, Some(sealed[0].root_hash));
    assert_eq!(sealed[2].previous_epoch_root, Some(sealed[1].root_hash));
    // The cross-chain verifier rebuilds + signs + linkbacks all OK.
    h.sealer.verify_epoch_chain(tenant).await.unwrap();
}

// ─────────────────────────────────────────────────────────────────────
//  7. Resume sealing after more traces arrive
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn resume_sealing_after_more_traces_arrive() {
    let h = make_harness(4).await;
    let tenant = TenantId::system();

    // N14 : each swarm adds 1 envelope.  Round 1 dispatches 3 ops →
    // 3 ops + 1 envelope = 4 chain entries → seals 1 epoch of size 4.
    let r1 = make_request(
        tenant,
        (0..3).map(|i| op_validate(&format!("r1-{i}"))).collect(),
    );
    h.dispatcher.dispatch(r1).await.unwrap();
    let s1 = h.sealer.seal_pending_epochs(tenant).await.unwrap();
    assert_eq!(s1.len(), 1);

    // Round 2 : 3 more ops + 1 envelope = 4 more entries → seals 1 more epoch.
    let r2 = make_request(
        tenant,
        (0..3).map(|i| op_validate(&format!("r2-{i}"))).collect(),
    );
    h.dispatcher.dispatch(r2).await.unwrap();
    let s2 = h.sealer.seal_pending_epochs(tenant).await.unwrap();
    assert_eq!(s2.len(), 1);
    assert_eq!(s2[0].epoch_id, 2);
    assert_eq!(s2[0].previous_epoch_root, Some(s1[0].root_hash));

    // Round 3 : 1 op + 1 envelope = 2 entries (not a full epoch) → no auto-seal.
    let r3 = make_request(
        tenant,
        (0..1).map(|i| op_validate(&format!("r3-{i}"))).collect(),
    );
    h.dispatcher.dispatch(r3).await.unwrap();
    let s3 = h.sealer.seal_pending_epochs(tenant).await.unwrap();
    assert!(s3.is_empty(), "partial chunk must not auto-seal");

    // Force-seal the partial.
    let partial = h.sealer.seal_partial_epoch(tenant).await.unwrap().unwrap();
    assert_eq!(partial.epoch_id, 3);
    assert_eq!(partial.leaf_count, 2);
    assert_eq!(partial.previous_epoch_root, Some(s2[0].root_hash));

    // Full chain still verifies.
    h.sealer.verify_epoch_chain(tenant).await.unwrap();
}

// ─────────────────────────────────────────────────────────────────────
//  8. Audit-grade : proof size grows as ceil(log2(epoch_size))
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_grade_log_n_inclusion_proof_size() {
    // 64 traces, epoch_size=64 → ceil(log2(64)) = 6 siblings in proof.
    let h = make_harness(64).await;
    let tenant = TenantId::system();
    // Dispatch 64 ops via 16 swarms of 4 ops (json_validate uniqueness
    // via op_id so the idempotency cache doesn't skip).
    for batch in 0..16 {
        let r = make_request(
            tenant,
            (0..4)
                .map(|i| op_validate(&format!("b{batch}-i{i}")))
                .collect(),
        );
        h.dispatcher.dispatch(r).await.unwrap();
    }
    let sealed = h.sealer.seal_pending_epochs(tenant).await.unwrap();
    assert_eq!(sealed.len(), 1);
    assert_eq!(sealed[0].leaf_count, 64);

    let chain = h.chain_store.chain_for_tenant(tenant, None).await.unwrap().into_iter().filter(|e| !e.trace.is_swarm_envelope()).collect::<Vec<_>>();
    // Spot-check : proof for the middle trace should have exactly 6 siblings.
    let middle = &chain[32];
    let (proof, _epoch) = h
        .sealer
        .inclusion_proof_for_trace(tenant, middle.trace_id())
        .await
        .unwrap();
    assert_eq!(
        proof.siblings.len(),
        6,
        "expected ceil(log2(64))=6 siblings for n=64, got {}",
        proof.siblings.len()
    );
}
