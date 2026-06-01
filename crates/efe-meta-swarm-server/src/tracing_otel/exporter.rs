//! PR #18 § 17 — OTLP/gRPC exporter wiring.
//!
//! Builds the [`opentelemetry_otlp::SpanExporter`] backed by tonic's
//! HTTP/2 client (reusing our PR #6 tonic 0.12 stack — no second
//! gRPC implementation pulled into the binary).
//!
//! Configuration is OTel-spec compliant : every knob is settable
//! via the standard `OTEL_EXPORTER_OTLP_*` env vars (§ 17.2 of the
//! architecture doc).
//!
//! ## Failure modes
//!
//!   * **Endpoint malformed** ⇒ [`OtelInitError::InvalidEndpoint`].
//!     Caught at boot ; no per-span penalty.
//!   * **Network unreachable** at boot ⇒ the tonic channel is lazy ;
//!     the error surfaces on the first export attempt as a metric
//!     increment (`otel_export_errors_total`).
//!   * **Sampler arg malformed** ⇒ [`OtelInitError::InvalidSamplerArg`].

use std::time::Duration;

use opentelemetry_otlp::{SpanExporter, WithExportConfig};
use opentelemetry_sdk::trace::Sampler;

use super::{
    DEFAULT_OTLP_ENDPOINT, DEFAULT_SAMPLER_ARG, ENV_OTLP_ENDPOINT, ENV_OTLP_PROTOCOL,
    ENV_OTLP_TIMEOUT_MS, ENV_SAMPLER, ENV_SAMPLER_ARG, ENV_SERVICE_NAME,
};

// ─────────────────────────────────────────────────────────────────────
// OtelInitError — emitted when SDK initialisation fails.
// ─────────────────────────────────────────────────────────────────────

/// Errors returned by [`super::init_otel`].
#[derive(Debug, thiserror::Error)]
pub enum OtelInitError {
    /// `OTEL_EXPORTER_OTLP_ENDPOINT` failed to parse as a URI.
    #[error("invalid OTLP endpoint {endpoint:?} : {message}")]
    InvalidEndpoint { endpoint: String, message: String },
    /// `OTEL_EXPORTER_OTLP_PROTOCOL` value not in {`grpc`,
    /// `http/protobuf`}.
    #[error("unsupported OTLP protocol {got:?} (expected `grpc` or `http/protobuf`)")]
    UnsupportedProtocol { got: String },
    /// `OTEL_EXPORTER_OTLP_TIMEOUT` not parseable as milliseconds.
    #[error("invalid OTLP timeout {got:?} : {message}")]
    InvalidTimeout { got: String, message: String },
    /// `OTEL_TRACES_SAMPLER_ARG` not in `[0.0, 1.0]`.
    #[error("invalid sampler arg {got:?} : must be a float in [0.0, 1.0]")]
    InvalidSamplerArg { got: String },
    /// `OTEL_TRACES_SAMPLER` value not in the supported set.
    #[error(
        "unsupported sampler {got:?} (only `parentbased_traceidratio` and `always_on` supported)"
    )]
    UnsupportedSampler { got: String },
    /// `OTEL_RESOURCE_ATTRIBUTES` malformed.
    #[error("malformed OTEL_RESOURCE_ATTRIBUTES : {0}")]
    InvalidResourceAttributes(String),
    /// Underlying OTel-SDK error (rare ; typically a tonic channel
    /// construction failure).
    #[error("OTel SDK error : {0}")]
    Sdk(String),
}

// ─────────────────────────────────────────────────────────────────────
// OtlpExporterConfig — strongly-typed snapshot of the env-var config.
// ─────────────────────────────────────────────────────────────────────

/// Parsed OTLP exporter configuration ; built once at boot.
#[derive(Debug, Clone)]
pub struct OtlpExporterConfig {
    /// Collector endpoint URL.
    pub endpoint: String,
    /// `grpc` or `http/protobuf`.
    pub protocol: OtlpProtocol,
    /// Per-export timeout.
    pub timeout: Duration,
    /// Effective service name (after env override).
    pub service_name: String,
    /// Sampling ratio in `[0.0, 1.0]`.
    pub sampler_arg: f64,
}

/// OTLP transport protocol.  Only `Grpc` is fully wired in Phase 2 ;
/// `HttpProtobuf` is rejected at boot with a helpful error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtlpProtocol {
    /// gRPC over HTTP/2 (default, our tonic stack).
    Grpc,
    /// HTTP/1.1 with protobuf body.  Not implemented in Phase 2.
    HttpProtobuf,
}

