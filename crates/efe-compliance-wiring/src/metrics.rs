// SPDX-License-Identifier: UNLICENSED
//! **Sprint 16 — N24 + Annex A5** : Prometheus-style counters for
//! the wired compliance stack.
//!
//! Sprint 16 keeps the metrics surface small + in-process : every
//! field is an `AtomicU64` counter that the gateway's
//! `/v1/admin/compliance-status` endpoint reads via
//! [`N24ComplianceMetrics::snapshot`].  Sprint 17 wires real
//! Prometheus exposition via the N22 `/metrics` endpoint.
//!
//! ## Counter labels (annex A5)
//!
//! | Counter                          | Outcome labels                                  |
//! |----------------------------------|-------------------------------------------------|
//! | `signature_verified_total`       | `ok`, `verify_failed`, `author_mismatch`,       |
//! |                                  | `publisher_not_found`, `key_not_found`,         |
//! |                                  | `signature_missing`, `grace_skip`, `other`      |
//! | `gold_bound_total`               | `ok`, `not_approved`, `hash_drift`,             |
//! |                                  | `lookup_failed`, `no_ref`, `grace_skip`,        |
//! |                                  | `other`                                          |
//! | `status_blocked_total`           | `ok`, `active`, `suspended`, `revoked`,         |
//! |                                  | `sanctions_check_stale`, `not_publisher`,       |
//! |                                  | `publisher_not_found`, `registry_timeout`       |
//! | `cache_hit_ratio`                | (gauge — derived from hit + miss counters)      |
//! | `lookup_latency_ms`              | (histogram — Sprint 17)                          |

use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

/// **N24** : per-runner compliance metrics ; cheap to clone (only
/// holds atomic counters).
#[derive(Default)]
pub struct N24ComplianceMetrics {
    sig_ok: AtomicU64,
    sig_verify_failed: AtomicU64,
    sig_author_mismatch: AtomicU64,
    sig_publisher_not_found: AtomicU64,
    sig_key_not_found: AtomicU64,
    sig_missing: AtomicU64,
    sig_grace_skip: AtomicU64,
    sig_other: AtomicU64,

    gold_ok: AtomicU64,
    gold_not_approved: AtomicU64,
    gold_hash_drift: AtomicU64,
    gold_lookup_failed: AtomicU64,
    gold_no_ref: AtomicU64,
    gold_grace_skip: AtomicU64,
    gold_other: AtomicU64,

    status_ok: AtomicU64,
    status_suspended: AtomicU64,
    status_revoked: AtomicU64,
    status_sanctions_check_stale: AtomicU64,
    status_not_publisher: AtomicU64,
    status_publisher_not_found: AtomicU64,
    status_registry_timeout: AtomicU64,
}

impl std::fmt::Debug for N24ComplianceMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("N24ComplianceMetrics")
            .field("snapshot", &self.snapshot())
            .finish()
    }
}

impl N24ComplianceMetrics {
    /// **N24** : construct empty metrics.
    pub fn new() -> Self {
        Self::default()
    }

    // ─── Signature gate ───────────────────────────────────────────

    /// **N24** : record a signature-gate outcome.
    pub fn signature_verified_total(&self, outcome: &str) {
        let c = match outcome {
            "ok" => &self.sig_ok,
            "verify_failed" => &self.sig_verify_failed,
            "author_mismatch" => &self.sig_author_mismatch,
            "publisher_not_found" => &self.sig_publisher_not_found,
            "key_not_found" => &self.sig_key_not_found,
            "signature_missing" => &self.sig_missing,
            "grace_skip" => &self.sig_grace_skip,
            _ => &self.sig_other,
        };
        c.fetch_add(1, Ordering::Relaxed);
    }

    // ─── Gold gate ────────────────────────────────────────────────

    /// **N24** : record a gold-gate outcome.
    pub fn gold_bound_total(&self, outcome: &str) {
        let c = match outcome {
            "ok" => &self.gold_ok,
            "not_approved" => &self.gold_not_approved,
            "hash_drift" => &self.gold_hash_drift,
            "lookup_failed" => &self.gold_lookup_failed,
            "no_ref" => &self.gold_no_ref,
            "grace_skip" => &self.gold_grace_skip,
            _ => &self.gold_other,
        };
        c.fetch_add(1, Ordering::Relaxed);
    }

