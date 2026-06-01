//! PR #16 § 23 — TLS audit event types + emitter trait.
//!
//! ## Architecture
//!
//! The architecture (§ 23) calls for every TLS handshake (success or
//! failure) to be appended to the per-tenant or infra-level audit
//! chain.  The audit chain itself — the `audit_events` Postgres
//! table, the HMAC-chained `hmac_chain_link` column, the per-tenant
//! signing keys — is **not yet wired in Phase 2** (it lands in a
//! later PR ; see roadmap WS-Compliance).
//!
//! Rather than block PR #16 on the audit infra, we define the event
//! types and an [`TlsAuditEmitter`] trait here.  Today the default
//! `NoopAuditEmitter` is a no-op ; when the audit chain lands, a
//! `PgAuditEmitter` will plug in via `set_default_emitter`.
//!
//! This module is therefore a **forward-compat stub** : it locks in
//! the API surface (event types, emit-trait) so the audit chain PR
//! is purely additive.
//!
//! Sanitisation policy (§ 23 / T43 — PII in handshake logs) :
//!
//!   * `peer_subject` is the cert's Common Name only ; SANs are NOT
//!     logged (a participant SAN like
//!     `participant-1.efe.svc.cluster.local` is structural, but
//!     custom additionalSans may contain customer-specific names).
//!   * `cipher_suite` + `protocol_version` are non-PII (IANA-listed).
//!   * No raw cert bytes are logged.

use std::sync::Arc;

/// One TLS-related event emitted into the audit chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TlsAuditEvent {
    /// A peer completed a TLS handshake (success or failure).
    Handshake {
        /// Whether the handshake succeeded.
        success: bool,
        /// Peer cert Common Name (sanitised — no SANs, no full DN).
        peer_subject_cn: String,
        /// IANA cipher suite name (e.g. `"TLS_AES_128_GCM_SHA256"`).
        cipher_suite: String,
        /// Protocol version (e.g. `"TLSv1.3"`).
        protocol_version: String,
        /// Server (we accepted) or client (we dialed).
        role: AuditRole,
        /// Failure reason if `success=false` ; `None` otherwise.
        failure_reason: Option<String>,
    },
    /// The reloader swapped the in-memory config.  Fires once per
    /// successful inotify-driven rotation.
    Reload {
        /// Whether the reload succeeded.
        success: bool,
        /// Path that triggered the reload (e.g. `/etc/efe-meta-swarm/tls/tls.crt`).
        triggering_path: String,
        /// New cert's Common Name (sanitised).
        new_cert_cn: Option<String>,
    },
}

/// Whether we are accepting or dialing the connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditRole {
    /// We accepted the connection.
    Server,
    /// We initiated the connection.
    Client,
}

impl AuditRole {
    /// Stable string form for the audit-chain `role` column.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Client => "client",
        }
    }
}

impl TlsAuditEvent {
    /// Stable event-type string for the audit-chain `event_type`
    /// column.
    #[must_use]
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::Handshake { .. } => "tls_handshake",
            Self::Reload { .. } => "tls_reload",
        }
    }
}

/// Trait every audit-chain emitter implements.
///
/// `emit` is **best-effort** : it MUST NOT block the TLS reload /
/// handshake path.  Real implementations queue the event onto an
/// async sender and process it on a background task.
pub trait TlsAuditEmitter: Send + Sync + std::fmt::Debug {
    /// Best-effort emission ; never blocks, never panics.
    fn emit(&self, event: TlsAuditEvent);
}

/// Default no-op emitter ; used until the real audit chain plumbing
/// lands.  Drops every event silently — by design.
#[derive(Debug, Default)]
pub struct NoopAuditEmitter;

impl TlsAuditEmitter for NoopAuditEmitter {
    fn emit(&self, _event: TlsAuditEvent) {
        // PR #16 stub : intentionally silent.  Future PR will
        // replace this with `PgAuditEmitter` that writes to the
        // `audit_events` table.
    }
}

/// In-memory audit-event recorder used by tests : captures every
/// event into a `Mutex<Vec<...>>`.
#[derive(Debug, Default)]
pub struct MemoryAuditEmitter {
    events: std::sync::Mutex<Vec<TlsAuditEvent>>,
}

impl MemoryAuditEmitter {
    /// Drain every event recorded so far.
    pub fn drain(&self) -> Vec<TlsAuditEvent> {
        let mut g = self.events.lock().expect("audit lock poisoned");
        std::mem::take(&mut *g)
    }

    /// Count of events seen.
    pub fn len(&self) -> usize {
        self.events.lock().map(|g| g.len()).unwrap_or(0)
    }

    /// True if no events have been recorded.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl TlsAuditEmitter for MemoryAuditEmitter {
    fn emit(&self, event: TlsAuditEvent) {
        if let Ok(mut g) = self.events.lock() {
            g.push(event);
        }
    }
}

/// Convenience handle for callers that want a shared emitter behind
/// an `Arc`.
pub type SharedAuditEmitter = Arc<dyn TlsAuditEmitter>;

/// Build a default [`SharedAuditEmitter`] — currently a no-op.
#[must_use]
pub fn default_audit_emitter() -> SharedAuditEmitter {
    Arc::new(NoopAuditEmitter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_type_is_stable() {
        let ev = TlsAuditEvent::Handshake {
            success: true,
            peer_subject_cn: "controller.efe.svc".into(),
            cipher_suite: "TLS_AES_128_GCM_SHA256".into(),
            protocol_version: "TLSv1.3".into(),
            role: AuditRole::Server,
            failure_reason: None,
        };
        assert_eq!(ev.event_type(), "tls_handshake");

        let ev = TlsAuditEvent::Reload {
            success: true,
            triggering_path: "/etc/efe-meta-swarm/tls/tls.crt".into(),
            new_cert_cn: Some("controller.efe.svc".into()),
        };
        assert_eq!(ev.event_type(), "tls_reload");
    }

    #[test]
    fn audit_role_strings_are_stable() {
        assert_eq!(AuditRole::Server.as_str(), "server");
        assert_eq!(AuditRole::Client.as_str(), "client");
    }

    #[test]
    fn noop_emitter_drops_silently() {
        let e: SharedAuditEmitter = Arc::new(NoopAuditEmitter);
        e.emit(TlsAuditEvent::Reload {
            success: false,
            triggering_path: "/dev/null".into(),
            new_cert_cn: None,
        });
        // No panic, no observable effect — that's the contract.
    }

    #[test]
    fn memory_emitter_records_events() {
        let e = MemoryAuditEmitter::default();
        assert!(e.is_empty());

        e.emit(TlsAuditEvent::Handshake {
            success: true,
            peer_subject_cn: "controller.efe.svc".into(),
            cipher_suite: "TLS_AES_128_GCM_SHA256".into(),
            protocol_version: "TLSv1.3".into(),
            role: AuditRole::Server,
            failure_reason: None,
        });
        e.emit(TlsAuditEvent::Reload {
            success: true,
            triggering_path: "/etc/efe-meta-swarm/tls/tls.crt".into(),
            new_cert_cn: Some("controller.efe.svc".into()),
        });

        assert_eq!(e.len(), 2);
        let drained = e.drain();
        assert_eq!(drained.len(), 2);
        assert!(e.is_empty(), "drain must clear");
    }

    #[test]
    fn default_emitter_is_noop() {
        let e = default_audit_emitter();
        // No assertion : it just shouldn't panic.
        e.emit(TlsAuditEvent::Reload {
            success: true,
            triggering_path: "/dev/null".into(),
            new_cert_cn: None,
        });
    }
}
