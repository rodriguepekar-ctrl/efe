//! Sprint **D13** — single-call observability bootstrap.
//!
//! Usage in `efe-server::main` :
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use efe_telemetry::observability_init_d13::{
//!     bootstrap_observability, ObservabilityConfig,
//! };
//!
//! let cfg = ObservabilityConfig::from_env()?;
//! let _guard = bootstrap_observability(&cfg)?;
//! // ... server runs ...
//! // _guard drops on shutdown, flushing telemetry.
//! # Ok(()) }
//! ```
//!
//! What it does in v0.1.13 :
//!
//!   1. Forces registration of the 8 D13 metric instruments via
//!      [`crate::metrics::register_all_d13`] so the family names
//!      appear in `/metrics` from the first scrape.
//!   2. Resolves the OTel config and (no-op) initialises the SDK
//!      via [`crate::otel::init`].
//!   3. Returns a [`ObservabilityGuard`] that drops cleanly. In
//!      D13-bis this guard will own the OTel TracerProvider so
//!      shutdown can flush in-flight spans.
//!
//! What it does NOT do (yet) :
//!
//!   * Initialise tracing-opentelemetry layer (D13-bis ; needs deps).
//!   * Wire promtail / Loki client (out of scope ; that's the
//!     operator's K8s ConfigMap territory).
//!   * Configure log redaction patterns (already done by
//!     `redaction.rs`).

use thiserror::Error;

use crate::metrics::register_all_d13;
use crate::otel::{init as otel_init, OtelConfig, OtelInitError};

// =====================================================================
// Config bundle
// =====================================================================

/// Top-level observability config — convenience wrapper around the
/// individual subsystem configs.
#[derive(Debug, Clone)]
pub struct ObservabilityConfig {
    pub otel: OtelConfig,
    /// When `true`, return an error if the OTel SDK init fails. Set
    /// false for dev clusters where you want to degrade gracefully.
    pub fail_on_otel_init_error: bool,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            otel: OtelConfig::default(),
            fail_on_otel_init_error: false,
        }
    }
}

impl ObservabilityConfig {
    /// Resolve from env vars. The `EFE_OBSERVABILITY_FAIL_ON_OTEL_ERROR`
    /// var maps to `fail_on_otel_init_error` ; everything else is
    /// inherited from [`OtelConfig::from_env`].
    pub fn from_env() -> Result<Self, BootstrapError> {
        let otel = OtelConfig::from_env().map_err(BootstrapError::Otel)?;
        let fail_on_otel = std::env::var("EFE_OBSERVABILITY_FAIL_ON_OTEL_ERROR")
            .map(|v| matches!(v.to_ascii_lowercase().as_str(), "true" | "1"))
            .unwrap_or(false);
        Ok(Self {
            otel,
            fail_on_otel_init_error: fail_on_otel,
        })
    }
}

// =====================================================================
// Errors
// =====================================================================

