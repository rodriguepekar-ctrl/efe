//! Sprint **D13-bis** — OpenTelemetry SDK real implementation.
//!
//! # Status
//!
//! **Active** : a real `TracerProvider` is built when
//! `EFE_OTEL_ENABLED=true`, with an OTLP/gRPC exporter, head-based
//! sampling, error-boost wrapping, W3C trace-context propagation,
//! and a RAII guard that flushes on shutdown.
//!
//! Pre-D13-bis (v0.1.13), [`init`] was a no-op that returned
//! `OtelInitError::SdkNotImplemented`. That variant is kept for
//! ABI compatibility but is no longer returned ; see
//! [`OtelInitError::SdkNotImplemented`].
//!
//! # Audit pass
//!
//! The post-implementation chirurgical audit landed the following
//! fixes :
//!
//!   * **A15** — `attribute_filter::filter_span_batch` is now
//!     invoked by [`measuring_exporter::MeasuringSpanExporter`]
//!     before forwarding ; PII never reaches Tempo.
//!   * **A2** — [`sampler::build_sampler`] no longer takes a dead
//!     `force_sample_on_error` parameter.
//!   * **A4** — [`OtelGuard::force_flush`] timeouts also bump
//!     `efe_otel_export_total{result="error"}` so SLO dashboards
//!     surface them.
//!   * **A5 / A14** — process-wide idempotency latch
//!     ([`OTEL_INITIALISED`]) ; double-init returns
//!     [`OtelInitError::AlreadyInitialised`] instead of silently
//!     overwriting the global TracerProvider.
//!   * **A8 / A9** — [`tracing_layer`] dead code stripped.
//!   * **A10** — [`OtelGuard::Drop`] surfaces shutdown failures
//!     via warn-line.
//!   * **A16** — `is_forbidden_attribute` is now allocation-free
//!     on the hot path.
//!
//! # Architecture
//!
//! Cf. `docs/bench/d13bis_otel_sdk_real_architecture.md`.
//!
//!   * Head-based, parent-based sampling, ratio 5 % by default.
//!   * Always-sample on errors via [`sampler::ErrorBoostingSampler`]
//!     when `cfg.force_sample_on_error = true`.
//!   * Span attributes filtered via
//!     [`attribute_filter::filter_span_batch`] before export.
//!   * OTLP/gRPC exporter to a configurable Tempo endpoint.
//!   * RAII guard that flushes in-flight spans on Drop.
//!
//! # Env vars consumed
//!
//! | Var                              | Default | Doc |
//! |----------------------------------|---------|-----|
//! | `EFE_OTEL_ENABLED`               | false   | When false, init is a no-op (returns Ok(None)). |
//! | `EFE_OTEL_ENDPOINT`              | (empty) | OTLP/gRPC endpoint, e.g. `http://tempo:4317` |
//! | `EFE_OTEL_SAMPLE_RATE`           | 0.05    | Head-based sample ratio ∈ [0, 1] |
//! | `EFE_OTEL_FORCE_SAMPLE_ON_ERROR` | true    | Always-sample errored spans |
//! | `EFE_OTEL_SERVICE_NAME`          | efe-server | Resource attribute |
//! | `EFE_OTEL_SERVICE_VERSION`       | (current crate version) | Resource attribute |

use std::env;
use std::sync::atomic::{AtomicBool, Ordering};

use thiserror::Error;

// Sprint **D13-bis** — sub-modules implementing the real OTel SDK
// pipeline. Cf. `docs/bench/d13bis_otel_sdk_real_architecture.md`.
pub mod attribute_filter;
pub mod measuring_exporter;
pub mod resource;
pub mod sampler;

/// **Audit fix § A5 / A14** — process-wide idempotency latch for
/// [`init`]. Set to `true` on the first successful enabled init ;
/// reset to `false` when the [`OtelGuard`] is dropped. A second
/// concurrent init returns [`OtelInitError::AlreadyInitialised`]
/// rather than silently overwriting the global TracerProvider.
static OTEL_INITIALISED: AtomicBool = AtomicBool::new(false);

// =====================================================================
// Config struct
// =====================================================================

