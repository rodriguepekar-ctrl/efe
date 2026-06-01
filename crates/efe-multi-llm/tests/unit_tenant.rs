// SPDX-License-Identifier: UNLICENSED
//! Tests `TenantPheromoneBlackboard` (invariant I12).

use efe_coordinator::{BlackboardEntry, EntryKind};
use efe_multi_llm::TenantPheromoneBlackboard;
use uuid::Uuid;

#[test]
fn invariant_i12_tenants_strictly_isolated() {
    let bb = TenantPheromoneBlackboard::new();
    let t_a = Uuid::now_v7();
    let t_b = Uuid::now_v7();
    let board_a = bb.for_tenant(t_a);
    let board_b = bb.for_tenant(t_b);
    board_a
        .deposit(BlackboardEntry::new(
            "x",
            EntryKind::Verdict,
            serde_json::json!({"secret": "tenant-A-only"}),
        ))
        .unwrap();
    // B ne voit RIEN de A.
    assert_eq!(board_b.read_kind(EntryKind::Verdict).len(), 0);
    assert_eq!(board_a.read_kind(EntryKind::Verdict).len(), 1);
}

#[test]
fn same_tenant_same_blackboard_handle() {
    let bb = TenantPheromoneBlackboard::new();
    let t = Uuid::now_v7();
    let a = bb.for_tenant(t);
    a.deposit(BlackboardEntry::new("x", EntryKind::Trace, serde_json::json!({})))
        .unwrap();
    let b = bb.for_tenant(t);
    assert_eq!(b.len(), 1);
}

#[test]
fn drop_tenant_clears_data() {
    let bb = TenantPheromoneBlackboard::new();
    let t = Uuid::now_v7();
    let board = bb.for_tenant(t);
    board
        .deposit(BlackboardEntry::new(
            "x",
            EntryKind::Trace,
            serde_json::json!({}),
        ))
        .unwrap();
    assert_eq!(bb.tenant_count(), 1);
    bb.drop_tenant(t);
    assert_eq!(bb.tenant_count(), 0);
}

#[test]
fn total_bytes_aggregates_across_tenants() {
    let bb = TenantPheromoneBlackboard::new();
    for _ in 0..3 {
        let board = bb.for_tenant(Uuid::now_v7());
        board
            .deposit(BlackboardEntry::new(
                "x",
                EntryKind::Trace,
                serde_json::json!({"pad": "x".repeat(500)}),
            ))
            .unwrap();
    }
    assert!(bb.total_bytes_used() > 1500);
}

#[test]
fn tenant_ids_enumerable() {
    let bb = TenantPheromoneBlackboard::new();
    let t1 = Uuid::now_v7();
    let t2 = Uuid::now_v7();
    let _ = bb.for_tenant(t1);
    let _ = bb.for_tenant(t2);
    let ids = bb.tenant_ids();
    assert!(ids.contains(&t1));
    assert!(ids.contains(&t2));
}
