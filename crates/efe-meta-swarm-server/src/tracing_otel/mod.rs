//! PR #18 — OpenTelemetry distributed-tracing surface.
//!
//! Implements the design frozen in
//! [`docs/EFE_A1_PHASE2_PR18_ARCHITECTURE.md`][doc] :
//!
//!   * 8 EFE-specific span types covering the controller's Iterate
//!     call path, the participant's Step call path, manifest reload
//!     events, and graceful shutdown drain.
//!   * W3C Trace Context propagation across the controller ↔
//!     participant gRPC channel via a pair of `tonic::service::
//!     Interceptor` implementations.
//!   * OTLP/gRPC exporter to an operator-configurable collector
//!     (default `http://otel-collector.observability.svc:4317`).
//!   * Head-based parent-respecting sampling with an
//!     `always_on_error` overlay (AD8).
//!   * Resource attributes per OTel semantic-conventions v1.27.
//!
//! ## Module layout
//!
//!   * [`exporter`]    — OTLP exporter + batch processor wiring.
//!   * [`sampler`]     — `always_on_error` overlay on the rate sampler.
//!   * [`propagator`]  — W3C TraceContext extract / inject.
//!   * [`interceptor`] — tonic Server + Client interceptors.
//!   * [`spans`]       — span! macros + semconv attribute helpers.
//!   * [`resources`]   — `service.name` / `k8s.*` / `host.*` detection.
//!   * [`in_memory`]   — test-only `InMemoryExporter` (AD29).
//!   * [`metrics`]     — 3 exporter health metrics (Prometheus).
//!
//! ## Lifecycle
//!
//! ```text
//!     startup
//!        ├─ resources.detect() — build the Resource (service.name, k8s.*, ...)
//!        ├─ exporter.build_otlp() — open OTLP gRPC channel
//!        ├─ batch_processor — bounded queue + 5s flush
//!        ├─ sampler — parent_based(rate=ENV_TRACES_SAMPLER_ARG)
//!        │           + always_on_error overlay
//!        ├─ TracerProvider — registered globally + installed as the
//!        │                    tracing-opentelemetry bridge
//!        └─ return TracerGuard — RAII flushes on drop
//!
//!     per gRPC call
//!        ├─ server interceptor — extract `traceparent` ⇒ set parent
//!        ├─ span! macros fire — create / record / close spans
//!        ├─ client interceptor (outbound) — inject `traceparent`
//!        └─ on close — span sampled? batch processor enqueue ; otherwise drop
//!
//!     shutdown
//!        ├─ TracerGuard::drop — calls shutdown_tracer_provider()
//!        ├─ batch processor — flushes remaining spans (5s timeout)
//!        └─ exporter — closes gRPC channel
//! ```
//!
//! ## Backward compatibility (AD19 + AD23)
//!
//! When `OTEL_SDK_DISABLED=true` (the OTel-spec env var) or the chart
//! sets `tracing.otel.enabled=false`, [`init_otel`] returns early with
//! a no-op `TracerGuard` ; spans still emit via the existing
//! `tracing_subscriber::fmt` stdout formatter from
//! [`crate::observability::tracing`].
//!
//! [doc]: ../../../docs/EFE_A1_PHASE2_PR18_ARCHITECTURE.md

pub mod exporter;
pub mod in_memory;
pub mod interceptor;
pub mod metrics;
pub mod propagator;
pub mod recording_exporter;
pub mod resources;
pub mod sampler;
pub mod spans;

pub use exporter::{OtelInitError, OtlpExporterConfig};
pub use in_memory::{InMemoryExporter, InMemorySpan};
pub use interceptor::{ClientTraceInterceptor, ServerTraceInterceptor};
pub use metrics::{OtelExporterMetrics, OtelExporterReason};
pub use propagator::W3CTraceContextPropagator;
pub use resources::{detect_resource, Role};
pub use sampler::AlwaysOnErrorSampler;
pub use spans::{
    record_grpc_error, record_grpc_ok, span_attr, ATTR_FAILURE_REASON, ATTR_MANIFEST_VERSION,
    ATTR_OUTCOME, ATTR_PARTICIPANT_ID,
};

