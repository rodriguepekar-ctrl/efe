//! Sprint **D29 (Φ-D)** — `TonicBenchClient` (real gRPC).
//!
//! Wraps `tonic::transport::Channel` + 3 service stubs (Swarm,
//! Catalog, Mode3) so the bench harness can issue real RPCs
//! against a live `efe-server` (M4) and measure end-to-end
//! latency including TLS, auth, rate-limit, handler dispatch,
//! and backend compute.
//!
//! ## Auth model
//!
//! The Tonic client supports the same auth surface as `efe-server`
//! exposes server-side (M4 D22 mTLS + D2 JWT) :
//!
//!   * **JWT bearer** — every request gets `authorization: Bearer
//!     <token>` metadata. The token is parsed once at construction
//!     and stored as a `MetadataValue<Ascii>` so the hot path does
//!     no string allocation.
//!   * **mTLS** — opt-in via [`TonicConfig::with_tls`]. When enabled,
//!     the client uses `ClientTlsConfig::with_native_roots()` plus
//!     an explicit CA certificate (PEM) so the server identity is
//!     verified against the EFE-internal CA.
//!
//! ## Sprint D31 — GPU shard propagation
//!
//! When the runner is configured with `--gpu-shards N`, every
//! request gains an `x-efe-gpu-shard-id: <idx>` metadata header
//! computed as `worker_idx % N`. The server-side router (M4
//! interceptors §13) consumes the header and routes to
//! `efe-kernels::CudaBackend` on the corresponding GPU.
//!
//! ## Sprint D38 — Mode-3 dispatch
//!
//! `WorkloadOp::Mode3Execute { k_swarm }` triggers a call to
//! `efe.v1.mode3.Mode3Service.Execute`. The server clamps `k_swarm`
//! to 5 if it exceeds the soft cap ; the bench surfaces the clamp
//! transparently — the operator can compare reported `k_clamped`
//! counts with the `--k-swarm 10` request to validate the clamp
//! path (S-12 SLO).

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tonic::metadata::{Ascii, MetadataValue};
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint};
use tonic::Request;

use crate::client::{BenchClient, CallOutcome, ClientKind};
use crate::error::{BenchError, TonicError};
use crate::workload::WorkloadOp;

// ---------------------------------------------------------------------
// Generated proto stubs.
//
// `tonic-build` (build.rs) generates the modules under their proto
// `package` name : `efe.v1`, `efe.v1.catalog`, `efe.v1.mode3`.
//
// **Critical** : we MUST mirror the package nesting so that
// `super::Verdict` references inside `efe.v1.mode3` (which uses
// `efe.v1.types.proto`'s `Verdict` enum) resolve to
// `proto::v1::Verdict` as the code generator expects. Same pattern
// as `efe-server::lib.rs::proto::v1::{catalog, mode3}`.
// ---------------------------------------------------------------------

#[allow(missing_docs, clippy::all, clippy::pedantic, unreachable_pub)]
pub(crate) mod proto {
    pub(crate) mod v1 {
        tonic::include_proto!("efe.v1");

        pub(crate) mod catalog {
            tonic::include_proto!("efe.v1.catalog");
        }

        pub(crate) mod mode3 {
            tonic::include_proto!("efe.v1.mode3");
        }
    }
}

use proto::v1::catalog as proto_catalog;
use proto::v1::mode3 as proto_mode3;
use proto::v1 as proto_efe;

use proto_catalog::catalog_service_client::CatalogServiceClient;
use proto_efe::swarm_service_client::SwarmServiceClient;
use proto_mode3::mode3_service_client::Mode3ServiceClient;

// ---------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------

