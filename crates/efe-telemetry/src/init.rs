//! Initialisation entry point — wires the configured sinks together
//! and returns a [`TelemetryHandle`] that must be held for the
//! lifetime of the process (dropping it triggers graceful flush).
//!
//! # Sprint D13-bis audit fix § A20 / A21 / A22 — operational wiring
//!
//! Pre-audit, the legacy `init_telemetry` call only registered the
//! D13 metric instruments and installed the JSON tracing-subscriber.
//! It NEVER built the OTel SDK pipeline (delivered by D13-bis).
//! Operators could set `EFE_OTEL_ENABLED=true` and not a single
//! span would be exported — the entire D13-bis surface was dead
//! code in production.
//!
//! Post-audit, this function :
//!
//!   1. Always calls `register_all_d13bis()` so the 5 OTel-self-obs
//!      Prometheus instruments appear in `/metrics` from the first
//!      scrape (zero-valued series in disabled clusters ; live
//!      counters in enabled ones).
//!   2. When `cfg.otel.enabled = true`, builds the real OTel pipeline
//!      via `crate::otel::init` and composes the
//!      `tracing-opentelemetry` layer into the subscriber stack.
//!      Failure modes :
//!        * If the OTel init returns Err and `cfg.fail_on_otel_init_error`
//!          is `true`, propagates the error → caller (typically
//!          `efe-server::main`) crashes the process.
//!        * If the OTel init returns Err and the flag is `false`,
//!          warns and continues without traces.
//!   3. Stores the `OtelGuard` inside `TelemetryHandle` so
//!      `TelemetryHandle::shutdown` can call `force_flush(timeout)`
//!      before the process exits.

use std::sync::Arc;

use thiserror::Error;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use crate::config::{LogFormat, TelemetryConfig};
use crate::logs::audit::{AuditLogger, AuditSink};
use crate::logs::JsonLayer;
use crate::metrics::{http_server::MetricsServerHandle, instruments};
use crate::otel::{OtelGuard, OtelInitError};