use std::time::Duration;

use opentelemetry::global;
use opentelemetry::propagation::TextMapCompositePropagator;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::{
    BatchConfigBuilder, BatchSpanProcessor, Sampler, TracerProvider as SdkTracerProvider,
};
use tracing::{info, warn};

use recording_exporter::MetricsRecordingExporter;

// ─────────────────────────────────────────────────────────────────────
// Public env-var names (OTel-spec compliant).
// ─────────────────────────────────────────────────────────────────────

/// OTel-spec : when `true`, the SDK initialises as a no-op.
pub const ENV_SDK_DISABLED: &str = "OTEL_SDK_DISABLED";

/// OTel-spec : OTLP collector endpoint URL.
pub const ENV_OTLP_ENDPOINT: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";

/// OTel-spec : transport protocol (`grpc` or `http/protobuf`).
pub const ENV_OTLP_PROTOCOL: &str = "OTEL_EXPORTER_OTLP_PROTOCOL";

/// OTel-spec : per-export timeout in milliseconds.
pub const ENV_OTLP_TIMEOUT_MS: &str = "OTEL_EXPORTER_OTLP_TIMEOUT";

/// OTel-spec : sampler name (`parentbased_traceidratio` etc.).
pub const ENV_SAMPLER: &str = "OTEL_TRACES_SAMPLER";

/// OTel-spec : sampler argument (the rate, e.g. `"0.01"`).
pub const ENV_SAMPLER_ARG: &str = "OTEL_TRACES_SAMPLER_ARG";

/// OTel-spec : service.name resource attribute.
pub const ENV_SERVICE_NAME: &str = "OTEL_SERVICE_NAME";

/// OTel-spec : `deployment.environment` resource attribute.
pub const ENV_DEPLOYMENT_ENVIRONMENT: &str = "OTEL_DEPLOYMENT_ENVIRONMENT";

/// OTel-spec : extra resource attributes (semicolon-sep `k=v;k=v`).
pub const ENV_RESOURCE_ATTRIBUTES: &str = "OTEL_RESOURCE_ATTRIBUTES";

// ─────────────────────────────────────────────────────────────────────
// TracerGuard — RAII handle returned by init_otel.
// ─────────────────────────────────────────────────────────────────────

/// RAII guard returned by [`init_otel`].  Drops the tracer provider on
/// shutdown, flushing any pending spans (5s timeout per AD9).
///
/// Hold this guard for the binary's lifetime ; dropping it before
/// `await_termination()` will lose in-flight spans.
pub struct TracerGuard {
    /// `Some` when the SDK was initialised with a real exporter ;
    /// `None` when `OTEL_SDK_DISABLED=true` (the no-op fallback).
    provider: Option<SdkTracerProvider>,
    /// Whether the SDK was actually enabled at boot.
    enabled: bool,
}

impl std::fmt::Debug for TracerGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TracerGuard")
            .field("enabled", &self.enabled)
            .finish_non_exhaustive()
    }
}

