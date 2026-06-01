//! `efe-telemetry` — observability stack for EFE (M11).
//!
//! Four pillars:
//! 1. **Structured JSON logs** via `tracing-subscriber`.
//! 2. **Tracing spans** (compatible with any OpenTelemetry exporter
//!    layered on `tracing-subscriber`; `tracing-opentelemetry` is not
//!    embedded in the MVP to keep the dependency footprint manageable).
//! 3. **Prometheus metrics** exposed on a dedicated HTTP sidecar.
//! 4. **Registry event sink** consuming M9 [`efe_core::RegistryEvent`]
//!    notifications and translating them to metrics + audit log entries.
//!
//! # Quick start
//!
//! ```no_run
//! # async fn example() -> anyhow::Result<()> {
//! use efe_telemetry::{init, TelemetryConfig};
//!
//! let handle = init(TelemetryConfig::default())?;
//! tracing::info!(msg = "efe-server starting", "bootstrap");
//! // ... run server ...
//! handle.shutdown().await;
//! # Ok(()) }
//! ```

#![warn(missing_docs)]
#![warn(clippy::all)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::must_use_candidate)]
#![cfg_attr(not(test), deny(clippy::unwrap_used))]

pub mod collector;
pub mod config;
pub mod events;
pub mod init;
pub mod logs;
pub mod metrics;
/// Sprint **D13** — single-call observability bootstrap.
pub mod observability_init_d13;
/// Sprint **D13** — OpenTelemetry SDK config + init scaffolding.
///
/// In v0.1.13 the init helper is a no-op when `enabled=false` and
/// returns `SdkNotImplemented` when `enabled=true`. The actual SDK
/// wire-up lands in D13-bis.
pub mod otel;
pub mod redaction;
pub mod shutdown;
/// Sprint **D13** — span name constants per architecture § 7.2.
pub mod spans;

pub use collector::{TracingTelemetryCollector, TracingTelemetryGuard};
pub use config::{LogFormat, TelemetryConfig};
pub use events::RegistrySink;
pub use init::{init, TelemetryHandle, TelemetryInitError};
pub use logs::audit::{AuditLogger, AuditSink, MemoryAuditSink};
pub use metrics::{
    instruments,
    recorder::record_request_completion,
    registry::global_registry,
    tower_layer::{map_grpc_status, MetricsLayer},
    MetricsServerHandle,
};
pub use observability_init_d13::{
    bootstrap_observability, BootstrapError, ObservabilityConfig,
    ObservabilityGuard,
};
pub use otel::{OtelConfig, OtelInitError};
pub use redaction::{mask_token, redact_prompt, truncate, REDACTED_MARKER};
