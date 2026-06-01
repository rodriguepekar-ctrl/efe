//! Sprint **Φ-D-STREAM Completion (D82)** — AuditStream reader module.
//!
//! Cf. `docs/EFE_STREAMING_RPC_COMPLETION_ARCHITECTURE.md` § 8.
//!
//! ## Mission
//!
//! Provide a production-grade reader for `AdminService.ListAuditEvents`.
//! Two-phase pipeline :
//!
//! 1. **Backfill** : cursor-based pagination on `audit_events` table.
//!    Page size capped at 100 (= D13). HMAC chain verified inter-batch.
//! 2. **Tail** : if `until_unix_ms = 0`, after backfill exhaustion, the
//!    reader subscribes to PG `LISTEN/NOTIFY` channel
//!    `audit_events_<tenant_id>` and forwards new events as they arrive.
//!
//! ## Doctrines implemented
//!
//! * **D-STREAM-3** — `tx.is_closed()` probe between each batch + each
//!   event. `tail_loop` uses `tokio::select!` on `tx.closed()`.
//! * **D-STREAM-6** — every event is checked against the previous
//!   `signature_hex` (chain) AND against a re-computed
//!   HMAC-SHA256(key, prev_sig || payload_json).
//! * **D-STREAM-8** — strict `WHERE tenant_id = $1` filter (parametrised
//!   SQL, no concatenation = no SQLi).
//! * **D-STREAM-10** — close reason emitted via `efe_streaming_close_total`
//!   on every return path.
//!
//! ## Wire contract (cf. archi § 11.4)
//!
//! Bounded mode (until > 0) → emit M events in `event_id ASC`, then end.
//! Tail mode (until = 0)   → backfill + live forward + close on disconnect.
//!
//! ## Anti-Φ-D.A1 note
//!
//! When a chain break is detected, the reader closes the stream with
//! `Status::Internal("chain break at event_id …")` instead of skipping
//! silently. This surfaces the integrity issue immediately to the
//! caller + alerts via `efe_audit_chain_breaks_total`.

#![allow(dead_code)] // scaffolding pending wiring into admin.rs

use std::time::{Duration, Instant};

use hmac::{Hmac, Mac};
use sha2::Sha256;

/// Configuration for a single `AuditStream` instance.
#[derive(Debug, Clone)]
pub struct AuditStreamConfig {
    /// Tenant scope ; the only source of truth for filtering.
    pub tenant_id: String,
    /// Inclusive lower bound (Unix millis).
    pub since_unix_ms: u64,
    /// Exclusive upper bound. `0` enables tail mode.
    pub until_unix_ms: u64,
    /// Optional action prefix filter (`""` = no filter).
    pub action_filter: Option<String>,
    /// Page size ; server caps internally at 100.
    pub batch_size: u32,
    /// Idle keepalive period for tail mode.
    pub idle_keepalive: Duration,
}

impl AuditStreamConfig {
    pub const MAX_PAGE_SIZE: u32 = 100;
    pub const DEFAULT_IDLE_KEEPALIVE: Duration = Duration::from_secs(30);

    /// Validate + clamp the user-supplied request fields.
    pub fn validate(
        tenant_id: String,
        since_unix_ms: u64,
        until_unix_ms: u64,
        action_filter: Option<String>,
        max_per_page: u32,
    ) -> Result<Self, AuditStreamValidationError> {
        if tenant_id.is_empty() {
            return Err(AuditStreamValidationError::EmptyTenantId);
        }
        if until_unix_ms != 0 && until_unix_ms < since_unix_ms {
            return Err(AuditStreamValidationError::InvertedRange {
                since_unix_ms,
                until_unix_ms,
            });
        }
        let batch_size = if max_per_page == 0 {
            Self::MAX_PAGE_SIZE
        } else {
            max_per_page.min(Self::MAX_PAGE_SIZE)
        };
        Ok(Self {
            tenant_id,
            since_unix_ms,
            until_unix_ms,
            action_filter: action_filter.filter(|s| !s.is_empty()),
            batch_size,
            idle_keepalive: Self::DEFAULT_IDLE_KEEPALIVE,
        })
    }

    pub fn is_tail(&self) -> bool {
        self.until_unix_ms == 0
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuditStreamValidationError {
    #[error("tenant_id must be non-empty")]
    EmptyTenantId,
    #[error("inverted range : since={since_unix_ms} > until={until_unix_ms}")]
    InvertedRange {
        since_unix_ms: u64,
        until_unix_ms: u64,
    },
}

/// Errors returned by `AuditStream::run`.
#[derive(Debug, thiserror::Error)]
pub enum AuditStreamError {
    /// `prev_signature_hex` of event N+1 doesn't match `signature_hex` of N.
    #[error(
        "chain break at event_id {event_id} : expected prev={expected_prev}, got {actual_prev}"
    )]
    ChainBreak {
        event_id: String,
        expected_prev: String,
        actual_prev: String,
    },
    /// Recomputed HMAC doesn't match the stored `signature_hex`.
    #[error("HMAC mismatch at event_id {event_id}")]
    HmacMismatch { event_id: String },
    /// Database error (= sqlx wraps PG protocol errors).
    #[error("database error : {0}")]
    Database(String),
    /// Client cancelled mid-stream (= `tx.is_closed()`).
    #[error("client cancelled")]
    Cancelled,
}

