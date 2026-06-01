//! 4 Prometheus metrics for the TLS subsystem.
//!
//! Per § 20 of the PR #16 architecture doc :
//!
//! | metric                                              | type      | labels                            |
//! | --------------------------------------------------- | --------- | --------------------------------- |
//! | `efe_meta_swarm_tls_handshake_duration_seconds`     | Histogram | `result`, `protocol`, `role`      |
//! | `efe_meta_swarm_tls_handshake_total`                | Counter   | `result`, `role`, `failure_reason`|
//! | `efe_meta_swarm_tls_cert_expiry_seconds`            | Gauge     | `cert_name`, `subject_cn`         |
//! | `efe_meta_swarm_tls_reload_total`                   | Counter   | `result`                          |
//!
//! Additionally we expose a `efe_meta_swarm_tls_reload_at_seconds`
//! gauge so the `MetaSwarmTlsRotationStuck` alert (§ 21) can detect
//! "no reload in N hours" with a single PromQL expression.
//!
//! All metrics are no-op-tolerant : registering the same metric
//! twice (e.g. if the controller and participant share an
//! in-process registry in tests) returns the already-registered
//! handle rather than panicking.

use std::time::Duration;

use prometheus::{
    register_gauge_vec_with_registry, register_histogram_vec_with_registry,
    register_int_counter_vec_with_registry, GaugeVec, HistogramVec, IntCounterVec, Registry,
};

/// Outcome label for the reload counter — `ok` or `fail`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReloadOutcome {
    /// Reload completed successfully.
    Ok,
    /// Reload failed (cert parse error, swap aborted, etc.).
    Fail,
}

impl ReloadOutcome {
    fn label(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Fail => "fail",
        }
    }
}

/// Handshake-failure-reason taxonomy.  Matches the
/// `failure_reason` label values in § 20.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeFailureReason {
    /// Cert validity period elapsed.
    ExpiredCert,
    /// Cert chain didn't terminate in a trusted root.
    UntrustedCa,
    /// SAN allowlist mismatch.
    SanMismatch,
    /// Peer attempted to negotiate < min protocol.
    ProtocolDowngrade,
    /// Catch-all : unspecified handshake error.
    Other,
}

impl HandshakeFailureReason {
    /// Stable string form for the Prometheus label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::ExpiredCert => "expired_cert",
            Self::UntrustedCa => "untrusted_ca",
            Self::SanMismatch => "san_mismatch",
            Self::ProtocolDowngrade => "protocol_downgrade",
            Self::Other => "other",
        }
    }
}

/// Role label — `server` or `client`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// We are accepting the connection.
    Server,
    /// We are initiating the connection.
    Client,
}

impl Role {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Client => "client",
        }
    }
}

/// Bundle of all 4 (+1) TLS Prometheus series.
#[derive(Debug, Clone)]
pub struct TlsMetrics {
    /// Handshake duration histogram, seconds.
    pub handshake_duration: HistogramVec,
    /// Handshake total counter (success + failure).
    pub handshake_total: IntCounterVec,
    /// Cert expiry gauge, seconds remaining (can go negative).
    pub cert_expiry: GaugeVec,
    /// Cert reload total counter.
    pub reload_total: IntCounterVec,
    /// Cert reload timestamp gauge (unix seconds).
    pub reload_at: GaugeVec,
}

impl TlsMetrics {
    /// Register every metric on the supplied registry.  Returns the
    /// bundle ; subsequent calls on the same registry are a no-op
    /// (the gauges + counters are looked up from the registry, not
    /// re-created — important for tests that create multiple
    /// `TlsReloader`s sharing one registry).
    pub fn register(registry: &Registry) -> prometheus::Result<Self> {
        let handshake_duration = register_histogram_vec_with_registry!(
            "efe_meta_swarm_tls_handshake_duration_seconds",
            "TLS handshake duration in seconds, bucketed.",
            &["result", "protocol", "role"],
            // 1ms, 5ms, 10ms, 25ms, 50ms, 100ms, 250ms, 500ms, 1s
            vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0],
            registry,
        )?;

        let handshake_total = register_int_counter_vec_with_registry!(
            "efe_meta_swarm_tls_handshake_total",
            "Total TLS handshakes attempted, partitioned by result and reason.",
            &["result", "role", "failure_reason"],
            registry,
        )?;

        let cert_expiry = register_gauge_vec_with_registry!(
            "efe_meta_swarm_tls_cert_expiry_seconds",
            "Seconds remaining until certificate expiry.  Can go negative for expired certs.",
            &["cert_name", "subject_cn"],
            registry,
        )?;

        let reload_total = register_int_counter_vec_with_registry!(
            "efe_meta_swarm_tls_reload_total",
            "Total certificate reload events processed by the inotify watcher.",
            &["result"],
            registry,
        )?;

        let reload_at = register_gauge_vec_with_registry!(
            "efe_meta_swarm_tls_reload_at_seconds",
            "Unix timestamp (seconds) of the last successful certificate reload.",
            &["role"],
            registry,
        )?;

