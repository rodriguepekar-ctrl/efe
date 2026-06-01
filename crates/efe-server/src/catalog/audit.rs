//! Audit-log emission for every catalog mutation.
//!
//! Architecture document § 13 — every register / evolve / delete
//! emits a structured `tracing::info!(target: "audit", ...)` event
//! with a stable schema. Operators redirect that target to a
//! durable sink via `EFE_AUDIT_SINK` (see
//! `crates/efe-telemetry/src/audit.rs` — to be added in D12-bis).
//!
//! Why a separate emitter rather than ad-hoc `info!` calls in
//! `memory.rs` ?
//!
//!   * Single point of evolution for the audit schema.
//!   * Easier to test : the emitter is injectable, so unit tests
//!     can capture events.
//!   * Future routing flexibility : we can swap to a JSON-line
//!     file appender, a syslog forwarder, or kafka without
//!     touching the registry impls.

use std::sync::Arc;

use tracing::info;

use super::registry::{CatalogKey, CatalogMetadata, EvolveOp};

/// Audit sink abstraction.
///
/// The default impl emits `tracing::info!` events on the `audit`
/// target. Tests can swap in `MemoryAuditSink` to capture events
/// for assertions.
pub trait AuditSink: Send + Sync {
    fn emit(&self, event: &AuditEvent);
}

/// One audit event. Mirrors the architecture doc § 13 schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEvent {
    pub action: AuditAction,
    pub key: CatalogKey,
    pub created_by: String,
    pub description: String,
    pub parent_key: String,
    pub content_sha256: String,
    /// One of the 5 evolve op tags, or the literal string
    /// `"register"` or `"delete"` for non-evolve actions.
    pub op_tag: String,
}

/// Tag for the kind of mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditAction {
    Register,
    Evolve,
    Delete,
}

impl AuditAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Register => "register",
            Self::Evolve => "evolve",
            Self::Delete => "delete",
        }
    }
}

// ---------------------------------------------------------------------
// Emitter
// ---------------------------------------------------------------------

/// Holds an `Arc<dyn AuditSink>` so the registry impl owns one
/// emitter and shares it across every concurrent mutation.
#[derive(Clone)]
pub struct AuditEmitter {
    sink: Arc<dyn AuditSink>,
}

impl AuditEmitter {
    /// Build with the default tracing-backed sink.
    pub fn new() -> Self {
        Self {
            sink: Arc::new(TracingAuditSink),
        }
    }

    /// Build with a custom sink (used in tests).
    pub fn with_sink(sink: Arc<dyn AuditSink>) -> Self {
        Self { sink }
    }

    pub fn emit_register(&self, key: &CatalogKey, meta: &CatalogMetadata) {
        self.sink.emit(&AuditEvent {
            action: AuditAction::Register,
            key: key.clone(),
            created_by: meta.created_by.clone(),
            description: meta.description.clone(),
            parent_key: meta.parent_key.clone(),
            content_sha256: meta.content_sha256.0.clone(),
            op_tag: "register".to_owned(),
        });
    }

    pub fn emit_evolve(&self, key: &CatalogKey, meta: &CatalogMetadata, op: &EvolveOp) {
        self.sink.emit(&AuditEvent {
            action: AuditAction::Evolve,
            key: key.clone(),
            created_by: meta.created_by.clone(),
            description: meta.description.clone(),
            parent_key: meta.parent_key.clone(),
            content_sha256: meta.content_sha256.0.clone(),
            op_tag: op.audit_tag().to_owned(),
        });
    }

    pub fn emit_delete(&self, key: &CatalogKey, principal: &str, force: bool) {
        self.sink.emit(&AuditEvent {
            action: AuditAction::Delete,
            key: key.clone(),
            created_by: principal.to_owned(),
            description: if force {
                "force=true".into()
            } else {
                String::new()
            },
            parent_key: String::new(),
            content_sha256: String::new(),
            op_tag: "delete".to_owned(),
        });
    }
}

impl Default for AuditEmitter {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------
// Default tracing-backed sink
// ---------------------------------------------------------------------

/// Default tracing-backed sink. Emits one
/// `tracing::info!(target = "audit", …)` event per mutation. D12
/// behaviour ; D14-bis adds the `HmacChainedAuditSink` alternative
/// which fans out to this sink when `chain_to_tracing = true`.
pub struct TracingAuditSink;

impl AuditSink for TracingAuditSink {
    fn emit(&self, event: &AuditEvent) {
        info!(
            target: "audit",
            action = event.action.as_str(),
            tenant = %event.key.tenant,
            name = %event.key.name,
            version = event.key.version,
            created_by = %event.created_by,
            parent_key = %event.parent_key,
            op_tag = %event.op_tag,
            content_sha256 = %event.content_sha256,
            description = %event.description,
            "catalog audit",
        );
    }
}

// ---------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------

/// In-memory sink used by unit tests to capture emitted events.
#[cfg(test)]
pub mod test_sink {
    use std::sync::{Arc, Mutex};

    use super::{AuditEvent, AuditSink};

    #[derive(Default, Clone)]
    pub struct MemoryAuditSink {
        events: Arc<Mutex<Vec<AuditEvent>>>,
    }

    impl MemoryAuditSink {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn events(&self) -> Vec<AuditEvent> {
            self.events.lock().unwrap().clone()
        }
    }

    impl AuditSink for MemoryAuditSink {
        fn emit(&self, event: &AuditEvent) {
            self.events.lock().unwrap().push(event.clone());
        }
    }
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use super::test_sink::MemoryAuditSink;
    use super::*;
    use crate::catalog::registry::{
        CatalogClass, CatalogKey, CatalogMetadata, ContentDigest, EvolveOp,
    };

    fn sample_meta() -> CatalogMetadata {
        CatalogMetadata {
            created_at_unix_ms: 0,
            created_by: "alice".into(),
            description: "init".into(),
            parent_key: String::new(),
            evolution_op: "register".into(),
            labels: BTreeMap::new(),
            content_sha256: ContentDigest("deadbeef".into()),
        }
    }

    #[test]
    fn emit_register_round_trips() {
        let sink = MemoryAuditSink::new();
        let emitter = AuditEmitter::with_sink(Arc::new(sink.clone()));
        let key = CatalogKey {
            tenant: "acme".into(),
            name: "billing".into(),
            version: 1,
        };
        emitter.emit_register(&key, &sample_meta());
        let events = sink.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, AuditAction::Register);
        assert_eq!(events[0].key, key);
        assert_eq!(events[0].op_tag, "register");
        assert_eq!(events[0].created_by, "alice");
    }

    #[test]
    fn emit_evolve_carries_op_tag() {
        let sink = MemoryAuditSink::new();
        let emitter = AuditEmitter::with_sink(Arc::new(sink.clone()));
        let key = CatalogKey {
            tenant: "acme".into(),
            name: "billing".into(),
            version: 2,
        };
        let op = EvolveOp::RemoveClass {
            class_name: "spam".into(),
        };
        emitter.emit_evolve(&key, &sample_meta(), &op);
        let events = sink.events();
        assert_eq!(events[0].op_tag, "remove_class");
    }

    #[test]
    fn emit_delete_records_force_flag() {
        let sink = MemoryAuditSink::new();
        let emitter = AuditEmitter::with_sink(Arc::new(sink.clone()));
        let key = CatalogKey {
            tenant: "acme".into(),
            name: "billing".into(),
            version: 3,
        };
        emitter.emit_delete(&key, "bob", true);
        let events = sink.events();
        assert_eq!(events[0].description, "force=true");
        assert_eq!(events[0].action, AuditAction::Delete);
    }
}
