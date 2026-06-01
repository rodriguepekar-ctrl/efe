//! Sprint D18 — bench error type.
//!
//! Sprint **D29 (Φ-D)** — extended with `TonicError` (transport,
//! TLS, JWT, status codes) so the real gRPC client can surface
//! every failure mode without flattening to a String.
//!
//! Sprint **D31 (Φ-D)** — extended with `GpuShardOutOfRange` so
//! the `--gpu-shards` flag fails fast on N=0 or N>16.
//!
//! Sprint **D38 (Φ-F)** — extended with `KSwarmAboveMax` so the
//! `--k-swarm` flag refuses K>5 unless `--allow-k-above-max` is
//! explicitly set.

use thiserror::Error;

/// Top-level error for bench runs.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum BenchError {
    /// **T-D18-12 mitigation** — bench refuses to run against
    /// `prod` / `production` target.
    #[error("refused to run bench against prod target environment ({0:?})")]
    RefusedProdTarget(String),

    /// gRPC client error from a workload op.
    #[error("grpc client error: {0}")]
    GrpcClient(String),

    /// **Sprint D29 (Φ-D)** — typed Tonic-side failure (transport,
    /// TLS, status code, JWT). Distinct from `GrpcClient` (which
    /// stays for legacy stringly-typed errors) so post-D29 code
    /// can pattern-match on the failure mode.
    #[error("tonic error: {0}")]
    Tonic(#[from] TonicError),

    /// **Sprint D31 (Φ-D)** — `--gpu-shards N` outside the valid
    /// range `[1, 16]`. Surface 2 (config) so CI gates can
    /// distinguish a flag misuse from an SLO failure.
    #[error(
        "gpu_shards={got} out of range (expected 1..=16 ; cluster cap matches \
         largest deployed cluster of 4× H100 nodes × 4 GPUs)"
    )]
    GpuShardOutOfRange {
        /// The invalid value the operator passed.
        got: u32,
    },

    /// **Sprint D38 (Φ-F)** — `--k-swarm K > 5` without the
    /// `--allow-k-above-max` opt-in. The hard cap mirrors the
    /// server-side `Mode3Service` clamp (S-12) ; the bench REFUSES
    /// to silently let an operator request k=10 unless they
    /// explicitly acknowledge that the server will clamp.
    #[error(
        "k_swarm={got} exceeds K_MAX=5 ; pass --allow-k-above-max to \
         intentionally exercise the server-side clamp path"
    )]
    KSwarmAboveMax {
        /// The k value the operator passed.
        got: u32,
    },

    /// I/O error during dataset generation or report write.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialisation error on the report.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// Workload spec invalid (negative duration, zero RPS, etc.).
    #[error("invalid workload spec: {0}")]
    InvalidWorkload(String),

    /// Latency histogram overflow (request took > max bucket).
    #[error("latency histogram overflow: {0:?} > max bucket")]
    HistogramOverflow(std::time::Duration),

    /// Generic configuration error.
    #[error("configuration error: {0}")]
    Config(String),
}

impl BenchError {
    /// Convenience builder for `Config`.
    pub fn config(msg: impl Into<String>) -> Self {
        Self::Config(msg.into())
    }

    /// Stable label used by report metric series.
    #[must_use]
    pub fn metric_label(&self) -> &'static str {
        match self {
            Self::RefusedProdTarget(_) => "refused_prod",
            Self::GrpcClient(_) => "grpc_client",
            Self::Tonic(_) => "tonic",
            Self::GpuShardOutOfRange { .. } => "gpu_shard_out_of_range",
            Self::KSwarmAboveMax { .. } => "k_swarm_above_max",
            Self::Io(_) => "io",
            Self::Json(_) => "json",
            Self::InvalidWorkload(_) => "invalid_workload",
            Self::HistogramOverflow(_) => "histogram_overflow",
            Self::Config(_) => "config",
        }
    }
}

