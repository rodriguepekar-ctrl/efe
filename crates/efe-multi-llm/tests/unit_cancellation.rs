// SPDX-License-Identifier: UNLICENSED
//! Tests `CancellationToken`.

use std::time::Duration;

use efe_multi_llm::{CancellationReason, CancellationToken};

#[test]
fn fresh_token_not_cancelled() {
    let t = CancellationToken::new();
    assert!(!t.is_cancelled());
    assert!(t.reason().is_none());
}

#[test]
fn cancel_idempotent() {
    let t = CancellationToken::new();
    t.cancel(CancellationReason::EarlyQuorum);
    t.cancel(CancellationReason::BudgetExceeded);
    assert_eq!(t.reason(), Some(CancellationReason::EarlyQuorum));
}

#[test]
fn clone_shares_state() {
    let t = CancellationToken::new();
    let t2 = t.clone();
    t.cancel(CancellationReason::DeadlineNear);
    assert!(t2.is_cancelled());
}

#[tokio::test]
async fn wait_returns_after_cancel() {
    let t = CancellationToken::new();
    let t_c = t.clone();
    let waiter = tokio::spawn(async move { t_c.wait_cancel().await });
    tokio::time::sleep(Duration::from_millis(10)).await;
    t.cancel(CancellationReason::SiblingFailed);
    let r = waiter.await.unwrap();
    assert_eq!(r, CancellationReason::SiblingFailed);
}

#[tokio::test]
async fn wait_returns_immediately_if_already_cancelled() {
    let t = CancellationToken::new();
    t.cancel(CancellationReason::UserDisconnect);
    let r = t.wait_cancel().await;
    assert_eq!(r, CancellationReason::UserDisconnect);
}

#[test]
fn reason_serializes_screaming() {
    assert_eq!(
        serde_json::to_string(&CancellationReason::EarlyQuorum).unwrap(),
        "\"EARLY_QUORUM\""
    );
    assert_eq!(
        serde_json::to_string(&CancellationReason::BudgetExceeded).unwrap(),
        "\"BUDGET_EXCEEDED\""
    );
}