    // ─── Status gate ──────────────────────────────────────────────

    /// **N24** : record a status-gate outcome.
    pub fn status_blocked_total(&self, outcome: &str) {
        let c = match outcome {
            "ok" | "active" => &self.status_ok,
            "suspended" => &self.status_suspended,
            "revoked" => &self.status_revoked,
            "sanctions_check_stale" => &self.status_sanctions_check_stale,
            "not_publisher" => &self.status_not_publisher,
            "publisher_not_found" => &self.status_publisher_not_found,
            "registry_timeout" => &self.status_registry_timeout,
            _ => &self.status_not_publisher,
        };
        c.fetch_add(1, Ordering::Relaxed);
    }

    /// **N24** : structured snapshot of every counter.  Returned
    /// via the admin endpoint.
    pub fn snapshot(&self) -> N24MetricsSnapshot {
        N24MetricsSnapshot {
            signature: SignatureSnapshot {
                ok: self.sig_ok.load(Ordering::Relaxed),
                verify_failed: self.sig_verify_failed.load(Ordering::Relaxed),
                author_mismatch: self.sig_author_mismatch.load(Ordering::Relaxed),
                publisher_not_found: self.sig_publisher_not_found.load(Ordering::Relaxed),
                key_not_found: self.sig_key_not_found.load(Ordering::Relaxed),
                missing: self.sig_missing.load(Ordering::Relaxed),
                grace_skip: self.sig_grace_skip.load(Ordering::Relaxed),
                other: self.sig_other.load(Ordering::Relaxed),
            },
            gold: GoldSnapshot {
                ok: self.gold_ok.load(Ordering::Relaxed),
                not_approved: self.gold_not_approved.load(Ordering::Relaxed),
                hash_drift: self.gold_hash_drift.load(Ordering::Relaxed),
                lookup_failed: self.gold_lookup_failed.load(Ordering::Relaxed),
                no_ref: self.gold_no_ref.load(Ordering::Relaxed),
                grace_skip: self.gold_grace_skip.load(Ordering::Relaxed),
                other: self.gold_other.load(Ordering::Relaxed),
            },
            status: StatusSnapshot {
                ok: self.status_ok.load(Ordering::Relaxed),
                suspended: self.status_suspended.load(Ordering::Relaxed),
                revoked: self.status_revoked.load(Ordering::Relaxed),
                sanctions_check_stale: self
                    .status_sanctions_check_stale
                    .load(Ordering::Relaxed),
                not_publisher: self.status_not_publisher.load(Ordering::Relaxed),
                publisher_not_found: self
                    .status_publisher_not_found
                    .load(Ordering::Relaxed),
                registry_timeout: self.status_registry_timeout.load(Ordering::Relaxed),
            },
        }
    }
}

/// **N24** : flat snapshot of every counter ; serde-friendly so
/// the admin endpoint can return it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct N24MetricsSnapshot {
    /// Signature-gate breakdown.
    pub signature: SignatureSnapshot,
    /// Gold-gate breakdown.
    pub gold: GoldSnapshot,
    /// Status-gate breakdown.
    pub status: StatusSnapshot,
}

/// **N24** : signature-gate counter snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignatureSnapshot {
    /// Successful verifications.
    pub ok: u64,
    /// Verifier returned an error.
    pub verify_failed: u64,
    /// `author.publisher_id != signature.publisher_id`.
    pub author_mismatch: u64,
    /// Publisher id unknown to the registry.
    pub publisher_not_found: u64,
    /// Signature's key id matches no `(active + historical)` key.
    pub key_not_found: u64,
    /// Catalogue entry has no signature attached.
    pub missing: u64,
    /// Tenant is in the grace window ; signature skipped.
    pub grace_skip: u64,
    /// Any other catch-all.
    pub other: u64,
}

/// **N24** : gold-gate counter snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoldSnapshot {
    /// Status `Approved` + hash match.
    pub ok: u64,
    /// Status is not `Approved`.
    pub not_approved: u64,
    /// `nano.gold_set_ref.hash != registry.gold_set.hash`.
    pub hash_drift: u64,
    /// `GoldApi::lookup_gold_set` returned an error.
    pub lookup_failed: u64,
    /// `nano.gold_set_ref` is `None`.
    pub no_ref: u64,
    /// Tenant is in the grace window ; gold skipped.
    pub grace_skip: u64,
    /// Any other catch-all.
    pub other: u64,
}