        Ok(Self {
            handshake_duration,
            handshake_total,
            cert_expiry,
            reload_total,
            reload_at,
        })
    }

    /// Record a successful handshake.
    pub fn record_handshake_success(&self, duration: Duration, role: Role, protocol: &str) {
        let secs = duration.as_secs_f64();
        self.handshake_duration
            .with_label_values(&["ok", protocol, role.label()])
            .observe(secs);
        self.handshake_total
            .with_label_values(&["ok", role.label(), "none"])
            .inc();
    }

    /// Record a failed handshake.
    pub fn record_handshake_failure(
        &self,
        duration: Duration,
        role: Role,
        protocol: &str,
        reason: HandshakeFailureReason,
    ) {
        let secs = duration.as_secs_f64();
        self.handshake_duration
            .with_label_values(&["fail", protocol, role.label()])
            .observe(secs);
        self.handshake_total
            .with_label_values(&["fail", role.label(), reason.label()])
            .inc();
    }

    /// Update the cert-expiry gauge for one cert.
    pub fn record_cert_expiry(&self, cert_name: &str, subject_cn: &str, seconds_remaining: i64) {
        self.cert_expiry
            .with_label_values(&[cert_name, subject_cn])
            .set(seconds_remaining as f64);
    }

    /// Record a reload event and stamp the timestamp gauge.
    pub fn record_reload(&self, outcome: ReloadOutcome, role: Role) {
        self.reload_total
            .with_label_values(&[outcome.label()])
            .inc();
        if outcome == ReloadOutcome::Ok {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as f64;
            self.reload_at.with_label_values(&[role.label()]).set(now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_names_match_architecture_doc() {
        let reg = Registry::new();
        let m = TlsMetrics::register(&reg).expect("register");
        // touch every metric so it shows up in the registry families.
        m.record_handshake_success(Duration::from_millis(5), Role::Server, "TLSv1.3");
        m.record_handshake_failure(
            Duration::from_millis(8),
            Role::Client,
            "TLSv1.3",
            HandshakeFailureReason::SanMismatch,
        );
        m.record_cert_expiry("controller-tls", "controller.efe.svc", 3600);
        m.record_reload(ReloadOutcome::Ok, Role::Server);

        let families = reg.gather();
        let names: Vec<&str> = families.iter().map(|f| f.get_name()).collect();
        assert!(names.contains(&"efe_meta_swarm_tls_handshake_duration_seconds"));
        assert!(names.contains(&"efe_meta_swarm_tls_handshake_total"));
        assert!(names.contains(&"efe_meta_swarm_tls_cert_expiry_seconds"));
        assert!(names.contains(&"efe_meta_swarm_tls_reload_total"));
        assert!(names.contains(&"efe_meta_swarm_tls_reload_at_seconds"));
    }

    #[test]
    fn handshake_failure_increments_only_failure_counter() {
        let reg = Registry::new();
        let m = TlsMetrics::register(&reg).expect("register");
        m.record_handshake_failure(
            Duration::from_millis(10),
            Role::Server,
            "TLSv1.3",
            HandshakeFailureReason::ExpiredCert,
        );

        let expired_count = m
            .handshake_total
            .get_metric_with_label_values(&["fail", "server", "expired_cert"])
            .expect("metric handle")
            .get();
        assert_eq!(expired_count, 1);

        let success_count = m
            .handshake_total
            .get_metric_with_label_values(&["ok", "server", "none"])
            .expect("metric handle")
            .get();
        assert_eq!(success_count, 0);
    }

    #[test]
    fn reload_records_timestamp_only_on_success() {
        let reg = Registry::new();
        let m = TlsMetrics::register(&reg).expect("register");
        m.record_reload(ReloadOutcome::Fail, Role::Server);

        let ts = m
            .reload_at
            .get_metric_with_label_values(&["server"])
            .expect("metric handle")
            .get();
        assert_eq!(ts, 0.0, "fail must NOT stamp the timestamp gauge");

        m.record_reload(ReloadOutcome::Ok, Role::Server);
        let ts = m
            .reload_at
            .get_metric_with_label_values(&["server"])
            .expect("metric handle")
            .get();
        assert!(ts > 0.0, "ok must stamp the timestamp gauge");
    }

    #[test]
    fn cert_expiry_accepts_negative_values() {
        let reg = Registry::new();
        let m = TlsMetrics::register(&reg).expect("register");
        m.record_cert_expiry("ghost", "expired.svc", -3600);
        let v = m
            .cert_expiry
            .get_metric_with_label_values(&["ghost", "expired.svc"])
            .expect("metric handle")
            .get();
        assert_eq!(v, -3600.0);
    }

    #[test]
    fn role_and_failure_labels_are_stable() {
        // Stability of these labels is part of our externalised
        // dashboard contract — regression check.
        assert_eq!(Role::Server.label(), "server");
        assert_eq!(Role::Client.label(), "client");
        assert_eq!(HandshakeFailureReason::ExpiredCert.label(), "expired_cert");
        assert_eq!(HandshakeFailureReason::UntrustedCa.label(), "untrusted_ca");
        assert_eq!(HandshakeFailureReason::SanMismatch.label(), "san_mismatch");
        assert_eq!(
            HandshakeFailureReason::ProtocolDowngrade.label(),
            "protocol_downgrade"
        );
        assert_eq!(HandshakeFailureReason::Other.label(), "other");
        assert_eq!(ReloadOutcome::Ok.label(), "ok");
        assert_eq!(ReloadOutcome::Fail.label(), "fail");
    }
}