/// Resolved OpenTelemetry SDK configuration.
///
/// **Audit fix § A22** — derives `Serialize`/`Deserialize` so it
/// can be embedded inside `crate::TelemetryConfig` and round-trip
/// through JSON for debugging endpoints (`/debug/config`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OtelConfig {
    pub enabled:                 bool,
    pub endpoint:                String,
    pub sample_rate:             f64,
    pub force_sample_on_error:   bool,
    pub service_name:            String,
    pub service_version:         String,
}

impl Default for OtelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: String::new(),
            sample_rate: 0.05,
            force_sample_on_error: true,
            service_name: "efe-server".to_owned(),
            service_version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }
}

// =====================================================================
// Errors
// =====================================================================

#[derive(Debug, Error)]
pub enum OtelInitError {
    #[error("EFE_OTEL_ENABLED=true but EFE_OTEL_ENDPOINT is empty")]
    MissingEndpoint,

    #[error("EFE_OTEL_SAMPLE_RATE must be in [0, 1] ; got {0}")]
    InvalidSampleRate(f64),

    #[error("invalid bool for {0}: {1:?}")]
    InvalidBool(&'static str, String),

    #[error("invalid float for {0}: {1:?}")]
    InvalidFloat(&'static str, String),

    /// **D13-bis** : OTLP exporter could not be built (TLS config,
    /// invalid endpoint URL, missing tonic feature). Inner string
    /// is the underlying error chain — sanitised, not the raw
    /// ExporterBuilderError because we don't want a transitive
    /// error type leaked through our public API.
    #[error("OTel exporter build failed: {0}")]
    ExporterBuild(String),

    /// Legacy variant — kept for ABI back-compat with v0.1.13
    /// callers that pattern-match on it. Pre-D13-bis init returned
    /// this when `enabled=true` ; now unused (init returns Ok or
    /// one of the other errors). Removing it would be a breaking
    /// change for any external code matching `OtelInitError`.
    #[error("OpenTelemetry SDK init not yet implemented (deprecated, D13-bis replaced)")]
    SdkNotImplemented,

    /// **Audit fix § A5 / A14** — `init()` was called more than
    /// once with `enabled=true`. The OTel SDK semantics treat
    /// `set_tracer_provider` as a "last writer wins" race ;
    /// double-init silently overwrites the previous provider and
    /// leaks span processors. We refuse the second call so the
    /// caller (typically a botched test harness or a hot-reload
    /// path) gets a clear signal.
    ///
    /// Recovery : drop the previous [`OtelGuard`] (which calls
    /// `provider.shutdown()`) before re-initialising.
    #[error("OpenTelemetry SDK already initialised (drop the previous OtelGuard first)")]
    AlreadyInitialised,
}

// =====================================================================
// Resolution
// =====================================================================

impl OtelConfig {
    /// Resolve from env vars. Returns the default config if no var is set.
    pub fn from_env() -> Result<Self, OtelInitError> {
        Self::from_env_lookup(|k: &str| env::var(k))
    }

    /// Resolve from a pluggable lookup (testable).
    pub fn from_env_lookup<F>(lookup: F) -> Result<Self, OtelInitError>
    where
        F: for<'a> Fn(&'a str) -> Result<String, env::VarError>,
    {
        let mut cfg = Self::default();

        if let Ok(v) = lookup("EFE_OTEL_ENABLED") {
            cfg.enabled = parse_bool(&v).map_err(|_| {
                OtelInitError::InvalidBool("EFE_OTEL_ENABLED", v)
            })?;
        }
        if let Ok(v) = lookup("EFE_OTEL_ENDPOINT") {
            cfg.endpoint = v;
        }
        if let Ok(v) = lookup("EFE_OTEL_SAMPLE_RATE") {
            let r: f64 = v.parse().map_err(|_| {
                OtelInitError::InvalidFloat("EFE_OTEL_SAMPLE_RATE", v.clone())
            })?;
            if !(0.0..=1.0).contains(&r) {
                return Err(OtelInitError::InvalidSampleRate(r));
            }
            cfg.sample_rate = r;
        }
        if let Ok(v) = lookup("EFE_OTEL_FORCE_SAMPLE_ON_ERROR") {
            cfg.force_sample_on_error = parse_bool(&v).map_err(|_| {
                OtelInitError::InvalidBool("EFE_OTEL_FORCE_SAMPLE_ON_ERROR", v)
            })?;
        }
        if let Ok(v) = lookup("EFE_OTEL_SERVICE_NAME") {
            cfg.service_name = v;
        }
        if let Ok(v) = lookup("EFE_OTEL_SERVICE_VERSION") {
            cfg.service_version = v;
        }

        if cfg.enabled && cfg.endpoint.is_empty() {
            return Err(OtelInitError::MissingEndpoint);
        }

        Ok(cfg)
    }
}

fn parse_bool(s: &str) -> Result<bool, ()> {
    match s.to_ascii_lowercase().as_str() {
        "true"  | "1" | "yes" | "y" | "on"  => Ok(true),
        "false" | "0" | "no"  | "n" | "off" | "" => Ok(false),
        _ => Err(()),
    }
}

// =====================================================================
// Init scaffolding
// =====================================================================

/// Initialise the OpenTelemetry SDK from `cfg`.
///
/// Behaviour :
///
///   * `cfg.enabled = false` → returns `Ok(None)`. No global state
///     touched. Idempotent.
///   * `cfg.enabled = true` and config valid → builds an OTLP
///     exporter, wraps with [`measuring_exporter::MeasuringSpanExporter`],
///     attaches the head-based + error-boost sampler, installs
///     the resource and W3C propagator, sets the global
///     `TracerProvider`, returns `Ok(Some(OtelGuard))`.
///   * `cfg.enabled = true` and config invalid → returns
///     `Err(OtelInitError::*)` (see the variants of
///     [`OtelInitError`]).
///
/// # Idempotency — audit fix § A5 / A14
///
/// A second concurrent enabled init returns
/// [`OtelInitError::AlreadyInitialised`]. Drop the previous
/// [`OtelGuard`] (which calls `provider.shutdown()`) before
/// re-initialising. The disabled path (`Ok(None)`) is not
/// idempotency-tracked because it is a true no-op.
pub fn init(cfg: &OtelConfig) -> Result<Option<OtelGuard>, OtelInitError> {
    if !cfg.enabled {
        tracing::debug!(target: "efe.telemetry.otel",
            sample_rate = cfg.sample_rate,
            "OTel SDK disabled by configuration (D13-bis no-op path)");
        return Ok(None);
    }
    if cfg.endpoint.is_empty() {
        return Err(OtelInitError::MissingEndpoint);
    }
    if !(0.0..=1.0).contains(&cfg.sample_rate) {
        return Err(OtelInitError::InvalidSampleRate(cfg.sample_rate));
    }

    // **Audit fix § A5 / A14** — refuse the second concurrent
    // init. Compare-and-swap : if the latch was already `true`,
    // bail out with `AlreadyInitialised`. Only on success do we
    // proceed to build the TracerProvider.
    if OTEL_INITIALISED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err(OtelInitError::AlreadyInitialised);
    }