/// **Sprint D29 (Φ-D)** — typed Tonic-side failure. Wraps the
/// distinct failure modes of the real gRPC client so callers can
/// pattern-match on the variant (e.g. retry on `Transport`, abort
/// on `InvalidJwtToken`).
///
/// All `From` conversions deliberately stringify the source so the
/// crate compiles with `--no-default-features` (no tonic dep) ;
/// the `tonic`-feature build adds `From<tonic::transport::Error>`
/// + `From<tonic::Status>` so feature-on code can use `?` directly.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TonicError {
    /// Endpoint string malformed (e.g. missing scheme).
    #[error("invalid endpoint: {0}")]
    InvalidEndpoint(String),

    /// TLS handshake / certificate parse failure.
    #[error("tls error: {0}")]
    Tls(String),

    /// Could not connect to the endpoint.
    #[error("transport error: {0}")]
    Transport(String),

    /// JWT token failed to encode into a metadata header (e.g.
    /// non-ASCII characters).
    #[error("invalid jwt token: {0}")]
    InvalidJwtToken(String),

    /// gRPC status code returned by the server (`Unavailable`,
    /// `DeadlineExceeded`, `Unauthenticated`, …).
    #[error("grpc status: code={code} message={message}")]
    Status {
        /// Numeric `tonic::Code` value (0 = OK, 14 = Unavailable, …).
        code: u32,
        /// Human-readable reason.
        message: String,
    },

    /// **Sprint D38 (Φ-F)** — `Mode3ServiceClient` not built. This
    /// happens when the `tonic` feature is disabled but a Mode-3
    /// workload was requested. Surface 2 so CI distinguishes from
    /// an actual SLO failure.
    #[error(
        "mode3 stub not available ; rebuild efe-bench with `--features tonic` \
         and ensure the mode3.proto is compiled by build.rs"
    )]
    Mode3StubMissing,
}

#[cfg(feature = "tonic")]
impl From<tonic::transport::Error> for TonicError {
    fn from(e: tonic::transport::Error) -> Self {
        TonicError::Transport(e.to_string())
    }
}

#[cfg(feature = "tonic")]
impl From<tonic::Status> for TonicError {
    fn from(s: tonic::Status) -> Self {
        TonicError::Status {
            code: s.code() as u32,
            message: s.message().to_string(),
        }
    }
}

#[cfg(feature = "tonic")]
impl From<tonic::transport::Error> for BenchError {
    fn from(e: tonic::transport::Error) -> Self {
        BenchError::Tonic(TonicError::from(e))
    }
}

#[cfg(feature = "tonic")]
impl From<tonic::Status> for BenchError {
    fn from(s: tonic::Status) -> Self {
        BenchError::Tonic(TonicError::from(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refused_prod_target_renders_message() {
        let e = BenchError::RefusedProdTarget("prod".into());
        let s = e.to_string();
        assert!(s.contains("prod"));
    }

    #[test]
    fn metric_label_is_total() {
        for e in [
            BenchError::RefusedProdTarget("x".into()),
            BenchError::GrpcClient("x".into()),
            BenchError::Tonic(TonicError::InvalidEndpoint("x".into())),
            BenchError::GpuShardOutOfRange { got: 0 },
            BenchError::KSwarmAboveMax { got: 11 },
            BenchError::InvalidWorkload("x".into()),
            BenchError::HistogramOverflow(std::time::Duration::from_secs(1)),
            BenchError::Config("x".into()),
        ] {
            let _ = e.metric_label();
        }
    }

    #[test]
    fn config_helper_builds_variant() {
        let e = BenchError::config("bad");
        assert!(matches!(e, BenchError::Config(_)));
    }

    #[test]
    fn gpu_shard_out_of_range_renders_clear_message() {
        let e = BenchError::GpuShardOutOfRange { got: 17 };
        let s = e.to_string();
        assert!(s.contains("17"));
        assert!(s.contains("1..=16"));
    }

    #[test]
    fn k_swarm_above_max_renders_opt_in_hint() {
        let e = BenchError::KSwarmAboveMax { got: 10 };
        let s = e.to_string();
        assert!(s.contains("10"));
        assert!(s.contains("--allow-k-above-max"));
    }

    #[test]
    fn tonic_error_variants_render() {
        assert!(TonicError::InvalidEndpoint("scheme".into())
            .to_string()
            .contains("scheme"));
        assert!(TonicError::Tls("ca".into()).to_string().contains("tls"));
        assert!(TonicError::Transport("eof".into())
            .to_string()
            .contains("transport"));
        assert!(TonicError::InvalidJwtToken("non-ascii".into())
            .to_string()
            .contains("jwt"));
        assert!(TonicError::Status {
            code: 14,
            message: "unavailable".into()
        }
        .to_string()
        .contains("14"));
        assert!(TonicError::Mode3StubMissing
            .to_string()
            .contains("mode3"));
    }

    #[test]
    fn tonic_error_into_bench_error_via_from() {
        let t = TonicError::Mode3StubMissing;
        let b: BenchError = t.into();
        assert!(matches!(b, BenchError::Tonic(TonicError::Mode3StubMissing)));
    }
}