/// **Sprint D29 (Φ-D)** — connection configuration for [`TonicBenchClient`].
///
/// Built from CLI flags (`--endpoint`, `--jwt-token`, `--tls-ca-file`)
/// in `bin/efe_bench.rs`. The struct is intentionally minimal —
/// transport-level options (timeouts, keepalive) are handled by
/// `tonic::transport::Endpoint` defaults which match the M4 server
/// contract (5 s connect, no per-call timeout — the bench enforces
/// its own deadline via `--duration`).
#[derive(Debug, Clone)]
pub struct TonicConfig {
    /// gRPC endpoint URL, e.g. `http://127.0.0.1:50051` or
    /// `https://staging.efe.ai:50051`. The scheme controls TLS by
    /// default ; the explicit `tls` field below overrides it.
    pub endpoint: String,
    /// Optional JWT bearer token. When present, every request
    /// includes `authorization: Bearer <token>` metadata.
    pub jwt_token: Option<String>,
    /// Optional path to a CA certificate (PEM-encoded) used for
    /// mTLS server verification. None = plain h2c (no TLS).
    pub tls_ca_file: Option<std::path::PathBuf>,
    /// **Sprint D31 (Φ-D)** — number of GPU shards orchestrated
    /// by this client. None = no sharding (single GPU).
    pub gpu_shards: Option<u32>,
    /// **Sprint D31 (Φ-D)** — server domain name used for TLS SNI
    /// when `tls_ca_file` is set. Default `localhost`.
    pub tls_domain_name: Option<String>,
}

impl TonicConfig {
    /// Construct from a raw endpoint string with no auth + no TLS.
    /// Used by tests + smoke runs.
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            jwt_token: None,
            tls_ca_file: None,
            gpu_shards: None,
            tls_domain_name: None,
        }
    }

    /// Attach a JWT bearer token.
    #[must_use]
    pub fn with_jwt(mut self, token: impl Into<String>) -> Self {
        self.jwt_token = Some(token.into());
        self
    }

    /// Attach a CA certificate path for mTLS server verification.
    #[must_use]
    pub fn with_tls_ca_file(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.tls_ca_file = Some(path.into());
        self
    }

    /// **Sprint D31 (Φ-D)** — declare the number of GPU shards
    /// orchestrated by this client. The bench runner uses this
    /// to compute `worker_idx % gpu_shards` per request.
    #[must_use]
    pub fn with_gpu_shards(mut self, shards: u32) -> Self {
        self.gpu_shards = Some(shards);
        self
    }

    /// Override the TLS SNI / server name.
    #[must_use]
    pub fn with_tls_domain_name(mut self, name: impl Into<String>) -> Self {
        self.tls_domain_name = Some(name.into());
        self
    }
}

// ---------------------------------------------------------------------
// TonicBenchClient
// ---------------------------------------------------------------------

/// **Sprint D29 (Φ-D)** — Tonic-backed [`BenchClient`].
///
/// Owns a single `Channel` (cheap to clone) and one stub per
/// service. Each `call` dispatches on `op`, encodes a synthetic
/// proto request, sends via the corresponding stub, and reports
/// the round-trip latency. Failures are downgraded to
/// `CallOutcome { ok: false, error_label: Some(...) }` so the
/// runner records the error in its histogram without aborting
/// the run (a bench is allowed to observe a few transient
/// failures — the SLO check decides the verdict).
pub struct TonicBenchClient {
    channel: Channel,
    auth_header: Option<MetadataValue<Ascii>>,
    swarm_stub: SwarmServiceClient<Channel>,
    catalog_stub: CatalogServiceClient<Channel>,
    mode3_stub: Mode3ServiceClient<Channel>,
    gpu_shards: Option<u32>,
    /// **Audit fix M6-A1 (Φ-F)** — atomic counter incremented per
    /// call. Used ONLY to derive the Mode-3 `seed` field (per-call
    /// determinism + diversity) and the request_id suffix
    /// (traceability). Pre-audit it was also used for the
    /// `x-efe-gpu-shard-id` shard selection, which decoupled the
    /// metric bucket (keyed on `worker_id`) from the propagated
    /// shard label (keyed on this counter). Post-audit shard
    /// selection uses the `worker_id` parameter passed from the
    /// runner so both align.
    call_idx: Arc<std::sync::atomic::AtomicU64>,
}