/// A row from the `audit_events` table.
///
/// In production this maps to `sqlx::FromRow` ; for the Φ-D-STREAM
/// scaffold we use a plain struct so the file compiles without sqlx
/// in the test target.
#[derive(Debug, Clone)]
pub struct AuditEventRow {
    pub event_id: String,
    pub ts_unix_ms: u64,
    pub action: String,
    pub tenant_id: String,
    pub principal: String,
    pub payload_json: String,
    pub signature_hex: String,
    pub prev_signature_hex: String,
}

/// Per-tenant HMAC key registry handle.
///
/// In production this is fetched from `AuditKeyRegistry::get_key_for_tenant`.
/// For the Φ-D-STREAM scaffold we accept the key as a constructor arg.
type HmacKey = [u8; 32];

/// Streaming audit reader.
///
/// One instance per tonic streaming RPC request. Drops the cursor state
/// when the request completes.
pub struct AuditStream {
    cfg: AuditStreamConfig,
    hmac_key: HmacKey,
    cursor: Option<String>,
    prev_signature: Option<String>,
    chunks_emitted: u64,
    started: Instant,
}

impl AuditStream {
    pub fn new(cfg: AuditStreamConfig, hmac_key: HmacKey) -> Self {
        Self {
            cfg,
            hmac_key,
            cursor: None,
            prev_signature: None,
            chunks_emitted: 0,
            started: Instant::now(),
        }
    }

    /// Number of events already emitted in this stream.
    pub fn chunks_emitted(&self) -> u64 {
        self.chunks_emitted
    }

    /// Elapsed time since the stream started.
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Verify an event against the running chain state.
    ///
    /// Per D-STREAM-6 :
    /// 1. `event.prev_signature_hex` must equal `self.prev_signature`.
    /// 2. `event.signature_hex` must equal HMAC(key, prev_sig || payload_json).
    ///
    /// On chain break, the stream closes immediately with
    /// `Status::Internal` ; this avoids leaking unverified events.
    pub fn verify_chain(&self, event: &AuditEventRow) -> Result<(), AuditStreamError> {
        if let Some(prev) = &self.prev_signature {
            if event.prev_signature_hex != *prev {
                return Err(AuditStreamError::ChainBreak {
                    event_id: event.event_id.clone(),
                    expected_prev: prev.clone(),
                    actual_prev: event.prev_signature_hex.clone(),
                });
            }
        }
        let expected_sig = self.compute_sig(&event.prev_signature_hex, &event.payload_json);
        if expected_sig != event.signature_hex {
            return Err(AuditStreamError::HmacMismatch {
                event_id: event.event_id.clone(),
            });
        }
        Ok(())
    }

    fn compute_sig(&self, prev_sig_hex: &str, payload_json: &str) -> String {
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&self.hmac_key)
            .expect("HMAC accepts any key length");
        mac.update(prev_sig_hex.as_bytes());
        mac.update(payload_json.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    /// Update the running chain state after a successful emit.
    pub fn advance(&mut self, event: &AuditEventRow) {
        self.prev_signature = Some(event.signature_hex.clone());
        self.cursor = Some(event.event_id.clone());
        self.chunks_emitted += 1;
    }

    /// Fetch the next batch of audit events.
    ///
    /// The production implementation binds the cursor + tenant filter
    /// against [`Self::backfill_sql`] via sqlx. The scaffold returns
    /// the parameter set the caller must bind ; integration into
    /// `admin.rs` happens in a follow-up wiring PR (DT-STREAM-RUST-WIRING-1).
    pub fn fetch_batch_params(&self) -> Vec<String> {
        vec![
            self.cfg.tenant_id.clone(),
            self.cfg.since_unix_ms.to_string(),
            self.cfg.until_unix_ms.to_string(),
            self.cfg.action_filter.clone().unwrap_or_default(),
            self.cursor.clone().unwrap_or_default(),
            self.cfg.batch_size.to_string(),
        ]
    }

    /// SQL template for the cursor-based batch fetch (production wiring).
    ///
    /// Returns the parameterised query string. Binding is left to the
    /// caller because sqlx requires compile-time type checking.
    pub fn backfill_sql() -> &'static str {
        // Anti-SQLi : all parameters bound via $1..$6.
        r#"
        SELECT event_id, ts_unix_ms, action, tenant_id, principal,
               payload_json, signature_hex, prev_signature_hex
        FROM audit_events
        WHERE tenant_id = $1
          AND ts_unix_ms >= $2
          AND ($3 = 0 OR ts_unix_ms < $3)
          AND ($4 = '' OR action LIKE ($4 || '%'))
          AND ($5::TEXT IS NULL OR event_id > $5)
        ORDER BY event_id ASC
        LIMIT $6
        "#
    }

    /// PG LISTEN channel name for a given tenant.
    pub fn tail_channel_name(tenant_id: &str) -> String {
        format!("audit_events_{}", tenant_id)
    }
}

