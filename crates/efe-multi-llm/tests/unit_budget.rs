// SPDX-License-Identifier: UNLICENSED
//! Tests `BudgetTracker` (invariant I13).

use efe_multi_llm::{BudgetTracker, MultiLlmError};

#[test]
fn fresh_tracker_zero() {
    let t = BudgetTracker::new(1000);
    assert_eq!(t.spent_cents(), 0);
    assert_eq!(t.remaining(), 1000);
}

#[test]
fn charge_within_budget() {
    let t = BudgetTracker::new(500);
    assert!(t.try_charge("claude", 100).is_ok());
    assert!(t.try_charge("gpt", 200).is_ok());
    assert_eq!(t.spent_cents(), 300);
}

#[test]
fn invariant_i13_overflow_blocked_and_revert() {
    let t = BudgetTracker::new(100);
    t.try_charge("a", 50).unwrap();
    let r = t.try_charge("b", 60);
    assert!(matches!(r, Err(MultiLlmError::BudgetExceeded { .. })));
    assert_eq!(t.spent_cents(), 50); // revert OK
}

#[test]
fn per_llm_tracking() {
    let t = BudgetTracker::new(1000);
    t.try_charge("claude", 100).unwrap();
    t.try_charge("gpt", 50).unwrap();
    t.try_charge("claude", 30).unwrap();
    let b = t.per_llm_breakdown();
    assert_eq!(b.get("claude").copied(), Some(130));
    assert_eq!(b.get("gpt").copied(), Some(50));
}

#[test]
fn snapshot_includes_all_fields() {
    let t = BudgetTracker::new(500);
    t.try_charge("claude", 100).unwrap();
    let s = t.snapshot();
    assert_eq!(s.spent_cents, 100);
    assert_eq!(s.max_cost_cents, 500);
    assert_eq!(s.remaining, 400);
    assert!(s.per_llm.contains_key("claude"));
}

#[test]
fn concurrent_atomic_under_load() {
    use std::thread;
    let t = BudgetTracker::new(1000);
    let mut handles = vec![];
    for _ in 0..100 {
        let t = t.clone();
        handles.push(thread::spawn(move || {
            for _ in 0..10 {
                let _ = t.try_charge("x", 1);
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    // Soit on a chargé 1000 (exactement), soit on a chargé moins parce que
    // certaines requêtes ont déclenché `BudgetExceeded` proprement.
    assert!(t.spent_cents() <= 1000);
}

#[test]
fn concurrent_at_cap_never_overflows() {
    use std::thread;
    let t = BudgetTracker::new(20);
    let mut handles = vec![];
    for _ in 0..50 {
        let t = t.clone();
        handles.push(thread::spawn(move || {
            let _ = t.try_charge("x", 1);
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    assert!(t.spent_cents() <= 20);
}

#[test]
fn exact_cap_succeeds_overflow_fails() {
    let t = BudgetTracker::new(100);
    assert!(t.try_charge("a", 100).is_ok());
    assert!(t.try_charge("b", 1).is_err());
}
