// SPDX-License-Identifier: UNLICENSED
//! **Sprint 13 — N21** : Blackboard observers — pluggable hooks
//! that fire on every blackboard write.
//!
//! Used by :
//! - the swarm orchestrator to tick metrics on blackboard activity,
//! - the audit collector (opt-in N14 envelope per agent),
//! - tests that want to assert a specific write sequence happened.

use std::sync::Arc;

use crate::blackboard::BlackboardWrite;

/// **N21** : observer trait.  Implementations receive a notification
/// for each blackboard write (after the write is committed to
/// storage).
///
/// **Concurrency** : `on_write` is called from the writer's task.
/// Implementations MUST :
/// - be fast (sync, non-blocking) ;
/// - not panic.
///
/// Blocking I/O or `tokio::block_on` inside `on_write` will starve
/// the writer task.
pub trait BlackboardObserver: Send + Sync + 'static {
    fn on_write(&self, write: &BlackboardWrite);
}

/// **N21** : registration handle.  Holding it keeps the observer
/// alive ; dropping it unregisters.
pub struct ObserverRegistration {
    pub(crate) observer: Arc<dyn BlackboardObserver>,
}

impl std::fmt::Debug for ObserverRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ObserverRegistration")
    }
}

impl ObserverRegistration {
    pub fn new(observer: Arc<dyn BlackboardObserver>) -> Self {
        Self { observer }
    }

    pub fn observer(&self) -> &Arc<dyn BlackboardObserver> {
        &self.observer
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Bundled observers — counter + capture
// ─────────────────────────────────────────────────────────────────────

/// **N21** : a counter observer.  Increments per-write.
pub struct CountingObserver {
    counter: std::sync::atomic::AtomicU64,
}

impl Default for CountingObserver {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for CountingObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CountingObserver")
            .field("count", &self.count())
            .finish()
    }
}

impl CountingObserver {
    pub fn new() -> Self {
        Self {
            counter: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub fn count(&self) -> u64 {
        self.counter.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl BlackboardObserver for CountingObserver {
    fn on_write(&self, _write: &BlackboardWrite) {
        self.counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// **N21** : a capturing observer.  Stores every write to an
/// internal Mutex-protected Vec.  Used by tests.
pub struct CapturingObserver {
    writes: std::sync::Mutex<Vec<BlackboardWrite>>,
}

impl Default for CapturingObserver {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for CapturingObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CapturingObserver")
            .field("captured", &self.writes.lock().map(|w| w.len()).unwrap_or(0))
            .finish()
    }
}

impl CapturingObserver {
    pub fn new() -> Self {
        Self {
            writes: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub fn writes(&self) -> Vec<BlackboardWrite> {
        self.writes.lock().map(|w| w.clone()).unwrap_or_default()
    }

    pub fn len(&self) -> usize {
        self.writes.lock().map(|w| w.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl BlackboardObserver for CapturingObserver {
    fn on_write(&self, write: &BlackboardWrite) {
        if let Ok(mut w) = self.writes.lock() {
            w.push(write.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::AgentId;
    use std::time::SystemTime;

    fn write(pointer: &str) -> BlackboardWrite {
        BlackboardWrite {
            pointer: pointer.into(),
            version: 1,
            agent_id: AgentId::new("test-agent").unwrap(),
            at: SystemTime::now(),
            bytes: 10,
        }
    }

    #[test]
    fn counting_observer_increments() {
        let o = CountingObserver::new();
        assert_eq!(o.count(), 0);
        o.on_write(&write("/x"));
        o.on_write(&write("/y"));
        o.on_write(&write("/z"));
        assert_eq!(o.count(), 3);
    }

    #[test]
    fn capturing_observer_records_each_write() {
        let o = CapturingObserver::new();
        o.on_write(&write("/a"));
        o.on_write(&write("/b"));
        let captured = o.writes();
        assert_eq!(captured.len(), 2);
        assert_eq!(captured[0].pointer, "/a");
        assert_eq!(captured[1].pointer, "/b");
    }

    #[test]
    fn capturing_observer_len() {
        let o = CapturingObserver::new();
        assert_eq!(o.len(), 0);
        o.on_write(&write("/a"));
        assert_eq!(o.len(), 1);
    }

    #[test]
    fn observer_registration_holds_observer() {
        let o = Arc::new(CountingObserver::new());
        let reg = ObserverRegistration::new(o.clone());
        // Calling on_write via the registration handle ticks.
        reg.observer().on_write(&write("/x"));
        assert_eq!(o.count(), 1);
    }

    #[test]
    fn counting_observer_debug_format_does_not_panic() {
        let o = CountingObserver::new();
        let _ = format!("{o:?}");
    }

    #[test]
    fn capturing_observer_debug_format_does_not_panic() {
        let o = CapturingObserver::new();
        let _ = format!("{o:?}");
    }
}