#[derive(Debug, Error)]
pub enum BootstrapError {
    #[error("OTel config error: {0}")]
    Otel(#[from] OtelInitError),
}

// =====================================================================
// Guard
// =====================================================================

/// RAII guard returned by [`bootstrap_observability`]. Drop flushes
/// telemetry pipelines.
///
/// In v0.1.13 this is a stub — D13-bis adds the OTel TracerProvider
/// + Loki client handle here.
#[derive(Debug)]
pub struct ObservabilityGuard {
    /// True when the OTel SDK was actually initialized (post D13-bis).
    pub otel_active: bool,
    /// **Sprint D13-bis** — RAII handle for the OTel TracerProvider.
    /// `Some(_)` iff `otel_active = true`. Dropping it flushes
    /// in-flight spans (best-effort sync). Operators that want a
    /// graceful flush should call
    /// [`crate::otel::OtelGuard::force_flush`] explicitly BEFORE
    /// the guard drops.
    pub otel_guard: Option<crate::otel::OtelGuard>,
}

impl Drop for ObservabilityGuard {
    fn drop(&mut self) {
        if self.otel_active {
            tracing::debug!(target: "efe.telemetry.bootstrap",
                "ObservabilityGuard drop : flushing OTel TracerProvider");
            // The OtelGuard's own Drop runs after this, calling
            // provider.shutdown() — single source of truth.
        }
    }
}

// =====================================================================
// Bootstrap
// =====================================================================

/// Single-call bootstrap that wires every observability subsystem.
///
/// Order matters :
///   1. Register D13 metric descriptors *first* so the `/metrics`
///      endpoint exposes the family names from scrape #1.
///   2. Initialise the OTel SDK *second*, after metrics — that way
///      the OTel exporter's own metrics appear under the same
///      `/metrics` family as the rest of `efe_*`.
///   3. Return the guard.
pub fn bootstrap_observability(
    cfg: &ObservabilityConfig,
) -> Result<ObservabilityGuard, BootstrapError> {
    // 1. Metrics — always-on, idempotent.
    register_all_d13();
    // **Sprint D13-bis** — register the 4 OTel-self-obs instruments
    // unconditionally. Even when OTel is disabled, ops want zero-
    // valued series in the dashboard rather than "no data".
    crate::metrics::register_all_d13bis();

    // 2. OTel — opt-in, degrade-gracefully unless
    // `fail_on_otel_init_error`.
    let (otel_active, otel_guard) = match otel_init(&cfg.otel) {
        Ok(Some(guard)) => {
            // SDK init succeeded ; we hold the guard.
            (true, Some(guard))
        }
        Ok(None) => {
            // Disabled by config — pre-D13-bis behaviour.
            (false, None)
        }
        Err(OtelInitError::SdkNotImplemented) if !cfg.fail_on_otel_init_error => {
            tracing::warn!(target: "efe.telemetry.bootstrap",
                "OTel SDK reports SdkNotImplemented (legacy variant) ; \
                 continuing without traces");
            (false, None)
        }
        Err(e) if cfg.fail_on_otel_init_error => return Err(e.into()),
        Err(e) => {
            tracing::error!(target: "efe.telemetry.bootstrap",
                error = %e,
                "OTel SDK init failed ; continuing without traces");
            (false, None)
        }
    };

    Ok(ObservabilityGuard {
        otel_active,
        otel_guard,
    })
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_with_default_config_returns_inactive_guard() {
        let cfg = ObservabilityConfig::default();
        let guard = bootstrap_observability(&cfg).unwrap();
        assert!(!guard.otel_active);
        assert!(guard.otel_guard.is_none());
    }

    #[test]
    fn bootstrap_registers_d13_metric_families() {
        let cfg = ObservabilityConfig::default();
        let _guard = bootstrap_observability(&cfg).unwrap();
        // Sanity : descriptors are registered ; touching one
        // instrument should not error.
        let g = crate::metrics::d13_instruments::catalog_versions_per_name();
        g.with_label_values(&["t", "n"]).set(1);
    }

    /// **Sprint D13-bis** — the pre-audit version of these two
    /// tests asserted `OtelInitError::SdkNotImplemented` was
    /// returned. D13-bis replaces the no-op with a real SDK init,
    /// which requires a Tokio runtime + a reachable collector. We
    /// can't easily exercise the success path in a synchronous
    /// unit test ; we test the *error* paths (missing endpoint,
    /// invalid sample rate) which fail fast before any tokio /
    /// collector dependency.
    #[test]
    fn bootstrap_with_invalid_otel_config_propagates_when_fail_mode() {
        let cfg = ObservabilityConfig {
            otel: OtelConfig {
                enabled: true,
                endpoint: String::new(), // empty → MissingEndpoint
                ..Default::default()
            },
            fail_on_otel_init_error: true,
        };
        let err = bootstrap_observability(&cfg).unwrap_err();
        assert!(matches!(
            err,
            BootstrapError::Otel(OtelInitError::MissingEndpoint)
        ));
    }

    #[test]
    fn bootstrap_with_invalid_otel_config_degrades_gracefully() {
        let cfg = ObservabilityConfig {
            otel: OtelConfig {
                enabled: true,
                endpoint: String::new(), // empty → MissingEndpoint
                ..Default::default()
            },
            fail_on_otel_init_error: false,
        };
        let guard = bootstrap_observability(&cfg).unwrap();
        // SDK init failed but we got a guard with otel_active=false.
        assert!(!guard.otel_active);
        assert!(guard.otel_guard.is_none());
    }

    #[test]
    fn guard_drop_does_not_panic() {
        let cfg = ObservabilityConfig::default();
        let guard = bootstrap_observability(&cfg).unwrap();
        drop(guard);
    }

    #[test]
    fn bootstrap_idempotent() {
        // Calling bootstrap twice MUST NOT panic — the metric
        // registration uses OnceLock so re-calls are no-ops.
        let cfg = ObservabilityConfig::default();
        let _g1 = bootstrap_observability(&cfg).unwrap();
        let _g2 = bootstrap_observability(&cfg).unwrap();
    }
}