/// Fatal errors raised by [`init`].
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TelemetryInitError {
    /// `tracing-subscriber` rejected the env filter string.
    #[error("invalid log filter: {0}")]
    InvalidFilter(String),
    /// The audit log path could not be opened.
    #[error("audit log open: {0}")]
    AuditOpen(#[source] std::io::Error),
    /// The `/metrics` HTTP server could not bind.
    #[error("metrics server bind: {0}")]
    MetricsBind(#[source] std::io::Error),
    /// The global tracing subscriber was already set.
    #[error("global tracing subscriber already installed")]
    SubscriberAlreadySet,
    /// **Audit fix § A20** — the OTel SDK init returned an error and
    /// `cfg.fail_on_otel_init_error` was `true`. The caller crashes
    /// the process so misconfigured collectors surface immediately.
    #[error("OTel SDK init failed: {0}")]
    OtelInit(#[from] OtelInitError),
}

/// Handle to the running telemetry stack. Must be kept alive; dropping
/// it triggers a best-effort graceful shutdown (HTTP server stop, audit
/// flush).
pub struct TelemetryHandle {
    audit: Option<Arc<dyn AuditSink>>,
    metrics_server: Option<MetricsServerHandle>,
    /// Address queued for lazy binding by [`Self::ensure_metrics_bound`].
    pending_metrics_addr: Option<std::net::SocketAddr>,
    flush_timeout: std::time::Duration,
    /// **Audit fix § A20 (D13-bis operational wiring)** — RAII guard
    /// for the OTel TracerProvider. `Some(_)` iff
    /// `cfg.otel.enabled = true` AND the SDK init succeeded.
    /// `Self::shutdown()` calls `force_flush(flush_timeout)` on it
    /// before letting it drop.
    otel_guard: Option<OtelGuard>,
}

impl std::fmt::Debug for TelemetryHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelemetryHandle")
            .field("audit_attached", &self.audit.is_some())
            .field("metrics_server", &self.metrics_server.is_some())
            .field("flush_timeout", &self.flush_timeout)
            .field("otel_active", &self.otel_guard.is_some())
            .finish()
    }
}

impl TelemetryHandle {
    /// Access the audit sink (if configured). Useful for injecting it
    /// into an M9 [`crate::events::RegistrySink`].
    #[must_use]
    pub fn audit_sink(&self) -> Option<Arc<dyn AuditSink>> {
        self.audit.clone()
    }

    /// Address the Prometheus `/metrics` server is bound to (if any).
    #[must_use]
    pub fn metrics_local_addr(&self) -> Option<std::net::SocketAddr> {
        self.metrics_server
            .as_ref()
            .map(MetricsServerHandle::local_addr)
    }

    /// Graceful shutdown — stops the HTTP server, flushes audit log,
    /// and force-flushes the OTel TracerProvider. Idempotent.
    ///
    /// **Flush semantics** (audit finding M11-#2): we invoke
    /// `AuditSink::flush()` explicitly *before* dropping our local
    /// Arc clone, because other components (e.g., a `RegistrySink`
    /// task spawned by `main`) may still hold clones that keep
    /// `AuditLogger::Drop` from firing. Explicit flush guarantees
    /// in-flight bytes reach disk regardless of refcount.
    ///
    /// **Audit fix § A20 (D13-bis)** — when `otel.enabled = true`,
    /// `OtelGuard::force_flush(self.flush_timeout)` is called before
    /// the guard drops. Without this, the `BatchSpanProcessor`'s
    /// scheduled-delay window (1 s by default, architecture § 6.2)
    /// strands its in-flight batch on every SIGTERM. Pre-D13-bis
    /// observation : ~5 s × p99-rps spans lost per termination.
    pub async fn shutdown(mut self) {
        if let Some(h) = self.metrics_server.take() {
            let _ = tokio::time::timeout(self.flush_timeout, h.shutdown()).await;
        }
        if let Some(audit) = self.audit.as_ref() {
            audit.flush();
        }
        drop(self.audit.take());
        // **Audit fix § A20** — force-flush OTel pipeline.
        if let Some(guard) = self.otel_guard.take() {
            match guard.force_flush(self.flush_timeout).await {
                Ok(()) => {
                    tracing::debug!(
                        target: "efe.telemetry",
                        "OTel pipeline force_flush completed cleanly"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        target: "efe.telemetry",
                        error = %e,
                        timeout_ms = self.flush_timeout.as_millis() as u64,
                        "OTel pipeline force_flush returned error ; some spans may be lost"
                    );
                }
            }
            // Guard drops here, calling provider.shutdown() sync.
        }
    }

    /// Bind the Prometheus HTTP sidecar. No-op when:
    /// - no `metrics_addr` was configured; or
    /// - the server has already been bound.
    ///
    /// # Errors
    ///
    /// [`TelemetryInitError::MetricsBind`] if the socket cannot be bound.
    pub async fn ensure_metrics_bound(&mut self) -> Result<(), TelemetryInitError> {
        if self.metrics_server.is_some() {
            return Ok(());
        }
        let Some(addr) = self.pending_metrics_addr.take() else {
            return Ok(());
        };
        let h = crate::metrics::http_server::spawn_metrics_server(addr)
            .await
            .map_err(TelemetryInitError::MetricsBind)?;
        self.metrics_server = Some(h);
        Ok(())
    }

    /// **Audit fix § A20** — `true` iff the embedded OTel SDK was
    /// successfully initialised. Useful for `/health` endpoints
    /// that want to surface "traces flowing" alongside "metrics
    /// flowing" status.
    #[must_use]
    pub fn otel_active(&self) -> bool {
        self.otel_guard.is_some()
    }
}

/// Install the telemetry stack according to `cfg`. Returns an async
/// handle that binds the Prometheus HTTP endpoint on first `.await`.
///
/// This is a **synchronous** call: the global tracing subscriber is
/// installed immediately, but the `/metrics` server is only bound when
/// [`TelemetryHandle::ensure_metrics_bound`] is invoked (or implicitly
/// when the handle is used inside an async context).
///
/// Callers from `#[tokio::main]` typically run:
/// ```ignore
/// let mut handle = efe_telemetry::init(cfg)?;
/// handle.ensure_metrics_bound().await?;
/// ```
pub fn init(cfg: TelemetryConfig) -> Result<TelemetryHandle, TelemetryInitError> {
    // Always register metrics so `/metrics` has at least zeroed entries.
    instruments::register_all();

    // Sprint D13 audit M-1 fix : the 8 D13 instruments (catalog
    // RPC histograms, audit events counter, alert fires counter,
    // dashboard query histogram) MUST be registered at startup
    // so the family names appear from the very first scrape.
    // Without this call, the recording rules + alerts referencing
    // `efe_audit_events_total` etc. silently see "no data" until
    // the first emit-site fires.
    crate::metrics::register_all_d13();

    // **Audit fix § A21** — the 5 D13-bis OTel-self-obs instruments
    // (`efe_otel_export_total`, `efe_otel_export_duration_seconds`,
    // `efe_otel_batch_size`, `efe_otel_dropped_total`,
    // `efe_otel_attributes_filtered_total`) MUST be registered at
    // startup, regardless of whether OTel is enabled. In disabled
    // clusters, the dashboards render zero-valued series instead of
    // "no data" ; in enabled clusters, the instruments are live
    // counters from the first export.
    crate::metrics::register_all_d13bis();

    // --- Audit sink (optional) ---
    let audit: Option<Arc<dyn AuditSink>> = match &cfg.audit_log_path {
        Some(path) => Some(Arc::new(
            AuditLogger::open(path).map_err(TelemetryInitError::AuditOpen)?,
        )),
        None => None,
    };

    // **Audit fix § A20 (D13-bis)** — build the OTel SDK pipeline
    // BEFORE installing the subscriber so we can compose the
    // tracing-opentelemetry layer into the same `try_init` call.
    // Failure modes :
    //   * `Ok(None)` — OTel disabled, normal path.
    //   * `Ok(Some(guard))` — OTel enabled + initialised.
    //   * `Err(e)` and `fail_on_otel_init_error = true` — propagate.
    //   * `Err(e)` and `fail_on_otel_init_error = false` — warn +
    //     continue without traces (dev-cluster degraded mode).
    let otel_guard: Option<OtelGuard> = match crate::otel::init(&cfg.otel) {
        Ok(g) => g,
        Err(e) if cfg.fail_on_otel_init_error => {
            return Err(TelemetryInitError::OtelInit(e));
        }
        Err(e) => {
            // We can't `tracing::warn` yet — subscriber not installed.
            // Use eprintln to surface the issue to operators reading
            // pod logs at boot. The subsequent `tracing::info` call
            // (after subscriber install) re-states it.
            eprintln!(
                "[efe-telemetry] WARN OTel SDK init failed: {e} — \
                 continuing without traces (set EFE_OBSERVABILITY_FAIL_ON_OTEL_ERROR=true \
                 to crash on this)"
            );
            None
        }
    };

    // --- Global tracing subscriber (optional) ---
    if cfg.install_global_subscriber {
        install_subscriber(&cfg, otel_guard.as_ref())?;
    }

    if cfg.log_prompts {
        tracing::warn!(
            "EFE_LOG_PROMPTS is enabled — raw prompts will be written to logs verbatim. \
             This is a PII risk intended only for controlled debug sessions."
        );
    }
    if otel_guard.is_some() {
        tracing::info!(
            target: "efe.telemetry",
            endpoint = %cfg.otel.endpoint,
            sample_rate = cfg.otel.sample_rate,
            "OTel pipeline active — spans flow through tracing-opentelemetry layer"
        );
    }
    tracing::info!(
        service = cfg.service_name,
        environment = cfg.environment,
        metrics_addr = ?cfg.metrics_addr,
        audit_log = ?cfg.audit_log_path,
        otel_active = otel_guard.is_some(),
        "telemetry initialised (metrics HTTP server not bound yet)"
    );

    Ok(TelemetryHandle {
        audit,
        metrics_server: None,
        pending_metrics_addr: cfg.metrics_addr,
        flush_timeout: cfg.flush_timeout,
        otel_guard,
    })
}

/// **Audit fix § A20** — compose the tracing subscriber, optionally
/// adding the OTel layer when `guard` is `Some(_)`. The OTel layer
/// MUST be added at the same `try_init()` call as the JSON layer ;
/// the global subscriber can only be installed once and locks
/// after `try_init`.
///
/// The match arms duplicate the subscriber construction because
/// the OTel `OpenTelemetryLayer<S, _>` generic must unify with the
/// concrete `Layered<...>` chain at each `.with()` call site.
/// Pre-building the layer in a `let otel_layer = ...` binding
/// fails to compile (S can't be inferred outside a use-site).
fn install_subscriber(
    cfg: &TelemetryConfig,
    guard: Option<&OtelGuard>,
) -> Result<(), TelemetryInitError> {
    let env_filter = EnvFilter::try_new(&cfg.log_filter)
        .map_err(|e| TelemetryInitError::InvalidFilter(e.to_string()))?;
    let result = if let Some(g) = guard {
        // **OTel-active branch** — the layer is constructed fresh per
        // arm so the OTel layer's S unifies with each concrete
        // subscriber type.
        let tracer = g.tracer_for_layer();
        match cfg.log_format {
            LogFormat::Json => tracing_subscriber::registry()
                .with(env_filter)
                .with(JsonLayer::stdout(cfg.log_prompts))
                .with(tracing_opentelemetry::layer().with_tracer(tracer))
                .try_init(),
            LogFormat::Pretty => tracing_subscriber::registry()
                .with(env_filter)
                .with(tracing_subscriber::fmt::layer().pretty())
                .with(tracing_opentelemetry::layer().with_tracer(tracer))
                .try_init(),
            LogFormat::Compact => tracing_subscriber::registry()
                .with(env_filter)
                .with(tracing_subscriber::fmt::layer().compact())
                .with(tracing_opentelemetry::layer().with_tracer(tracer))
                .try_init(),
        }
    } else {
        // **OTel-inactive branch** — pre-D13-bis behaviour.
        match cfg.log_format {
            LogFormat::Json => tracing_subscriber::registry()
                .with(env_filter)
                .with(JsonLayer::stdout(cfg.log_prompts))
                .try_init(),
            LogFormat::Pretty => tracing_subscriber::registry()
                .with(env_filter)
                .with(tracing_subscriber::fmt::layer().pretty())
                .try_init(),
            LogFormat::Compact => tracing_subscriber::registry()
                .with(env_filter)
                .with(tracing_subscriber::fmt::layer().compact())
                .try_init(),
        }
    };
    result.map_err(|_| TelemetryInitError::SubscriberAlreadySet)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn silent_config_returns_handle_without_io() {
        let cfg = TelemetryConfig::silent();
        let handle = init(cfg).expect("init ok");
        assert!(handle.audit_sink().is_none());
        assert!(handle.metrics_local_addr().is_none());
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn init_with_audit_file_writes_and_shuts_down() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut cfg = TelemetryConfig::silent();
        cfg.audit_log_path = Some(tmp.path().to_path_buf());
        let handle = init(cfg).expect("init ok");
        assert!(handle.audit_sink().is_some());
        if let Some(sink) = handle.audit_sink() {
            sink.write_entry(serde_json::json!({ "event": "test" }));
        }
        handle.shutdown().await;
        let contents = std::fs::read_to_string(tmp.path()).unwrap();
        assert!(contents.contains("test"));
    }

    #[tokio::test]
    async fn init_with_metrics_endpoint_lazy_binds_port() {
        let mut cfg = TelemetryConfig::silent();
        cfg.metrics_addr = Some("127.0.0.1:0".parse().unwrap());
        let mut handle = init(cfg).expect("init ok");
        // Not yet bound — pending.
        assert!(handle.metrics_local_addr().is_none());
        handle.ensure_metrics_bound().await.unwrap();
        assert!(handle.metrics_local_addr().is_some());
        // Idempotent.
        handle.ensure_metrics_bound().await.unwrap();
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn init_with_invalid_filter_returns_error() {
        let mut cfg = TelemetryConfig::silent();
        cfg.install_global_subscriber = true;
        // `EnvFilter::try_new` rejects unescaped `[` or garbage syntax.
        cfg.log_filter = "[INVALID_FILTER_SYNTAX_!!@".into();
        let err = init(cfg).unwrap_err();
        assert!(matches!(err, TelemetryInitError::InvalidFilter(_)));
    }

    #[tokio::test]
    async fn init_with_metrics_addr_none_skips_binding() {
        let cfg = TelemetryConfig::silent();
        let mut handle = init(cfg).expect("init ok");
        handle.ensure_metrics_bound().await.unwrap();
        assert!(handle.metrics_local_addr().is_none());
        handle.shutdown().await;
    }

    // =================================================================
    // Audit fix § A20 / A21 — operational wiring
    // =================================================================

    #[tokio::test]
    async fn init_registers_d13bis_instruments_unconditionally() {
        // **Audit fix § A21** — even with OTel disabled, the 5
        // D13-bis instruments MUST appear in the global registry
        // so dashboards render zero-valued series.
        let cfg = TelemetryConfig::silent();
        let _handle = init(cfg).expect("init ok");
        // Touching one of the instruments should not panic and
        // should bump it (proving registration happened).
        let m = crate::metrics::d13bis_instruments::otel_attributes_filtered_total();
        let before = m.get();
        m.inc();
        let after = m.get();
        assert_eq!(after, before + 1);
        _handle.shutdown().await;
    }

    #[tokio::test]
    async fn init_returns_inactive_otel_when_disabled() {
        // **Audit fix § A20** — disabled OTel must produce a
        // handle with `otel_active = false`.
        let cfg = TelemetryConfig::silent();
        let handle = init(cfg).expect("init ok");
        assert!(!handle.otel_active());
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn init_propagates_otel_error_when_fail_mode_enabled() {
        // **Audit fix § A20** — `fail_on_otel_init_error = true`
        // turns a bad endpoint into a fatal init error.
        let mut cfg = TelemetryConfig::silent();
        cfg.otel = crate::otel::OtelConfig {
            enabled: true,
            endpoint: String::new(), // invalid → MissingEndpoint
            ..Default::default()
        };
        cfg.fail_on_otel_init_error = true;
        let err = init(cfg).unwrap_err();
        assert!(matches!(
            err,
            TelemetryInitError::OtelInit(crate::otel::OtelInitError::MissingEndpoint)
        ));
    }

    #[tokio::test]
    async fn init_degrades_gracefully_when_fail_mode_disabled() {
        // **Audit fix § A20** — `fail_on_otel_init_error = false`
        // (default) means a bad endpoint → warn + no guard, but
        // logs / metrics still flow.
        let mut cfg = TelemetryConfig::silent();
        cfg.otel = crate::otel::OtelConfig {
            enabled: true,
            endpoint: String::new(),
            ..Default::default()
        };
        cfg.fail_on_otel_init_error = false;
        let handle = init(cfg).expect("degraded init must succeed");
        assert!(!handle.otel_active(), "guard should be absent");
        handle.shutdown().await;
    }
}