impl OtlpExporterConfig {
    /// Build a config snapshot from the OTel-spec env vars.
    pub fn from_env() -> Result<Self, OtelInitError> {
        let endpoint =
            std::env::var(ENV_OTLP_ENDPOINT).unwrap_or_else(|_| DEFAULT_OTLP_ENDPOINT.to_string());

        // Validate that the endpoint at least parses as a URI ; tonic
        // will fail later otherwise.  We don't open a connection here.
        // `http` is re-exported through `hyper` which is a direct dep.
        let _: hyper::http::Uri = endpoint
            .parse()
            .map_err(
                |e: hyper::http::uri::InvalidUri| OtelInitError::InvalidEndpoint {
                    endpoint: endpoint.clone(),
                    message: e.to_string(),
                },
            )?;

        let protocol = match std::env::var(ENV_OTLP_PROTOCOL).as_deref() {
            Ok("grpc") | Err(_) => OtlpProtocol::Grpc,
            Ok("http/protobuf") => OtlpProtocol::HttpProtobuf,
            Ok(other) => {
                return Err(OtelInitError::UnsupportedProtocol {
                    got: other.to_string(),
                })
            }
        };

        let timeout_ms = match std::env::var(ENV_OTLP_TIMEOUT_MS) {
            Ok(s) => s
                .parse::<u64>()
                .map_err(|e| OtelInitError::InvalidTimeout {
                    got: s,
                    message: e.to_string(),
                })?,
            Err(_) => 10_000,
        };
        let timeout = Duration::from_millis(timeout_ms);

        let service_name =
            std::env::var(ENV_SERVICE_NAME).unwrap_or_else(|_| "efe-meta-swarm".to_string());

        let sampler_arg = parse_sampler_arg_from_env()?;

        Ok(Self {
            endpoint,
            protocol,
            timeout,
            service_name,
            sampler_arg,
        })
    }
}

/// Read `OTEL_TRACES_SAMPLER_ARG` from env and parse to `f64` in
/// `[0.0, 1.0]`.  Default [`DEFAULT_SAMPLER_ARG`] when unset.
pub fn parse_sampler_arg_from_env() -> Result<f64, OtelInitError> {
    match std::env::var(ENV_SAMPLER_ARG) {
        Err(_) => Ok(DEFAULT_SAMPLER_ARG),
        Ok(s) => {
            let v = s
                .parse::<f64>()
                .map_err(|_| OtelInitError::InvalidSamplerArg { got: s.clone() })?;
            if !v.is_finite() || v.is_nan() || !(0.0..=1.0).contains(&v) {
                return Err(OtelInitError::InvalidSamplerArg { got: s });
            }
            Ok(v)
        }
    }
}

/// Build the inner (non-error-overlay) sampler from env config.
///
/// Supported `OTEL_TRACES_SAMPLER` values :
///
///   * `parentbased_traceidratio` (default) — rate-based, parent-respecting.
///   * `always_on` — sample every trace.
///   * `always_off` — sample no trace (still goes through the
///      `always_on_error` overlay).
///
/// Anything else is a hard error (per AD6 ; we don't silently fall
/// back).
pub fn parse_sampler_from_env() -> Result<Sampler, OtelInitError> {
    let name =
        std::env::var(ENV_SAMPLER).unwrap_or_else(|_| "parentbased_traceidratio".to_string());
    let arg = parse_sampler_arg_from_env()?;
    match name.as_str() {
        "parentbased_traceidratio" => Ok(Sampler::TraceIdRatioBased(arg)),
        "always_on" => Ok(Sampler::AlwaysOn),
        "always_off" => Ok(Sampler::AlwaysOff),
        other => Err(OtelInitError::UnsupportedSampler {
            got: other.to_string(),
        }),
    }
}

// ─────────────────────────────────────────────────────────────────────
// build_otlp_exporter — the actual SpanExporter constructor.
// ─────────────────────────────────────────────────────────────────────

/// Build the OTLP/gRPC exporter from a validated config.
///
/// Returns an `opentelemetry_otlp::SpanExporter` ready to plug into
/// `BatchSpanProcessor::builder(exporter, runtime::Tokio)`.
pub fn build_otlp_exporter(cfg: &OtlpExporterConfig) -> Result<SpanExporter, OtelInitError> {
    match cfg.protocol {
        OtlpProtocol::Grpc => SpanExporter::builder()
            .with_tonic()
            .with_endpoint(&cfg.endpoint)
            .with_timeout(cfg.timeout)
            .build()
            .map_err(|e| OtelInitError::Sdk(e.to_string())),
        OtlpProtocol::HttpProtobuf => Err(OtelInitError::UnsupportedProtocol {
            got: "http/protobuf is Phase 3 ; use grpc in Phase 2".to_string(),
        }),
    }
}

