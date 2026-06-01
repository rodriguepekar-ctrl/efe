//! Per-provider circuit breaker (M21 Phase A).
//!
//! State machine :
//!   CLOSED        : healthy ; calls go through.
//!   OPEN          : N consecutive failures observed ; calls skipped
//!                   for `open_duration` ; then transition to
//!                   HALF_OPEN.
//!   HALF_OPEN     : one probe call allowed ; success → CLOSED, failure
//!                   → OPEN with reset timer.

use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

impl CircuitState {
    pub fn as_str(&self) -> &'static str {
        match self {
            CircuitState::Closed => "closed",
            CircuitState::Open => "open",
            CircuitState::HalfOpen => "half_open",
        }
    }
}

#[derive(Debug, Clone)]
struct ProviderCircuit {
    state: CircuitState,
    consecutive_failures: u32,
    opened_at: Option<Instant>,
}

impl ProviderCircuit {
    fn new() -> Self {
        Self {
            state: CircuitState::Closed,
            consecutive_failures: 0,
            opened_at: None,
        }
    }
}

/// Per-provider circuit breaker registry.
///
/// Thread-safe via `RwLock`. Cheap to clone the `Arc`.
#[derive(Debug, Clone)]
pub struct CircuitBreaker {
    threshold: u32,
    open_duration: Duration,
    inner: Arc<RwLock<HashMap<String, ProviderCircuit>>>,
}

impl CircuitBreaker {
    pub fn new(threshold: u32, open_duration: Duration) -> Self {
        Self {
            threshold,
            open_duration,
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Default 5 failures → OPEN, 60s before HALF_OPEN.
    pub fn defaults() -> Self {
        Self::new(5, Duration::from_secs(60))
    }

    /// Register a provider name (idempotent).
    pub fn register(&self, name: &str) {
        let mut inner = self.inner.write();
        inner
            .entry(name.to_string())
            .or_insert_with(ProviderCircuit::new);
    }

    /// Returns true if a call should be attempted (CLOSED or HALF_OPEN).
    pub fn is_closed(&self, name: &str) -> bool {
        let mut inner = self.inner.write();
        let circuit = inner
            .entry(name.to_string())
            .or_insert_with(ProviderCircuit::new);
        match circuit.state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                if let Some(opened_at) = circuit.opened_at {
                    if opened_at.elapsed() >= self.open_duration {
                        circuit.state = CircuitState::HalfOpen;
                        return true;
                    }
                }
                false
            }
            CircuitState::HalfOpen => true,
        }
    }

    pub fn record_success(&self, name: &str) {
        let mut inner = self.inner.write();
        let circuit = inner
            .entry(name.to_string())
            .or_insert_with(ProviderCircuit::new);
        circuit.consecutive_failures = 0;
        circuit.state = CircuitState::Closed;
        circuit.opened_at = None;
    }

    pub fn record_failure(&self, name: &str) {
        let mut inner = self.inner.write();
        let circuit = inner
            .entry(name.to_string())
            .or_insert_with(ProviderCircuit::new);
        circuit.consecutive_failures += 1;
        if circuit.consecutive_failures >= self.threshold {
            circuit.state = CircuitState::Open;
            circuit.opened_at = Some(Instant::now());
        }
    }

    pub fn state(&self, name: &str) -> CircuitState {
        let inner = self.inner.read();
        inner
            .get(name)
            .map(|c| c.state)
            .unwrap_or(CircuitState::Closed)
    }

    /// Snapshot states (for `/admin/llm/cache/stats` + dashboards).
    pub fn states_snapshot(&self) -> HashMap<String, CircuitState> {
        let inner = self.inner.read();
        inner.iter().map(|(k, v)| (k.clone(), v.state)).collect()
    }

    /// Force reset (for tests + ops emergency).
    pub fn reset(&self, name: &str) {
        let mut inner = self.inner.write();
        if let Some(c) = inner.get_mut(name) {
            *c = ProviderCircuit::new();
        }
    }
}