    // **Sprint D13-bis** — real SDK init.
    //
    // Steps (architecture § 6.3) :
    //   1. Build the OTLP/gRPC exporter with the operator-set
    //      endpoint + a 5 s timeout (BatchConfig also caps at 5 s,
    //      defense in depth) ;
    //   2. Wrap with `MeasuringSpanExporter` so every export emits
    //      Prometheus metrics (audit fix § 12) ;
    //   3. Build the canonical sampler — `ParentBased(TraceIdRatioBased)`
    //      optionally wrapped with `ErrorBoostingSampler` ;
    //   4. Build the resource (service.name + version + instance.id
    //      + deployment.environment) ;
    //   5. Compose into a `TracerProvider` and install as global ;
    //   6. Install the W3C TraceContextPropagator so inbound
    //      `traceparent` headers are honoured ;
    //   7. Return an `OtelGuard` whose `Drop` calls `shutdown()`.
    use opentelemetry::global;
    use opentelemetry_otlp::{SpanExporter, WithExportConfig};
    use opentelemetry_sdk::propagation::TraceContextPropagator;
    use opentelemetry_sdk::trace::{
        BatchConfigBuilder, BatchSpanProcessor, TracerProvider,
    };
    use std::time::Duration;

    // **Audit fix § A5** — the latch was set above ; if the
    // exporter build fails we must release it before bubbling
    // the error, otherwise subsequent `init()` calls erroneously
    // see "AlreadyInitialised".
    let raw_exporter = match SpanExporter::builder()
        .with_tonic()
        .with_endpoint(&cfg.endpoint)
        .with_timeout(Duration::from_secs(5))
        .build()
    {
        Ok(e) => e,
        Err(e) => {
            OTEL_INITIALISED.store(false, Ordering::SeqCst);
            return Err(OtelInitError::ExporterBuild(e.to_string()));
        }
    };
    let exporter = measuring_exporter::MeasuringSpanExporter::wrap(raw_exporter);