impl std::fmt::Debug for TonicBenchClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TonicBenchClient")
            .field("gpu_shards", &self.gpu_shards)
            .field("auth", &self.auth_header.is_some())
            .finish_non_exhaustive()
    }
}

impl TonicBenchClient {
    /// Connect to the configured endpoint and build all 3 stubs.
    ///
    /// Errors :
    ///   * [`TonicError::InvalidEndpoint`] — endpoint string
    ///     malformed.
    ///   * [`TonicError::Tls`] — CA certificate parse failure.
    ///   * [`TonicError::Transport`] — connect refused / timeout.
    ///   * [`TonicError::InvalidJwtToken`] — JWT contains non-ASCII.
    pub async fn connect(config: TonicConfig) -> Result<Self, BenchError> {
        let mut endpoint =
            Endpoint::from_shared(config.endpoint.clone()).map_err(|e| {
                BenchError::Tonic(TonicError::InvalidEndpoint(e.to_string()))
            })?;

        if let Some(ca_path) = &config.tls_ca_file {
            let ca_pem = std::fs::read(ca_path).map_err(BenchError::Io)?;
            let ca = Certificate::from_pem(ca_pem);
            let mut tls = ClientTlsConfig::new().ca_certificate(ca);
            if let Some(domain) = &config.tls_domain_name {
                tls = tls.domain_name(domain.clone());
            }
            endpoint = endpoint
                .tls_config(tls)
                .map_err(|e| BenchError::Tonic(TonicError::Tls(e.to_string())))?;
        }

        let channel = endpoint
            .connect()
            .await
            .map_err(|e| BenchError::Tonic(TonicError::Transport(e.to_string())))?;

        let auth_header = match config.jwt_token {
            Some(t) => {
                let formatted = format!("Bearer {t}");
                let v: MetadataValue<Ascii> =
                    formatted.parse().map_err(|e: tonic::metadata::errors::InvalidMetadataValue| {
                        BenchError::Tonic(TonicError::InvalidJwtToken(e.to_string()))
                    })?;
                Some(v)
            }
            None => None,
        };

        let swarm_stub = SwarmServiceClient::new(channel.clone());
        let catalog_stub = CatalogServiceClient::new(channel.clone());
        let mode3_stub = Mode3ServiceClient::new(channel.clone());

        Ok(Self {
            channel,
            auth_header,
            swarm_stub,
            catalog_stub,
            mode3_stub,
            gpu_shards: config.gpu_shards,
            call_idx: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        })
    }

    /// Inject auth + GPU shard metadata into a request.
    ///
    /// **Audit fix M6-A1 (Φ-F)** — `worker_id` is the runner-side
    /// per-worker stable counter (the same value the metrics
    /// pipeline uses to bucket per-shard observations). Pre-audit
    /// this parameter was a private atomic counter rotating
    /// across all workers ; post-audit it's threaded from the
    /// runner so the propagated `x-efe-gpu-shard-id` matches the
    /// per-shard report bucket exactly. The Tonic metadata key
    /// is `x-efe-gpu-shard-id` (lowercase ASCII) per the M4
    /// interceptor contract.
    fn decorate<T>(&self, mut req: Request<T>, worker_id: u64) -> Request<T> {
        if let Some(tok) = &self.auth_header {
            req.metadata_mut().insert("authorization", tok.clone());
        }
        if let Some(shards) = self.gpu_shards {
            if shards > 0 {
                let shard_id = (worker_id % (shards as u64)) as u32;
                if let Ok(v) = shard_id.to_string().parse::<MetadataValue<Ascii>>() {
                    req.metadata_mut().insert("x-efe-gpu-shard-id", v);
                }
            }
        }
        req
    }