impl TracerGuard {
    /// Whether the OTel SDK is active.  `false` when
    /// `OTEL_SDK_DISABLED=true`.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

impl Drop for TracerGuard {
    fn drop(&mut self) {
        if let Some(provider) = self.provider.take() {
            // `shutdown()` flushes the BatchSpanProcessor (5s timeout per
            // AD9).  If the OTLP collector is unreachable, the spans are
            // dropped and an error is logged ; the drop completes
            // regardless so the binary can exit.
            if let Err(e) = provider.shutdown() {
                warn!(error = ?e, "OTel TracerProvider::shutdown returned an error");
            }
        }
        global::shutdown_tracer_provider();
    }
}

// ─────────────────────────────────────────────────────────────────────
// init_otel — public entry point called from bin/{controller,participant}.rs
// ─────────────────────────────────────────────────────────────────────

/// Initialise the OpenTelemetry SDK.
///
/// Reads configuration from OTel-spec env vars (§ 17.2 of the
/// architecture doc) :
///
///   * `OTEL_SDK_DISABLED=true` ⇒ no-op (returns a disabled guard).
///   * `OTEL_EXPORTER_OTLP_ENDPOINT` ⇒ defaults to
///     `http://otel-collector.observability.svc:4317`.
///   * `OTEL_TRACES_SAMPLER_ARG` ⇒ rate-based sampler ratio
///     (defaults to `1.0` for dev / unit-test reliability).
///   * `OTEL_SERVICE_NAME` ⇒ resource attribute ; falls back to
///     `"efe-meta-swarm-controller"` or `"efe-meta-swarm-participant"`
///     based on the `role` parameter (AD22).
///
/// Wires the resulting `TracerProvider` into the global OTel registry
/// and installs the W3C Trace Context propagator.  Returns a
/// [`TracerGuard`] that must be held for the binary's lifetime.
///
/// # Errors
///
/// Returns [`OtelInitError`] if :
///
///   * The endpoint URL fails to parse.
///   * The gRPC channel can't be established (network unreachable —
///     diagnosed at boot rather than per-call).
///   * The sampler argument is malformed (e.g. `"abc"` instead of a
///     ratio).
///
/// On any error, the caller should LOG the error and proceed without
/// the OTel SDK (the fallback is the existing
/// `tracing_subscriber::fmt` stdout formatter).  Spans created by
/// `tracing` macros still emit to stdout ; only the OTLP export is
/// missing.
pub fn init_otel(role: Role) -> Result<TracerGuard, OtelInitError> {
    init_otel_with_metrics(role, None)
}

/// PR #18 § 17.3 — flavour of [`init_otel`] that wires the 3 OTLP
/// exporter health Prometheus series (`exported_spans_total`,
/// `dropped_spans_total{reason}`, `export_duration_seconds`).
///
/// `metrics` should be `Some(registry.otel_exporter().clone())` when
/// called from the controller / participant binaries.  Pass `None` for
/// stand-alone callers that don't have a Prometheus registry handy
/// (the test suite, for example).
///
/// The metrics-recording layer is implemented via the
/// [`recording_exporter::MetricsRecordingExporter`] adapter wrapping
/// the OTLP `SpanExporter`.
///
/// This entry point also enforces the explicit BatchConfig knobs from
/// PR #18 § 17.4 / AC15 :
///
///   * `max_export_batch_size = 1024` (`DEFAULT_MAX_EXPORT_BATCH_SIZE`)
///   * `max_queue_size = 8192`        (`DEFAULT_MAX_QUEUE_SIZE`)
///   * `scheduled_delay = 5s`         (`DEFAULT_SCHEDULED_DELAY`)
///   * `max_export_timeout = 5s`      (`DEFAULT_MAX_EXPORT_TIMEOUT`)
///
/// These supersede the OTel-rs SDK defaults (batch 512, queue 2048,
/// delay 5s, timeout 30s) and are the values the architecture doc
/// commits to for AD9 + T62 budgeting.
pub fn init_otel_with_metrics(
    role: Role,
    metrics: Option<crate::tracing_otel::metrics::OtelExporterMetrics>,
) -> Result<TracerGuard, OtelInitError> {
    // ── 1. Honour OTEL_SDK_DISABLED ──
    if std::env::var(ENV_SDK_DISABLED).as_deref() == Ok("true") {
        info!(
            "OTel SDK disabled via {}=true ; spans emit to stdout only",
            ENV_SDK_DISABLED,
        );
        return Ok(TracerGuard {
            provider: None,
            enabled: false,
        });
    }

    // ── 2. Build the resource (service.name, k8s.*, host.*) ──
    let resource = detect_resource(role)?;

    // ── 3. Build the OTLP exporter ──
    let exporter_cfg = OtlpExporterConfig::from_env()?;
    let raw_exporter = exporter::build_otlp_exporter(&exporter_cfg)?;

    // ── 4. Build the sampler ──
    let inner_sampler = exporter::parse_sampler_from_env()?;
    let sampler = Sampler::ParentBased(Box::new(AlwaysOnErrorSampler::new(inner_sampler)));

    // ── 5. Build the explicit BatchConfig (AC15) ──
    let batch_config = BatchConfigBuilder::default()
        .with_max_export_batch_size(DEFAULT_MAX_EXPORT_BATCH_SIZE)
        .with_max_queue_size(DEFAULT_MAX_QUEUE_SIZE)
        .with_scheduled_delay(DEFAULT_SCHEDULED_DELAY)
        .with_max_export_timeout(DEFAULT_MAX_EXPORT_TIMEOUT)
        .build();

    // ── 6. Wrap the exporter with the metrics-recording adapter
    //       (AC20) when a metrics handle is provided.  Also enforce
    //       the per-span 200-attribute cardinality budget (AD12 /
    //       AC19 / T64) — supersedes the OTel-rs SDK default of 128. ──
    let attr_budget = u32::try_from(ATTRIBUTE_BUDGET_PER_SPAN).unwrap_or(200);
    let provider_builder = SdkTracerProvider::builder()
        .with_sampler(sampler)
        .with_resource(resource)
        .with_max_attributes_per_span(attr_budget);
    let provider = if let Some(m) = metrics {
        let wrapped = MetricsRecordingExporter::new(raw_exporter, m);
        let processor = BatchSpanProcessor::builder(wrapped, opentelemetry_sdk::runtime::Tokio)
            .with_batch_config(batch_config)
            .build();
        provider_builder.with_span_processor(processor).build()
    } else {
        let processor =
            BatchSpanProcessor::builder(raw_exporter, opentelemetry_sdk::runtime::Tokio)
                .with_batch_config(batch_config)
                .build();
        provider_builder.with_span_processor(processor).build()
    };

    // ── 7. Install globally ──
    global::set_tracer_provider(provider.clone());
    let composite = TextMapCompositePropagator::new(vec![Box::new(TraceContextPropagator::new())]);
    global::set_text_map_propagator(composite);

    info!(
        endpoint = %exporter_cfg.endpoint,
        service_name = %exporter_cfg.service_name,
        sampler_arg = exporter_cfg.sampler_arg,
        max_export_batch_size = DEFAULT_MAX_EXPORT_BATCH_SIZE,
        max_queue_size = DEFAULT_MAX_QUEUE_SIZE,
        scheduled_delay_ms = DEFAULT_SCHEDULED_DELAY.as_millis() as u64,
        max_export_timeout_ms = DEFAULT_MAX_EXPORT_TIMEOUT.as_millis() as u64,
        "OTel SDK initialised — spans will be exported to OTLP collector",
    );

    Ok(TracerGuard {
        provider: Some(provider),
        enabled: true,
    })
}

/// Best-effort variant of [`init_otel`] — logs errors and returns a
/// disabled guard rather than propagating the error.  Used in
/// `bin/{controller,participant}.rs` so a misconfigured OTel collector
/// doesn't crash the binary at boot.
#[must_use]
pub fn init_otel_or_disabled(role: Role) -> TracerGuard {
    match init_otel(role) {
        Ok(g) => g,
        Err(e) => {
            warn!(
                error = %e,
                "OTel init failed — falling back to stdout-only tracing",
            );
            TracerGuard {
                provider: None,
                enabled: false,
            }
        }
    }
}

/// Same as [`init_otel_or_disabled`] but wires the
/// [`OtelExporterMetrics`] so the BatchSpanProcessor's export results
/// surface as the 3 PR #18 Prometheus series on the registry passed in.
#[must_use]
pub fn init_otel_or_disabled_with_metrics(
    role: Role,
    metrics: crate::tracing_otel::metrics::OtelExporterMetrics,
) -> TracerGuard {
    match init_otel_with_metrics(role, Some(metrics)) {
        Ok(g) => g,
        Err(e) => {
            warn!(
                error = %e,
                "OTel init failed — falling back to stdout-only tracing",
            );
            TracerGuard {
                provider: None,
                enabled: false,
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
// Tests.
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Use a process-wide lock so env-var-touching tests across this
    /// module don't race.  Same pattern as `crate::config::ENV_LOCK`.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn tracer_guard_disabled_when_otel_sdk_disabled_env_set() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        std::env::set_var(ENV_SDK_DISABLED, "true");
        let guard = init_otel(Role::Controller).expect("init returns Ok with disabled");
        assert!(!guard.is_enabled());
        std::env::remove_var(ENV_SDK_DISABLED);
    }

    #[test]
    fn tracer_guard_disabled_when_init_or_disabled_called_with_bogus_endpoint() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // Setting the endpoint to garbage doesn't actually open a
        // connection (the gRPC channel is lazy) ; the URL parse step
        // accepts almost anything ; instead we simulate failure by
        // setting a clearly-invalid sampler arg that the parser rejects.
        std::env::remove_var(ENV_SDK_DISABLED);
        std::env::set_var(ENV_SAMPLER_ARG, "not-a-number");
        let guard = init_otel_or_disabled(Role::Controller);
        // The init failed ; the helper returns a disabled guard.
        assert!(!guard.is_enabled());
        std::env::remove_var(ENV_SAMPLER_ARG);
    }

    #[test]
    fn attribute_budget_constant_is_200() {
        // PR #18 AD12 / AC19 / T64 — committed budget per span.
        assert_eq!(ATTRIBUTE_BUDGET_PER_SPAN, 200);
    }

    #[test]
    fn batch_config_constants_match_architecture() {
        // PR #18 § 17.4 / AC15 — committed batch config knobs.
        assert_eq!(DEFAULT_MAX_EXPORT_BATCH_SIZE, 1024);
        assert_eq!(DEFAULT_MAX_QUEUE_SIZE, 8192);
        assert_eq!(DEFAULT_SCHEDULED_DELAY, Duration::from_secs(5));
        assert_eq!(DEFAULT_MAX_EXPORT_TIMEOUT, Duration::from_secs(5));
    }

    #[test]
    fn env_var_constants_match_otel_spec() {
        // Defence against typos : these are the canonical OTel-spec
        // env-var names.  Renaming them breaks every operator's
        // documentation + every OTel-aware tool.
        assert_eq!(ENV_SDK_DISABLED, "OTEL_SDK_DISABLED");
        assert_eq!(ENV_OTLP_ENDPOINT, "OTEL_EXPORTER_OTLP_ENDPOINT");
        assert_eq!(ENV_OTLP_PROTOCOL, "OTEL_EXPORTER_OTLP_PROTOCOL");
        assert_eq!(ENV_OTLP_TIMEOUT_MS, "OTEL_EXPORTER_OTLP_TIMEOUT");
        assert_eq!(ENV_SAMPLER, "OTEL_TRACES_SAMPLER");
        assert_eq!(ENV_SAMPLER_ARG, "OTEL_TRACES_SAMPLER_ARG");
        assert_eq!(ENV_SERVICE_NAME, "OTEL_SERVICE_NAME");
        assert_eq!(ENV_DEPLOYMENT_ENVIRONMENT, "OTEL_DEPLOYMENT_ENVIRONMENT");
        assert_eq!(ENV_RESOURCE_ATTRIBUTES, "OTEL_RESOURCE_ATTRIBUTES");
    }
}

/// Default OTLP collector endpoint (in-cluster K8s Service DNS).
pub const DEFAULT_OTLP_ENDPOINT: &str = "http://otel-collector.observability.svc:4317";

/// Default batch processor maximum export batch size (AD9).
pub const DEFAULT_MAX_EXPORT_BATCH_SIZE: usize = 1024;

/// Default batch processor scheduled-delay between flushes (AD9).
pub const DEFAULT_SCHEDULED_DELAY: Duration = Duration::from_secs(5);

/// Default batch processor max export timeout per attempt (AD9).
pub const DEFAULT_MAX_EXPORT_TIMEOUT: Duration = Duration::from_secs(5);

/// Default batch processor max queue size (AD9).
pub const DEFAULT_MAX_QUEUE_SIZE: usize = 8192;

/// Default sampler-arg rate (AD7) — 1.0 (100 %) for dev / unit-test
/// ergonomics.  The Helm chart overrides this to `0.01` for prod.
pub const DEFAULT_SAMPLER_ARG: f64 = 1.0;

/// Span attribute cardinality budget per span (AD12).
pub const ATTRIBUTE_BUDGET_PER_SPAN: usize = 200;