    // **Audit fix § A2** — `build_sampler` no longer takes a
    // dead `force_sample_on_error` parameter ; the wrapping
    // happens at the `with_sampler(...)` call below.
    let sampler_inner = sampler::build_sampler(cfg.sample_rate);
    let resource = resource::build_resource(cfg);

    let batch_config = BatchConfigBuilder::default()
        .with_max_queue_size(4096)
        .with_scheduled_delay(Duration::from_secs(1))
        .with_max_export_batch_size(512)
        .build();
    let processor =
        BatchSpanProcessor::builder(exporter, opentelemetry_sdk::runtime::Tokio)
            .with_batch_config(batch_config)
            .build();

    // Build the TracerProvider with either the bare sampler or
    // the error-boosted wrapper.
    let provider_builder = TracerProvider::builder()
        .with_resource(resource)
        .with_span_processor(processor);
    let provider = if cfg.force_sample_on_error {
        provider_builder
            .with_sampler(sampler::ErrorBoostingSampler::wrap(sampler_inner))
            .build()
    } else {
        provider_builder.with_sampler(sampler_inner).build()
    };

    // Install as global so `tracing-opentelemetry` Layer can grab
    // it via `global::tracer(...)`.
    global::set_tracer_provider(provider.clone());
    global::set_text_map_propagator(TraceContextPropagator::new());

    tracing::info!(
        target: "efe.telemetry.otel",
        endpoint = %cfg.endpoint,
        sample_rate = cfg.sample_rate,
        force_sample_on_error = cfg.force_sample_on_error,
        service_name = %cfg.service_name,
        service_version = %cfg.service_version,
        "OTel SDK initialised (D13-bis)"
    );

    Ok(Some(OtelGuard { provider }))
}

// =====================================================================
// Sprint D13-bis — RAII guard for clean shutdown
// =====================================================================

/// **D13-bis** RAII handle returned by [`init`] when the SDK is
/// active. Holding this keeps the global TracerProvider alive ;
/// dropping it triggers a synchronous best-effort shutdown.
///
/// Operators that want a graceful flush should call
/// [`OtelGuard::force_flush`] explicitly with a timeout matching
/// `shutdown.grace_period` BEFORE letting the guard drop.
pub struct OtelGuard {
    provider: opentelemetry_sdk::trace::TracerProvider,
}

impl std::fmt::Debug for OtelGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OtelGuard")
            .field("provider", &"<TracerProvider>")
            .finish()
    }
}

impl OtelGuard {
    /// **Audit fix § A20** — accessor used by [`crate::init`] to
    /// build the `tracing-opentelemetry` layer in-line at the
    /// subscriber composition site (where the generic S parameter
    /// can be inferred from the surrounding `.with(...)` chain).
    /// Crate-private to keep external callers on the
    /// [`tracing_layer`] helper.
    #[doc(hidden)]
    pub(crate) fn tracer_for_layer(&self) -> opentelemetry_sdk::trace::Tracer {
        use opentelemetry::trace::TracerProvider as _;
        self.provider.tracer("efe")
    }