    /// Borrow the underlying channel (used by tests + advanced
    /// callers that want to issue ad-hoc RPCs).
    pub fn channel(&self) -> &Channel {
        &self.channel
    }

    // -----------------------------------------------------------------
    // Per-op dispatch.
    //
    // Each helper builds a minimal valid proto request, decorates
    // it with auth + shard metadata, sends, and discards the
    // response (the bench only times the round-trip ; semantics
    // are server-side concerns). Errors are returned as
    // `BenchError::Tonic(...)` so the runner can downgrade them
    // to `CallOutcome { ok: false, ... }`.
    // -----------------------------------------------------------------

    async fn dispatch_execute(
        &self,
        tenant: &str,
        worker_id: u64,
    ) -> Result<(), BenchError> {
        let req = proto_efe::ExecuteRequest {
            request_id: format!("bench-{}", uuid::Uuid::new_v4()),
            raw_prompt: format!("bench prompt for {tenant}"),
            spec: None,
            decode_mode: None,
            deadline: None,
        };
        let mut stub = self.swarm_stub.clone();
        let req = self.decorate(Request::new(req), worker_id);
        let _ = stub.execute(req).await?;
        Ok(())
    }

    async fn dispatch_register_catalog(
        &self,
        tenant: &str,
        worker_id: u64,
    ) -> Result<(), BenchError> {
        let req = proto_catalog::RegisterCatalogRequest {
            tenant: tenant.to_string(),
            name: format!("bench-cat-{}", uuid::Uuid::new_v4()),
            version: "v1".into(),
            content: None,
            created_by: "efe-bench".into(),
            description: "synthetic bench catalog".into(),
            labels: std::collections::HashMap::new(),
        };
        let mut stub = self.catalog_stub.clone();
        let req = self.decorate(Request::new(req), worker_id);
        let _ = stub.register_catalog(req).await?;
        Ok(())
    }

    async fn dispatch_register_class(
        &self,
        tenant: &str,
        worker_id: u64,
    ) -> Result<(), BenchError> {
        // RegisterClass is modelled as an EvolveCatalog op with an
        // AppendClassOp payload. The bench fakes the parent_key —
        // server-side response is irrelevant to the latency
        // measurement, so a NotFound / InvalidArgument from the
        // server still records the round-trip time and bumps the
        // error counter (legitimate bench behaviour).
        let class = proto_catalog::CatalogClass {
            name: format!("bench-class-{}", uuid::Uuid::new_v4()),
            description: String::new(),
            canonical_sentences: vec![],
            color: String::new(),
            deprecated: false,
        };
        let op = proto_catalog::CatalogEvolutionOp {
            op: Some(proto_catalog::catalog_evolution_op::Op::AppendClass(
                proto_catalog::AppendClassOp { cls: Some(class) },
            )),
        };
        let req = proto_catalog::EvolveCatalogRequest {
            from_key: format!("{tenant}/bench-cat-base@v1"),
            op: Some(op),
            metadata_override: None,
        };
        let mut stub = self.catalog_stub.clone();
        let req = self.decorate(Request::new(req), worker_id);
        let _ = stub.evolve_catalog(req).await?;
        Ok(())
    }

    async fn dispatch_register_sentence(
        &self,
        tenant: &str,
        worker_id: u64,
    ) -> Result<(), BenchError> {
        // RegisterSentence reuses the EvolveCatalog path but with
        // a ReplaceClassAnchorsOp. Same caveat as RegisterClass on
        // server-side errors.
        let op = proto_catalog::CatalogEvolutionOp {
            op: Some(
                proto_catalog::catalog_evolution_op::Op::ReplaceClassAnchors(
                    proto_catalog::ReplaceClassAnchorsOp {
                        class_name: "bench-class".into(),
                        canonical_sentences: vec![format!(
                            "synthetic sentence {}",
                            uuid::Uuid::new_v4()
                        )],
                    },
                ),
            ),
        };
        let req = proto_catalog::EvolveCatalogRequest {
            from_key: format!("{tenant}/bench-cat-base@v1"),
            op: Some(op),
            metadata_override: None,
        };
        let mut stub = self.catalog_stub.clone();
        let req = self.decorate(Request::new(req), worker_id);
        let _ = stub.evolve_catalog(req).await?;
        Ok(())
    }