// ─────────────────────────────────────────────────────────────────────
// Tests.
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn clear() {
        for v in [
            ENV_OTLP_ENDPOINT,
            ENV_OTLP_PROTOCOL,
            ENV_OTLP_TIMEOUT_MS,
            ENV_SAMPLER,
            ENV_SAMPLER_ARG,
            ENV_SERVICE_NAME,
        ] {
            std::env::remove_var(v);
        }
    }

    #[test]
    fn config_from_env_uses_defaults_when_unset() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear();
        let cfg = OtlpExporterConfig::from_env().expect("default ok");
        assert_eq!(cfg.endpoint, DEFAULT_OTLP_ENDPOINT);
        assert_eq!(cfg.protocol, OtlpProtocol::Grpc);
        assert_eq!(cfg.timeout, Duration::from_secs(10));
        assert_eq!(cfg.service_name, "efe-meta-swarm");
        assert!((cfg.sampler_arg - DEFAULT_SAMPLER_ARG).abs() < 1.0e-9);
    }

    #[test]
    fn config_honours_env_overrides() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear();
        std::env::set_var(ENV_OTLP_ENDPOINT, "http://tempo.observability.svc:4317");
        std::env::set_var(ENV_OTLP_TIMEOUT_MS, "5000");
        std::env::set_var(ENV_SERVICE_NAME, "my-service");
        std::env::set_var(ENV_SAMPLER_ARG, "0.1");
        let cfg = OtlpExporterConfig::from_env().expect("overrides ok");
        assert_eq!(cfg.endpoint, "http://tempo.observability.svc:4317");
        assert_eq!(cfg.timeout, Duration::from_millis(5000));
        assert_eq!(cfg.service_name, "my-service");
        assert!((cfg.sampler_arg - 0.1).abs() < 1.0e-9);
        clear();
    }

    #[test]
    fn rejects_invalid_endpoint() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear();
        std::env::set_var(ENV_OTLP_ENDPOINT, "::not a uri::");
        let err = OtlpExporterConfig::from_env().unwrap_err();
        assert!(matches!(err, OtelInitError::InvalidEndpoint { .. }));
        clear();
    }

    #[test]
    fn rejects_invalid_sampler_arg_out_of_range() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear();
        std::env::set_var(ENV_SAMPLER_ARG, "1.5");
        let err = OtlpExporterConfig::from_env().unwrap_err();
        assert!(matches!(err, OtelInitError::InvalidSamplerArg { .. }));
        clear();
    }

    #[test]
    fn rejects_invalid_sampler_arg_nan() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear();
        std::env::set_var(ENV_SAMPLER_ARG, "abc");
        let err = OtlpExporterConfig::from_env().unwrap_err();
        assert!(matches!(err, OtelInitError::InvalidSamplerArg { .. }));
        clear();
    }

    #[test]
    fn rejects_unsupported_protocol() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear();
        std::env::set_var(ENV_OTLP_PROTOCOL, "unsupported");
        let err = OtlpExporterConfig::from_env().unwrap_err();
        assert!(matches!(err, OtelInitError::UnsupportedProtocol { .. }));
        clear();
    }

    #[test]
    fn rejects_invalid_timeout() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear();
        std::env::set_var(ENV_OTLP_TIMEOUT_MS, "not-a-number");
        let err = OtlpExporterConfig::from_env().unwrap_err();
        assert!(matches!(err, OtelInitError::InvalidTimeout { .. }));
        clear();
    }

    #[test]
    fn parse_sampler_supports_always_on_off() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear();
        std::env::set_var(ENV_SAMPLER, "always_on");
        let s = parse_sampler_from_env().expect("ok");
        assert!(matches!(s, Sampler::AlwaysOn));
        std::env::set_var(ENV_SAMPLER, "always_off");
        let s = parse_sampler_from_env().expect("ok");
        assert!(matches!(s, Sampler::AlwaysOff));
        clear();
    }

    #[test]
    fn parse_sampler_rejects_unknown_name() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear();
        std::env::set_var(ENV_SAMPLER, "magic_8ball");
        let err = parse_sampler_from_env().unwrap_err();
        assert!(matches!(err, OtelInitError::UnsupportedSampler { .. }));
        clear();
    }
}