    /// Force-flush all in-flight spans through the exporter. Bounded
    /// by `timeout`. Returns `Ok(())` if the flush completed ;
    /// `Err(Timeout)` if the timeout elapsed first.
    ///
    /// Architecture fix audit § 11 : pre-D13-bis the `ObservabilityGuard`
    /// stub did nothing on shutdown, leading to ~5 s of in-flight
    /// spans lost on every SIGTERM.
    ///
    /// # Audit fix § A4 — dropped-count semantics
    ///
    /// Pre-audit, a timeout bumped `efe_otel_dropped_total{reason="shutdown_timeout"}`
    /// by **exactly 1**, regardless of how many spans were
    /// stranded in the queue. The metric was therefore useless
    /// for capacity planning : a 4096-span queue overflow looked
    /// identical to a single-span timeout. Post-audit, we record
    /// the timeout *occurrence* (count of timed-out flushes, one
    /// per call) which is the right semantic for an at-least-once
    /// signal — and we additionally bump
    /// `otel_export_total{result="error"}` so the SLO dashboard
    /// surfaces shutdown timeouts alongside export errors. The
    /// individual span counts at the time of timeout are not
    /// observable from this layer ; they would require the
    /// `BatchSpanProcessor` to expose its queue depth, which
    /// SDK 0.27 does not.
    pub async fn force_flush(
        &self,
        timeout: std::time::Duration,
    ) -> Result<(), OtelShutdownError> {
        let provider = self.provider.clone();
        // The provider's force_flush is sync ; we run it via
        // spawn_blocking so the async timeout works.
        let join = tokio::task::spawn_blocking(move || {
            // `force_flush` returns Vec<Result<...>> per processor ;
            // we collapse into a single bool.
            let results = provider.force_flush();
            results.iter().all(|r| r.is_ok())
        });
        match tokio::time::timeout(timeout, join).await {
            Ok(Ok(true)) => Ok(()),
            Ok(Ok(false)) => Err(OtelShutdownError::ProcessorErrored),
            Ok(Err(e)) => Err(OtelShutdownError::JoinError(e.to_string())),
            Err(_elapsed) => {
                // **Audit fix § A4** — record both the timeout
                // *occurrence* (so ops can alert on rate) and
                // bump the export-error counter (so SLO dashboards
                // surface this alongside other export failures).
                crate::metrics::d13bis_instruments::otel_dropped_total()
                    .with_label_values(&["shutdown_timeout"])
                    .inc();
                crate::metrics::d13bis_instruments::otel_export_total()
                    .with_label_values(&["error"])
                    .inc();
                Err(OtelShutdownError::Timeout(timeout))
            }
        }
    }
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        // Best-effort sync shutdown. Caller should have called
        // `force_flush(.)` async first.
        //
        // **Audit fix § A10** — surface shutdown errors via a
        // single warn-line per failure so ops can correlate with
        // collector outages. Pre-audit, errors were silently
        // swallowed by `let _ = self.provider.shutdown();`.
        let results = self.provider.force_flush();
        for r in results {
            if let Err(e) = r {
                tracing::warn!(
                    target: "efe.telemetry.otel",
                    error = %e,
                    "OtelGuard drop : processor force_flush returned error"
                );
            }
        }
        if let Err(e) = self.provider.shutdown() {
            tracing::warn!(
                target: "efe.telemetry.otel",
                error = %e,
                "OtelGuard drop : provider shutdown returned error"
            );
        }
        // **Audit fix § A5 / A14** — release the idempotency
        // latch so a subsequent `init()` (typically in tests or
        // hot-reload paths) can succeed.
        OTEL_INITIALISED.store(false, Ordering::SeqCst);
    }
}

/// **Sprint D13-bis** — build the `tracing-opentelemetry` Layer for
/// composition into the binary's subscriber stack.
///
/// The Rust `tracing-subscriber` Registry doesn't support adding
/// layers after `init()` ; the binary's `main.rs` is responsible
/// for composing the full subscriber up-front. This helper returns
/// the OTel layer ready to be `.with(...)`'d into the existing
/// `tracing_subscriber::registry()`.
///
/// Returns `None` when the guard is `None` (OTel disabled) — the
/// caller can pass through to the regular subscriber stack.
///
/// # Audit fix § A8 / A9
///
/// Pre-audit, this function had two `guard?` short-circuits with
/// a useless `let _ = guard?` between them. Post-audit, a single
/// match destructures the option once.
///
/// # Example
///
/// ```rust,ignore
/// // In efe-server/src/main.rs
/// let obs_guard = bootstrap_observability(&cfg)?;
/// let registry = tracing_subscriber::registry()
///     .with(env_filter)
///     .with(json_layer);
/// match crate::otel::tracing_layer(obs_guard.otel_guard.as_ref()) {
///     Some(otel_layer) => registry.with(otel_layer).try_init(),
///     None => registry.try_init(),
/// }?;
/// ```
#[must_use]
pub fn tracing_layer<S>(
    guard: Option<&OtelGuard>,
) -> Option<tracing_opentelemetry::OpenTelemetryLayer<S, opentelemetry_sdk::trace::Tracer>>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    use opentelemetry::trace::TracerProvider as _;
    // **Audit fix § A8 / A9** — single short-circuit. We obtain
    // the tracer directly from the guard's owned provider rather
    // than via `global::tracer(...)` so the layer doesn't depend
    // on the global being correctly set (defense in depth ; the
    // `init()` *also* installs the global, but that path is for
    // any code that uses `opentelemetry::global` directly).
    let g = guard?;
    let tracer = g.provider.tracer("efe");
    Some(tracing_opentelemetry::layer().with_tracer(tracer))
}