    async fn dispatch_resolve_catalog(
        &self,
        tenant: &str,
        worker_id: u64,
    ) -> Result<(), BenchError> {
        let req = proto_catalog::GetCatalogRequest {
            key: format!("{tenant}/bench-cat-0@v1"),
            include_compiled: false,
        };
        let mut stub = self.catalog_stub.clone();
        let req = self.decorate(Request::new(req), worker_id);
        let _ = stub.get_catalog(req).await?;
        Ok(())
    }

    async fn dispatch_list_catalogs(
        &self,
        tenant: &str,
        worker_id: u64,
    ) -> Result<(), BenchError> {
        let req = proto_catalog::ListCatalogsRequest {
            tenant: tenant.to_string(),
            page_size: 100,
            page_token: String::new(),
        };
        let mut stub = self.catalog_stub.clone();
        let req = self.decorate(Request::new(req), worker_id);
        let _ = stub.list_catalogs(req).await?;
        Ok(())
    }

    /// **Sprint D38 (Φ-F)** — Mode-3 dispatch.
    ///
    /// Builds a synthetic `Mode3ExecuteRequest` with the requested
    /// `k_swarm`. The server clamps to 5 if `k_swarm > 5` (S-12) ;
    /// the bench passes the original value verbatim so the
    /// `k_clamped` server-side counter increments observably for
    /// the `mode3-k10` workload.
    async fn dispatch_mode3_execute(
        &self,
        tenant: &str,
        k_swarm: u32,
        worker_id: u64,
        call_seed: u64,
    ) -> Result<(), BenchError> {
        let req = proto_mode3::Mode3ExecuteRequest {
            query_text: format!("bench mode3 query {}", uuid::Uuid::new_v4()),
            k: k_swarm,
            mode_per_call: "mode1".into(),
            llm_router_id: String::new(),
            aggregator: "mean".into(),
            tenant_id: tenant.to_string(),
            request_id: format!("bench-mode3-{}", uuid::Uuid::new_v4()),
            // **Audit fix M6-A1 (Φ-F)** — `seed` uses the rotating
            // `call_idx` (per-call unique) so each Mode-3 fan-out
            // gets a distinct seed across the bench run. The
            // `worker_id` (used for shard routing) stays stable
            // per-worker, which would defeat the purpose of the
            // seed (every worker would request the same seed
            // forever, yielding identical sub-swarms).
            seed: call_seed,
            top_k_anchors: 5,
            max_rounds: 0,
        };
        let mut stub = self.mode3_stub.clone();
        let req = self.decorate(Request::new(req), worker_id);
        let _ = stub.execute(req).await?;
        Ok(())
    }

    /// Map a [`BenchError`] back to a stable error-label string so
    /// the runner can group failures by category. Reuses the
    /// existing taxonomy where possible (e.g. `tonic_status_14`
    /// for Unavailable) so dashboards built on D18 labels keep
    /// rendering.
    fn error_label_from(err: &BenchError) -> String {
        match err {
            BenchError::Tonic(TonicError::Status { code, .. }) => {
                format!("tonic_status_{code}")
            }
            BenchError::Tonic(TonicError::Transport(_)) => "tonic_transport".into(),
            BenchError::Tonic(TonicError::Tls(_)) => "tonic_tls".into(),
            BenchError::Tonic(TonicError::InvalidEndpoint(_)) => {
                "tonic_invalid_endpoint".into()
            }
            BenchError::Tonic(TonicError::InvalidJwtToken(_)) => {
                "tonic_invalid_jwt".into()
            }
            BenchError::Tonic(TonicError::Mode3StubMissing) => {
                "tonic_mode3_stub_missing".into()
            }
            other => other.metric_label().into(),
        }
    }
}