/// **N24** : status-gate counter snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusSnapshot {
    /// Publisher status is `Active` (or not a publisher).
    pub ok: u64,
    /// Publisher is `Suspended`.
    pub suspended: u64,
    /// Publisher is `Revoked`.
    pub revoked: u64,
    /// Publisher sanctions check is stale.
    pub sanctions_check_stale: u64,
    /// Catalogue author is not a publisher (no check applicable).
    pub not_publisher: u64,
    /// Publisher id unknown to the registry.
    pub publisher_not_found: u64,
    /// Registry lookup timed out.
    pub registry_timeout: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_metrics_zero() {
        let m = N24ComplianceMetrics::new();
        let s = m.snapshot();
        assert_eq!(s.signature.ok, 0);
        assert_eq!(s.gold.ok, 0);
        assert_eq!(s.status.ok, 0);
    }

    #[test]
    fn signature_ok_increments() {
        let m = N24ComplianceMetrics::new();
        m.signature_verified_total("ok");
        m.signature_verified_total("ok");
        assert_eq!(m.snapshot().signature.ok, 2);
    }

    #[test]
    fn signature_each_label_routes_to_its_counter() {
        let m = N24ComplianceMetrics::new();
        for label in [
            "ok",
            "verify_failed",
            "author_mismatch",
            "publisher_not_found",
            "key_not_found",
            "signature_missing",
            "grace_skip",
        ] {
            m.signature_verified_total(label);
        }
        let s = m.snapshot().signature;
        assert_eq!(s.ok, 1);
        assert_eq!(s.verify_failed, 1);
        assert_eq!(s.author_mismatch, 1);
        assert_eq!(s.publisher_not_found, 1);
        assert_eq!(s.key_not_found, 1);
        assert_eq!(s.missing, 1);
        assert_eq!(s.grace_skip, 1);
        assert_eq!(s.other, 0);
    }

    #[test]
    fn unknown_signature_label_routes_to_other() {
        let m = N24ComplianceMetrics::new();
        m.signature_verified_total("totally_invented_label");
        assert_eq!(m.snapshot().signature.other, 1);
    }

    #[test]
    fn gold_each_label_routes_to_its_counter() {
        let m = N24ComplianceMetrics::new();
        for label in [
            "ok",
            "not_approved",
            "hash_drift",
            "lookup_failed",
            "no_ref",
            "grace_skip",
        ] {
            m.gold_bound_total(label);
        }
        let s = m.snapshot().gold;
        assert_eq!(s.ok, 1);
        assert_eq!(s.not_approved, 1);
        assert_eq!(s.hash_drift, 1);
        assert_eq!(s.lookup_failed, 1);
        assert_eq!(s.no_ref, 1);
        assert_eq!(s.grace_skip, 1);
        assert_eq!(s.other, 0);
    }

    #[test]
    fn status_each_label_routes_to_its_counter() {
        let m = N24ComplianceMetrics::new();
        for label in [
            "ok",
            "active",
            "suspended",
            "revoked",
            "sanctions_check_stale",
            "not_publisher",
            "publisher_not_found",
            "registry_timeout",
        ] {
            m.status_blocked_total(label);
        }
        let s = m.snapshot().status;
        // "ok" and "active" route to the same counter (both = OK).
        assert_eq!(s.ok, 2);
        assert_eq!(s.suspended, 1);
        assert_eq!(s.revoked, 1);
        assert_eq!(s.sanctions_check_stale, 1);
        assert_eq!(s.not_publisher, 1);
        assert_eq!(s.publisher_not_found, 1);
        assert_eq!(s.registry_timeout, 1);
    }

    #[test]
    fn snapshot_serde_round_trip() {
        let m = N24ComplianceMetrics::new();
        m.signature_verified_total("ok");
        let s = m.snapshot();
        let json = serde_json::to_string(&s).unwrap();
        let back: N24MetricsSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn debug_format_no_panic() {
        let m = N24ComplianceMetrics::new();
        let _ = format!("{m:?}");
    }
}