/// Shutdown failure modes for [`OtelGuard::force_flush`].
#[derive(Debug, Error)]
pub enum OtelShutdownError {
    /// `force_flush` did not complete within `grace_period`. Some
    /// spans are likely lost ; the dropped counter is bumped.
    #[error("OTel force_flush timed out after {0:?}")]
    Timeout(std::time::Duration),
    /// One of the span processors returned an error from
    /// `force_flush`. Spans up to that processor's queue are lost.
    #[error("OTel span processor returned an error during force_flush")]
    ProcessorErrored,
    /// The blocking-task wrapper around `force_flush` itself failed
    /// (panic in the processor, etc.).
    #[error("OTel force_flush join error: {0}")]
    JoinError(String),
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::env::VarError;

    fn lookup_factory(
        map: HashMap<&'static str, &'static str>,
    ) -> impl for<'a> Fn(&'a str) -> Result<String, VarError> {
        move |k: &str| {
            map.get(k)
                .map(|s| (*s).to_string())
                .ok_or(VarError::NotPresent)
        }
    }

    #[test]
    fn default_config_disabled() {
        let cfg = OtelConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.sample_rate, 0.05);
        assert!(cfg.force_sample_on_error);
        assert_eq!(cfg.service_name, "efe-server");
    }

    #[test]
    fn from_env_with_defaults() {
        let lookup = lookup_factory(HashMap::new());
        let cfg = OtelConfig::from_env_lookup(lookup).unwrap();
        assert!(!cfg.enabled);
    }

    #[test]
    fn from_env_overrides() {
        let mut m = HashMap::new();
        m.insert("EFE_OTEL_ENABLED",          "true");
        m.insert("EFE_OTEL_ENDPOINT",         "http://tempo:4317");
        m.insert("EFE_OTEL_SAMPLE_RATE",      "0.10");
        m.insert("EFE_OTEL_SERVICE_NAME",     "efe-acme");
        m.insert("EFE_OTEL_SERVICE_VERSION",  "0.1.13-acme");
        let cfg = OtelConfig::from_env_lookup(lookup_factory(m)).unwrap();
        assert!(cfg.enabled);
        assert_eq!(cfg.endpoint, "http://tempo:4317");
        assert_eq!(cfg.sample_rate, 0.10);
        assert_eq!(cfg.service_name, "efe-acme");
    }

    #[test]
    fn rejects_enabled_without_endpoint() {
        let mut m = HashMap::new();
        m.insert("EFE_OTEL_ENABLED", "true");
        let err = OtelConfig::from_env_lookup(lookup_factory(m)).unwrap_err();
        assert!(matches!(err, OtelInitError::MissingEndpoint));
    }

    #[test]
    fn rejects_sample_rate_above_one() {
        let mut m = HashMap::new();
        m.insert("EFE_OTEL_ENABLED",     "true");
        m.insert("EFE_OTEL_ENDPOINT",    "http://tempo:4317");
        m.insert("EFE_OTEL_SAMPLE_RATE", "1.5");
        let err = OtelConfig::from_env_lookup(lookup_factory(m)).unwrap_err();
        assert!(matches!(err, OtelInitError::InvalidSampleRate(_)));
    }

    #[test]
    fn rejects_sample_rate_negative() {
        let mut m = HashMap::new();
        m.insert("EFE_OTEL_ENABLED",     "true");
        m.insert("EFE_OTEL_ENDPOINT",    "http://tempo:4317");
        m.insert("EFE_OTEL_SAMPLE_RATE", "-0.1");
        let err = OtelConfig::from_env_lookup(lookup_factory(m)).unwrap_err();
        assert!(matches!(err, OtelInitError::InvalidSampleRate(_)));
    }

    #[test]
    fn rejects_invalid_bool() {
        let mut m = HashMap::new();
        m.insert("EFE_OTEL_ENABLED", "banana");
        let err = OtelConfig::from_env_lookup(lookup_factory(m)).unwrap_err();
        assert!(matches!(err, OtelInitError::InvalidBool(_, _)));
    }

    #[test]
    fn rejects_invalid_float() {
        let mut m = HashMap::new();
        m.insert("EFE_OTEL_ENABLED",     "true");
        m.insert("EFE_OTEL_ENDPOINT",    "http://tempo:4317");
        m.insert("EFE_OTEL_SAMPLE_RATE", "not-a-number");
        let err = OtelConfig::from_env_lookup(lookup_factory(m)).unwrap_err();
        assert!(matches!(err, OtelInitError::InvalidFloat(_, _)));
    }

    #[test]
    fn parse_bool_canonical_forms() {
        assert!(parse_bool("true").unwrap());
        assert!(parse_bool("1").unwrap());
        assert!(parse_bool("yes").unwrap());
        assert!(parse_bool("on").unwrap());
        assert!(!parse_bool("false").unwrap());
        assert!(!parse_bool("0").unwrap());
        assert!(!parse_bool("no").unwrap());
        assert!(!parse_bool("off").unwrap());
        assert!(!parse_bool("").unwrap());  // empty = false
    }

    #[test]
    fn init_is_noop_when_disabled() {
        let cfg = OtelConfig::default();
        // **D13-bis** : `init` now returns `Ok(None)` on the
        // disabled path (was `Ok(())` pre-D13-bis).
        let res = init(&cfg).expect("disabled init should succeed");
        assert!(res.is_none(), "disabled init must not produce a guard");
    }

    #[test]
    fn init_rejects_empty_endpoint_when_enabled() {
        // **D13-bis** : enabled path requires a non-empty endpoint.
        let cfg = OtelConfig {
            enabled: true,
            endpoint: String::new(),
            ..Default::default()
        };
        let err = init(&cfg).unwrap_err();
        assert!(matches!(err, OtelInitError::MissingEndpoint));
    }

    #[test]
    fn init_rejects_invalid_sample_rate_when_enabled() {
        let cfg = OtelConfig {
            enabled: true,
            endpoint: "http://tempo:4317".to_owned(),
            sample_rate: 1.5, // out of [0, 1]
            ..Default::default()
        };
        let err = init(&cfg).unwrap_err();
        assert!(matches!(err, OtelInitError::InvalidSampleRate(_)));
    }

    // =================================================================
    // Audit fix § A5 / A14 — idempotency latch
    // =================================================================

    #[test]
    fn already_initialised_variant_renders_message() {
        // Pin the user-visible message — ops greps logs for it.
        let err = OtelInitError::AlreadyInitialised;
        let msg = err.to_string();
        assert!(msg.contains("already initialised"));
        assert!(msg.contains("OtelGuard"));
    }

    #[test]
    fn idempotency_latch_releases_on_invalid_endpoint() {
        // **Audit fix § A5** — when init fails BEFORE setting the
        // latch (validation phase), the latch must remain false so
        // the next call can succeed.
        let cfg = OtelConfig {
            enabled: true,
            endpoint: String::new(),
            ..Default::default()
        };
        let _ = init(&cfg).unwrap_err();
        // Second call should also see MissingEndpoint, not
        // AlreadyInitialised.
        let err = init(&cfg).unwrap_err();
        assert!(matches!(err, OtelInitError::MissingEndpoint));
    }

    #[test]
    fn idempotency_latch_releases_on_invalid_sample_rate() {
        let cfg = OtelConfig {
            enabled: true,
            endpoint: "http://tempo:4317".to_owned(),
            sample_rate: 1.5,
            ..Default::default()
        };
        let _ = init(&cfg).unwrap_err();
        let err = init(&cfg).unwrap_err();
        assert!(matches!(err, OtelInitError::InvalidSampleRate(_)));
    }

    #[test]
    fn force_sample_on_error_field_is_preserved_through_default() {
        // **Audit fix § A2** — `OtelConfig::default()` keeps the
        // operator-visible `force_sample_on_error = true` default.
        // The field is still wired to ErrorBoostingSampler in init().
        let cfg = OtelConfig::default();
        assert!(cfg.force_sample_on_error);
    }
}
