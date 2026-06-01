//! Pin tests for CircuitBreaker (M21 Phase A).

use efe_llm_router::circuit_breaker::{CircuitBreaker, CircuitState};
use std::time::Duration;

#[test]
fn test_initial_state_closed() {
    let cb = CircuitBreaker::defaults();
    assert!(cb.is_closed("anthropic"));
    assert_eq!(cb.state("anthropic"), CircuitState::Closed);
}

#[test]
fn test_below_threshold_stays_closed() {
    let cb = CircuitBreaker::new(3, Duration::from_secs(60));
    cb.record_failure("p1");
    cb.record_failure("p1");
    assert!(cb.is_closed("p1"));
}

#[test]
fn test_threshold_opens() {
    let cb = CircuitBreaker::new(3, Duration::from_secs(60));
    for _ in 0..3 {
        cb.record_failure("p1");
    }
    assert_eq!(cb.state("p1"), CircuitState::Open);
    assert!(!cb.is_closed("p1"));
}

#[test]
fn test_success_resets_counter() {
    let cb = CircuitBreaker::new(3, Duration::from_secs(60));
    cb.record_failure("p1");
    cb.record_failure("p1");
    cb.record_success("p1");
    cb.record_failure("p1");
    cb.record_failure("p1");
    // After success, 2 failures < 3 → still closed.
    assert!(cb.is_closed("p1"));
}

#[test]
fn test_open_to_half_open_after_duration() {
    let cb = CircuitBreaker::new(2, Duration::from_millis(50));
    cb.record_failure("p1");
    cb.record_failure("p1");
    assert!(!cb.is_closed("p1"));
    std::thread::sleep(Duration::from_millis(60));
    // First is_closed call after duration → transitions to HalfOpen.
    assert!(cb.is_closed("p1"));
    assert_eq!(cb.state("p1"), CircuitState::HalfOpen);
}

#[test]
fn test_half_open_success_returns_closed() {
    let cb = CircuitBreaker::new(2, Duration::from_millis(50));
    cb.record_failure("p1");
    cb.record_failure("p1");
    std::thread::sleep(Duration::from_millis(60));
    cb.is_closed("p1"); // → HalfOpen
    cb.record_success("p1");
    assert_eq!(cb.state("p1"), CircuitState::Closed);
}

#[test]
fn test_isolation_per_provider() {
    let cb = CircuitBreaker::new(2, Duration::from_secs(60));
    cb.record_failure("p1");
    cb.record_failure("p1");
    // p1 should now be open ; p2 untouched.
    assert!(!cb.is_closed("p1"));
    assert!(cb.is_closed("p2"));
}

#[test]
fn test_states_snapshot_returns_all_known() {
    let cb = CircuitBreaker::defaults();
    cb.register("anthropic");
    cb.register("openai");
    let snapshot = cb.states_snapshot();
    assert_eq!(snapshot.len(), 2);
    assert_eq!(snapshot["anthropic"], CircuitState::Closed);
    assert_eq!(snapshot["openai"], CircuitState::Closed);
}