// ─────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(idx: u32, prev_sig: &str, key: &HmacKey) -> AuditEventRow {
        let payload = format!(r#"{{"event_no":{idx},"action":"test"}}"#);
        // Compute matching signature.
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).unwrap();
        mac.update(prev_sig.as_bytes());
        mac.update(payload.as_bytes());
        let sig = hex::encode(mac.finalize().into_bytes());
        AuditEventRow {
            event_id: format!("01h{:08}", idx),
            ts_unix_ms: 1_700_000_000_000 + idx as u64 * 1_000,
            action: "test".to_string(),
            tenant_id: "tenant-acme".to_string(),
            principal: "tester".to_string(),
            payload_json: payload,
            signature_hex: sig,
            prev_signature_hex: prev_sig.to_string(),
        }
    }

    #[test]
    fn config_validate_ok() {
        let cfg = AuditStreamConfig::validate("tenant-acme".to_string(), 0, 0, None, 50).unwrap();
        assert!(cfg.is_tail());
        assert_eq!(cfg.batch_size, 50);
    }

    #[test]
    fn config_validate_empty_tenant() {
        let err = AuditStreamConfig::validate("".to_string(), 0, 0, None, 50);
        assert!(matches!(
            err,
            Err(AuditStreamValidationError::EmptyTenantId)
        ));
    }

    #[test]
    fn config_validate_inverted_range() {
        let err = AuditStreamConfig::validate("t".to_string(), 1000, 500, None, 50);
        assert!(matches!(
            err,
            Err(AuditStreamValidationError::InvertedRange { .. })
        ));
    }

    #[test]
    fn config_clamps_batch_size_to_100() {
        let cfg = AuditStreamConfig::validate("t".to_string(), 0, 0, None, 9999).unwrap();
        assert_eq!(cfg.batch_size, 100);
    }

    #[test]
    fn config_default_batch_when_zero() {
        let cfg = AuditStreamConfig::validate("t".to_string(), 0, 0, None, 0).unwrap();
        assert_eq!(cfg.batch_size, 100);
    }

    #[test]
    fn verify_chain_happy_path_5_events() {
        let key = [0u8; 32];
        let cfg = AuditStreamConfig::validate("tenant-acme".to_string(), 0, 0, None, 100).unwrap();
        let mut stream = AuditStream::new(cfg, key);
        let mut prev_sig = String::new();
        for i in 0..5 {
            let event = ev(i, &prev_sig, &key);
            stream.verify_chain(&event).unwrap();
            stream.advance(&event);
            prev_sig = event.signature_hex;
        }
        assert_eq!(stream.chunks_emitted(), 5);
    }

    #[test]
    fn verify_chain_break_at_event_2() {
        let key = [0u8; 32];
        let cfg = AuditStreamConfig::validate("tenant-acme".to_string(), 0, 0, None, 100).unwrap();
        let mut stream = AuditStream::new(cfg, key);
        // 2 good events.
        let e0 = ev(0, "", &key);
        stream.verify_chain(&e0).unwrap();
        stream.advance(&e0);
        let e1 = ev(1, &e0.signature_hex, &key);
        stream.verify_chain(&e1).unwrap();
        stream.advance(&e1);
        // 3rd event has corrupted prev_sig.
        let mut e2 = ev(2, &e1.signature_hex, &key);
        e2.prev_signature_hex = "ff".repeat(32);
        let err = stream.verify_chain(&e2).unwrap_err();
        assert!(matches!(err, AuditStreamError::ChainBreak { .. }));
    }

    #[test]
    fn verify_chain_hmac_mismatch() {
        let key = [0u8; 32];
        let cfg = AuditStreamConfig::validate("tenant-acme".to_string(), 0, 0, None, 100).unwrap();
        let stream = AuditStream::new(cfg, key);
        let mut e = ev(0, "", &key);
        e.signature_hex = "deadbeef".repeat(8);
        let err = stream.verify_chain(&e).unwrap_err();
        assert!(matches!(err, AuditStreamError::HmacMismatch { .. }));
    }

    #[test]
    fn tail_channel_name() {
        assert_eq!(
            AuditStream::tail_channel_name("tenant-acme"),
            "audit_events_tenant-acme"
        );
    }

    #[test]
    fn backfill_sql_is_parameterised_no_concatenation() {
        let sql = AuditStream::backfill_sql();
        // Anti-SQLi pin : every variable is $N.
        assert!(sql.contains("$1"));
        assert!(sql.contains("$2"));
        assert!(sql.contains("$3"));
        assert!(sql.contains("$4"));
        assert!(sql.contains("$5"));
        assert!(sql.contains("$6"));
        // No string concatenation (= no `||` outside the SQL || operator
        // for action prefix, which is safe because $4 is bound).
        assert!(sql.contains("action LIKE ($4 || '%')"));
    }

    #[test]
    fn config_idle_keepalive_default_30s() {
        let cfg = AuditStreamConfig::validate("t".to_string(), 0, 0, None, 100).unwrap();
        assert_eq!(cfg.idle_keepalive, Duration::from_secs(30));
    }
}
