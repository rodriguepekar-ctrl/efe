// SPDX-License-Identifier: UNLICENSED
//! Tests d'intégration de la blackboard (Phase 2 invariants I2/I7).

use efe_coordinator::{
    BlackboardEntry, EntryKind, PheromoneBlackboard, MAX_BLACKBOARD_BYTES, MAX_ENTRIES,
};

#[test]
fn deposit_increases_byte_counter() {
    let bb = PheromoneBlackboard::new();
    assert_eq!(bb.bytes_used(), 0);
    bb.deposit(BlackboardEntry::new(
        "x",
        EntryKind::Entity,
        serde_json::json!({"k": "v"}),
    ))
    .unwrap();
    assert!(bb.bytes_used() > 0);
}

#[test]
fn invariant_i2_never_exceeds_max_bytes() {
    let bb = PheromoneBlackboard::new();
    for i in 0..100 {
        let entry = BlackboardEntry::new(
            "x",
            EntryKind::Trace,
            serde_json::json!({"i": i, "pad": "x".repeat(5_000)}),
        );
        let _ = bb.deposit(entry);
    }
    assert!(bb.bytes_used() <= MAX_BLACKBOARD_BYTES);
}

#[test]
fn invariant_i7_intensity_monotone_in_read() {
    let bb = PheromoneBlackboard::new();
    let e = BlackboardEntry::with_half_life(
        "x",
        EntryKind::Trace,
        serde_json::json!({"v": 1}),
        2,
    );
    bb.deposit(e).unwrap();

    let snap = bb.read_kind(EntryKind::Trace);
    assert_eq!(snap.len(), 1);
    let i0 = snap[0].1;
    std::thread::sleep(std::time::Duration::from_millis(800));
    let snap2 = bb.read_kind(EntryKind::Trace);
    if snap2.is_empty() {
        // GC peut être passé sous le seuil — c'est une décroissance vers 0, OK.
        return;
    }
    let i1 = snap2[0].1;
    assert!(
        i1 <= i0,
        "intensity must be monotone non-increasing: {i0} → {i1}"
    );
}

#[test]
fn read_kind_filters_correctly() {
    let bb = PheromoneBlackboard::new();
    bb.deposit(BlackboardEntry::new(
        "x",
        EntryKind::Entity,
        serde_json::json!({"a": 1}),
    ))
    .unwrap();
    bb.deposit(BlackboardEntry::new(
        "y",
        EntryKind::Vote,
        serde_json::json!({"b": 2}),
    ))
    .unwrap();
    assert_eq!(bb.read_kind(EntryKind::Entity).len(), 1);
    assert_eq!(bb.read_kind(EntryKind::Vote).len(), 1);
    assert!(bb.read_kind(EntryKind::Verdict).is_empty());
}

#[test]
fn stats_show_max_bytes_constant() {
    let bb = PheromoneBlackboard::new();
    let s = bb.stats();
    assert_eq!(s.bytes_max, MAX_BLACKBOARD_BYTES);
}

#[test]
fn max_entries_is_documented() {
    // Sanity check : la borne est exposée à l'API.
    assert_eq!(MAX_ENTRIES, 4096);
}

#[test]
fn shared_handle_visibility() {
    let bb = PheromoneBlackboard::new();
    let bb2 = bb.clone();
    bb.deposit(BlackboardEntry::new(
        "a",
        EntryKind::Trace,
        serde_json::json!({}),
    ))
    .unwrap();
    assert_eq!(bb2.len(), 1);
}

#[test]
fn clear_resets_all() {
    let bb = PheromoneBlackboard::new();
    for _ in 0..5 {
        bb.deposit(BlackboardEntry::new(
            "x",
            EntryKind::Trace,
            serde_json::json!({}),
        ))
        .unwrap();
    }
    bb.clear();
    assert!(bb.is_empty());
    assert_eq!(bb.bytes_used(), 0);
}

#[test]
fn read_latest_returns_most_recent_by_uuid_v7() {
    let bb = PheromoneBlackboard::new();
    for i in 0..3 {
        bb.deposit(BlackboardEntry::new(
            format!("a{i}"),
            EntryKind::Verdict,
            serde_json::json!({"i": i}),
        ))
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let last = bb.read_latest(EntryKind::Verdict).unwrap();
    assert_eq!(last.author, "a2");
}