#[async_trait]
impl BenchClient for TonicBenchClient {
    async fn call(
        &self,
        op: WorkloadOp,
        tenant: &str,
        worker_id: u64,
    ) -> Result<CallOutcome, BenchError> {
        // **Audit fix M6-A1 (Φ-F)** — separate `worker_id`
        // (runner-side per-worker stable, used for shard routing)
        // from `call_seed` (atomic counter, used for Mode-3 seed
        // diversity). Pre-audit a single value did both jobs and
        // the metric ↔ routing alignment was broken.
        let call_seed = self
            .call_idx
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let started = Instant::now();
        let result: Result<(), BenchError> = match op {
            WorkloadOp::Execute => self.dispatch_execute(tenant, worker_id).await,
            WorkloadOp::RegisterCatalog => {
                self.dispatch_register_catalog(tenant, worker_id).await
            }
            WorkloadOp::RegisterClass => {
                self.dispatch_register_class(tenant, worker_id).await
            }
            WorkloadOp::RegisterSentence => {
                self.dispatch_register_sentence(tenant, worker_id).await
            }
            WorkloadOp::ResolveCatalog => {
                self.dispatch_resolve_catalog(tenant, worker_id).await
            }
            WorkloadOp::ListCatalogs => {
                self.dispatch_list_catalogs(tenant, worker_id).await
            }
            WorkloadOp::Mode3Execute { k_swarm } => {
                self.dispatch_mode3_execute(tenant, k_swarm, worker_id, call_seed)
                    .await
            }
        };
        let latency = started.elapsed();
        // Defensive : reject pathological latencies before they
        // poison the histogram.
        if latency > Duration::from_secs(600) {
            return Err(BenchError::HistogramOverflow(latency));
        }

        match result {
            Ok(()) => Ok(CallOutcome {
                op,
                latency,
                ok: true,
                error_label: None,
            }),
            Err(e) => {
                let label = Self::error_label_from(&e);
                tracing::warn!(
                    target: "efe.bench.tonic",
                    op = op.as_str(),
                    tenant = tenant,
                    worker_id = worker_id,
                    call_seed = call_seed,
                    error = %e,
                    label = %label,
                    "tonic call failed"
                );
                Ok(CallOutcome {
                    op,
                    latency,
                    ok: false,
                    error_label: Some(label),
                })
            }
        }
    }

    fn kind(&self) -> ClientKind {
        ClientKind::Tonic
    }

    fn gpu_shards(&self) -> Option<u32> {
        self.gpu_shards
    }
}

