//! Concrete [`efe_core::TelemetryCollector`] implementation that bridges
//! the controller's lifecycle events to:
//! - `tracing` spans/events (consumable by any subscriber installed at
//!   process startup — JSON logger, OTel bridge, etc.);
//! - Prometheus metrics via the instruments in [`crate::metrics`].

pub mod otel_collector;
pub mod otel_guard;

pub use otel_collector::TracingTelemetryCollector;
pub use otel_guard::TracingTelemetryGuard;