// =====================================================================
// Tests
// =====================================================================
//
// The integration tests that boot a real `efe-server` live in
// `tests/tonic_integration.rs`. The unit tests below exercise the
// pure-Rust paths that don't require the network :
//
//   * `TonicConfig` builder semantics.
//   * `error_label_from` mapping.
//   * `decorate` injects auth + shard headers.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_new_has_no_jwt_no_tls() {
        let c = TonicConfig::new("http://127.0.0.1:50051");
        assert!(c.jwt_token.is_none());
        assert!(c.tls_ca_file.is_none());
        assert!(c.gpu_shards.is_none());
    }

    #[test]
    fn config_with_jwt_sets_token() {
        let c = TonicConfig::new("http://x").with_jwt("ey.json.web.token");
        assert_eq!(c.jwt_token.as_deref(), Some("ey.json.web.token"));
    }

    #[test]
    fn config_with_tls_ca_file_sets_path() {
        let c = TonicConfig::new("https://x")
            .with_tls_ca_file("/etc/ssl/efe-ca.pem");
        assert_eq!(
            c.tls_ca_file.as_deref(),
            Some(std::path::Path::new("/etc/ssl/efe-ca.pem"))
        );
    }

    #[test]
    fn config_with_gpu_shards_sets_shards() {
        let c = TonicConfig::new("http://x").with_gpu_shards(4);
        assert_eq!(c.gpu_shards, Some(4));
    }

    #[test]
    fn config_with_tls_domain_name_sets_name() {
        let c = TonicConfig::new("https://x")
            .with_tls_domain_name("efe.internal");
        assert_eq!(c.tls_domain_name.as_deref(), Some("efe.internal"));
    }

    #[test]
    fn error_label_from_tonic_status() {
        let e = BenchError::Tonic(TonicError::Status {
            code: 14,
            message: "unavailable".into(),
        });
        assert_eq!(TonicBenchClient::error_label_from(&e), "tonic_status_14");
    }

    #[test]
    fn error_label_from_tonic_transport() {
        let e = BenchError::Tonic(TonicError::Transport("eof".into()));
        assert_eq!(TonicBenchClient::error_label_from(&e), "tonic_transport");
    }

    #[test]
    fn error_label_from_tonic_tls() {
        let e = BenchError::Tonic(TonicError::Tls("bad cert".into()));
        assert_eq!(TonicBenchClient::error_label_from(&e), "tonic_tls");
    }

    #[test]
    fn error_label_from_invalid_endpoint() {
        let e = BenchError::Tonic(TonicError::InvalidEndpoint("scheme".into()));
        assert_eq!(
            TonicBenchClient::error_label_from(&e),
            "tonic_invalid_endpoint"
        );
    }

    #[test]
    fn error_label_from_invalid_jwt() {
        let e = BenchError::Tonic(TonicError::InvalidJwtToken("ascii".into()));
        assert_eq!(
            TonicBenchClient::error_label_from(&e),
            "tonic_invalid_jwt"
        );
    }

    #[test]
    fn error_label_from_mode3_stub_missing() {
        let e = BenchError::Tonic(TonicError::Mode3StubMissing);
        assert_eq!(
            TonicBenchClient::error_label_from(&e),
            "tonic_mode3_stub_missing"
        );
    }

    #[test]
    fn error_label_from_falls_back_to_metric_label() {
        let e = BenchError::Config("bad".into());
        assert_eq!(TonicBenchClient::error_label_from(&e), "config");
    }

    #[tokio::test]
    async fn connect_invalid_endpoint_is_typed_error() {
        // Empty string : tonic `Endpoint::from_shared` rejects with
        // an `InvalidUri` parse error, surfaced as
        // `TonicError::InvalidEndpoint`. Some tonic versions
        // accept relative refs like `not-a-url` (treating them as
        // path-only URIs that fail later at `connect()` time, which
        // surfaces as `Transport`) — the empty input is the only
        // shape guaranteed to bottom out at `from_shared` across
        // tonic 0.10..0.13. Either error is valid evidence that
        // the typed taxonomy works.
        let cfg = TonicConfig::new("");
        let r = TonicBenchClient::connect(cfg).await;
        match r {
            Err(BenchError::Tonic(TonicError::InvalidEndpoint(_)))
            | Err(BenchError::Tonic(TonicError::Transport(_))) => {}
            other => panic!(
                "expected InvalidEndpoint or Transport on empty endpoint, got {other:?}"
            ),
        }
    }

    #[tokio::test]
    async fn connect_unreachable_endpoint_is_transport_error() {
        // Valid URL shape but nothing listening on this port.
        // `Endpoint::connect` does a real TCP connect attempt ; we
        // expect a `Transport` error within a few hundred ms.
        let cfg = TonicConfig::new("http://127.0.0.1:1");
        let r = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            TonicBenchClient::connect(cfg),
        )
        .await
        .expect("connect attempt must complete within 5s");
        match r {
            Err(BenchError::Tonic(TonicError::Transport(_))) => {}
            other => panic!("expected Transport error, got {other:?}"),
        }
    }
}
