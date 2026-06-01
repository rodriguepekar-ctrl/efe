//! Server configuration — lives in this module to keep `main.rs` lean.
//!
//! The configuration is driven by three layers that compose left-to-right:
//!
//! 1. **Defaults** — [`ServerConfig::default`] and the nested `*Config`
//!    defaults. Every field has a sane value that works for a local
//!    developer running `cargo run`.
//! 2. **Environment variables** — [`ServerConfig::from_env`] reads the
//!    `EFE_*` namespace. Missing variables fall back to the defaults;
//!    malformed variables return [`ConfigError`] (no silent coercion).
//! 3. **Validation** — [`ServerConfig::validate`] checks cross-field
//!    invariants that cannot be expressed in the type system alone
//!    (e.g. `shutdown.force_abort_after >= shutdown.grace_period`, or
//!    `auth.enabled && hs256_secrets.is_empty()` → refusal).
//!
//! Secrets (`hs256_secrets`) are **never** read from environment
//! variables — they are loaded from a file whose path is given by
//! `EFE_JWT_SECRETS_FILE` (default `/etc/efe/secrets/hs256`). Each line
//! of the file is a base64-url-encoded key ; the first line is the
//! **active** (signing) key, subsequent lines are verification-only
//! keys used during rotation (M12.9 §15.4).
//!
//! ## Precedence (M12 §8.4)
//!
//! ```text
//! defaults  <  environment variables  <  secret file (for hs256 only)
//! ```
//!
//! No CLI arguments are consumed in the MVP. Any `argv[1..]` passed to
//! the binary is logged as a warning and ignored to keep Helm values
//! and `docker run` flags as the single source of configuration truth.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment variable prefix (for documentation / tests).
pub const ENV_PREFIX: &str = "EFE_";

/// Default path where the binary looks for HS256 secret material.
///
/// The file is expected to have one base64-url-encoded key per line.
/// A chart-provided `ExternalSecret` mounts the sibling `hs256` file
/// under this parent directory.
pub const DEFAULT_SECRETS_FILE: &str = "/etc/efe/secrets/hs256";

/// Full server configuration. All fields have sane defaults.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ServerConfig {
    /// Address to bind. Default `127.0.0.1:50051`.
    pub listen_addr: SocketAddr,
    /// Per-connection concurrency limit (tower `concurrency_limit`).
    pub max_concurrent_streams: usize,
    /// Per-call timeout (applied as a tower `timeout` layer).
    pub request_timeout: Duration,
    /// Auth config.
    pub auth: AuthConfig,
    /// Rate-limit config.
    pub ratelimit: RateLimitConfig,
    /// Idempotency cache config.
    pub idempotency: IdempotencyConfig,
    /// Graceful shutdown parameters.
    pub shutdown: ShutdownConfig,
    /// Whether to serve the admin API (requires a registry injected).
    pub enable_admin: bool,
    /// Sprint **D12-bis** — catalog admin service config.
    pub catalog: CatalogConfig,
    /// Sprint **D16** — embedder bundling config.
    pub embedder: EmbedderConfig,
    /// Sprint **D18** — multi-replica + leader election + 3-phase
    /// shutdown configuration.
    pub multi_replica: MultiReplicaConfig,
    /// Sprint **D28 (Φ-D)** — GPU compute substrate runtime config.
    /// Cf. `docs/EFE_KERNELS_ARCHITECTURE.md` § 19.1.
    pub runtime: RuntimeConfig,
    /// **Phase-2 PR #11** — meta-swarm subsystem flag + gRPC client config.
    /// Plumbing only ; the client is built but not yet dispatched.
    /// Cf. `docs/EFE_A1_PHASE2_PR11_ARCHITECTURE.md`.
    pub meta_swarm: MetaSwarmConfig,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1:50051".parse().expect("valid default addr"),
            max_concurrent_streams: 1024,
            request_timeout: Duration::from_secs(30),
            auth: AuthConfig::default(),
            ratelimit: RateLimitConfig::default(),
            idempotency: IdempotencyConfig::default(),
            shutdown: ShutdownConfig::default(),
            enable_admin: true,
            catalog: CatalogConfig::default(),
            embedder: EmbedderConfig::default(),
            multi_replica: MultiReplicaConfig::default(),
            runtime: RuntimeConfig::default(),
            meta_swarm: MetaSwarmConfig::default(),
        }
    }
}

// =====================================================================
// Phase-2 PR #11 — meta-swarm subsystem config
// =====================================================================

/// Phase-2 PR #11 — meta-swarm gRPC subsystem config.
///
/// Master switch `enabled` (default `false`) gates the entire
/// subsystem.  When `true`, `efe-server` builds a `MetaSwarmClient`
/// at startup that connects to `controller_endpoint` ; the client is
/// held but NOT consumed by the request path in PR #11 (plumbing
/// only).  PR #15 will route specific multi-swarm coordination
/// calls through it.
///
/// Env-var contract (cf.
/// `docs/EFE_A1_PHASE2_PR11_ARCHITECTURE.md` § 7) :
///
/// | env var | type | default |
/// |---|---|---|
/// | `EFE_META_SWARM_ENABLED` | bool | `false` |
/// | `EFE_META_SWARM_CONTROLLER_ENDPOINT` | URL | `http://127.0.0.1:50080` |
/// | `EFE_META_SWARM_CONNECT_TIMEOUT_MS` | u64 | `5000` |
/// | `EFE_META_SWARM_CALL_TIMEOUT_MS` | u64 | `20000` |
/// | `EFE_META_SWARM_HEALTH_PROBE_AT_STARTUP` | bool | `true` |
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct MetaSwarmConfig {
    /// Master switch.  Default `false` — pre-PR-#11 behaviour
    /// (byte-identical to existing deployments).
    pub enabled: bool,

    /// gRPC endpoint of the meta-swarm controller.  Only consulted
    /// when `enabled` is `true`.  Default `http://127.0.0.1:50080`.
    pub controller_endpoint: String,

    /// `tonic::transport::Endpoint::connect_timeout(_)` value.
    pub connect_timeout: Duration,

    /// `tonic::transport::Endpoint::timeout(_)` default for RPC
    /// calls.  PR #15 dispatch will inherit this default.
    pub call_timeout: Duration,

    /// If `true`, probe `MetaSwarmService::Health` once at startup ;
    /// refuse to boot if it returns anything other than SERVING.
    /// Set to `false` for staged rollouts where the controller is
    /// known to be slow to start.
    pub health_probe_at_startup: bool,
}

impl Default for MetaSwarmConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            controller_endpoint: "http://127.0.0.1:50080".to_string(),
            connect_timeout: Duration::from_secs(5),
            call_timeout: Duration::from_secs(20),
            health_probe_at_startup: true,
        }
    }
}

// =====================================================================
// Sprint D28 (Φ-D) — GPU runtime config
// =====================================================================

/// Sprint **D28** — GPU compute substrate configuration.
///
/// **Audit fix K-A2** — pre-fix `efe-kernels` shipped a fully-formed
/// `CudaBackend` but no env var path could enable it ; an operator
/// setting `EFE_RUNTIME_GPU_ENABLED=true` in the Helm ConfigMap got
/// no effect. This config struct flows the toggles through.
///
/// All fields are operator-tunable via env vars. The runtime
/// honours them in `Server::build` :
///
///   * if `gpu_enabled = false` (default) → CPU-only path.
///   * if `gpu_enabled = true` AND feature `cuda` compiled →
///     instantiate `efe_kernels::CudaBackend` + register the 6 GPU
///     instruments via `efe_telemetry::metrics::register_all_gpu`.
///   * if `gpu_enabled = true` BUT feature `cuda` NOT compiled →
///     log warning "GPU requested but CUDA feature absent" + fall
///     back to CPU.
///
/// Defaults are safe : single GPU device 0, 80 % VRAM ceiling, 4
/// streams.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RuntimeConfig {
    /// Master switch — when `true`, the runtime tries to use a GPU
    /// for the compute substrate. When `false`, CPU only.
    pub gpu_enabled: bool,
    /// Device ordinal (0-based). Default 0 = first visible GPU.
    /// Multi-GPU sharding (D30) consumes a *list* of ordinals via
    /// `gpu_shard_ordinals` — this single-ordinal field is the
    /// fallback for non-sharded runs.
    pub gpu_device_ordinal: u32,
    /// VRAM high-water mark percent (1-95). Refuses allocations
    /// above this level. Default 80.
    pub gpu_memory_hwm_percent: u8,
    /// Stream pool size. Default 4 (matches `StreamPool::with_default_size`).
    pub gpu_streams: u32,
    /// Comma-separated list of ordinals for multi-GPU sharding
    /// (D30). Empty = single-GPU. Parsed at boot ; invalid entries
    /// fall back to `gpu_device_ordinal`.
    pub gpu_shard_ordinals: String,
    /// Fallback policy when a GPU op fails mid-request.
    /// `"strict"` (default) → return ResourceExhausted to client.
    /// `"cpu_on_error"` → re-execute on `CpuBackend` ; log warn.
    pub gpu_fallback_policy: String,
    /// Enable the debug-only `check_finite` kernel after each
    /// hot-path kernel. Off by default in prod (perf cost).
    pub gpu_check_finite: bool,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            gpu_enabled: false,
            gpu_device_ordinal: 0,
            gpu_memory_hwm_percent: 80,
            gpu_streams: 4,
            gpu_shard_ordinals: String::new(),
            gpu_fallback_policy: "strict".to_string(),
            gpu_check_finite: false,
        }
    }
}

// =====================================================================
// Sprint D18 — Multi-réplica + leader election + 3-phase shutdown
// =====================================================================

/// Sprint **D18** — Multi-replica runtime configuration.
///
/// **Audit fix D18-A1** — pre-fix the multi_replica module was
/// shipped but never read from env, so operator overrides for
/// `EFE_REPLICAS_MIN/MAX`, `EFE_LEADER_*`, `EFE_SHUTDOWN_DRAIN_RPC_MS`
/// etc. had no effect. The config layer now flows these through to
/// `ServerBuilder::build`.
///
/// All fields default-initialise to safe single-replica values so
/// existing operators on D17 continue to work without re-rolling
/// their ConfigMap.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct MultiReplicaConfig {
    /// Symbolic identity of this replica (typically `HOSTNAME`).
    /// Default `efe-pod-0` ; override with env `EFE_REPLICA_ID` or
    /// the Downward API.
    pub replica_id: String,
    /// HPA min replicas. Information-only at the runtime layer
    /// (HPA owns the actual scaling) ; surfaced in logs to help
    /// operators diagnose scaling issues.
    pub replicas_min: u32,
    /// HPA max replicas. Same role as `replicas_min`.
    pub replicas_max: u32,
    /// Whether leader-election is enabled. When `false`, the
    /// runtime spawns leader-only background tasks on every
    /// replica (acceptable when `replicas_min == 1`).
    pub leader_election_enabled: bool,
    /// Lease name (default `efe-leader`).
    pub leader_lease_name: String,
    /// Lease duration in seconds.
    pub leader_lease_duration_s: u64,
    /// Renew deadline in seconds. MUST be < lease_duration_s.
    pub leader_renew_deadline_s: u64,
    /// Phase 1 (drain RPC) timeout in milliseconds.
    pub shutdown_drain_rpc_ms: u64,
    /// Phase 2 (audit flush) timeout in milliseconds.
    pub shutdown_audit_flush_ms: u64,
    /// Phase 3 (pg_close) timeout in milliseconds.
    pub shutdown_pg_close_ms: u64,
}

impl Default for MultiReplicaConfig {
    fn default() -> Self {
        Self {
            replica_id: "efe-pod-0".to_string(),
            replicas_min: 1,
            replicas_max: 1,
            leader_election_enabled: false,
            leader_lease_name: "efe-leader".to_string(),
            leader_lease_duration_s: 15,
            leader_renew_deadline_s: 10,
            shutdown_drain_rpc_ms: 10_000,
            shutdown_audit_flush_ms: 10_000,
            shutdown_pg_close_ms: 30_000,
        }
    }
}

// =====================================================================
// Sprint D16 — Embedder bundling config
// =====================================================================

/// Sprint **D16** — embedder backend selection.
///
/// Two paths coexist :
///
///   * [`EmbedderBackend::Legacy`] (default) — pre-D16 behaviour.
///     The runtime delegates to the Python `LocalSTEmbedder` (cf.
///     `clients/python/efe_tool/embedder.py`) which lazy-fetches
///     from HuggingFace Hub on first encode. Backwards-compat for
///     every catalog fitted before D16.
///
///   * [`EmbedderBackend::Bundle`] — D16 byte-exact pinning. The
///     runtime loads a verified bundle from disk (Ed25519-signed,
///     SHA-256-checked components, frozen tokenizer + weights).
///     Required for air-gap deploys, regulated environments, and
///     reproducible-CI compliance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EmbedderBackend {
    /// Pre-D16 fetch-from-HF-Hub path. Default for back-compat.
    Legacy,
    /// D16 verified-bundle path.
    Bundle,
}

impl EmbedderBackend {
    /// Lowercase canonical name for env / log / metric labels.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            EmbedderBackend::Legacy => "legacy",
            EmbedderBackend::Bundle => "bundle",
        }
    }

    /// Parse the env-var form. Architecture § 14.1.
    pub fn from_str(s: &str) -> Result<Self, ConfigError> {
        match s {
            "legacy" => Ok(EmbedderBackend::Legacy),
            "bundle" => Ok(EmbedderBackend::Bundle),
            other => Err(ConfigError::invalid(
                "embedder.backend",
                &format!("unknown value {other:?} (expected 'legacy' or 'bundle')"),
            )),
        }
    }
}

/// Sprint **D16** — signature verification policy for the bundle path.
///
/// Three modes from strictest to most permissive :
///
///   * [`SignaturePolicy::Required`] (prod default) — bundle MUST
///     carry a `manifest.json.sig` and it MUST verify against the
///     trust store. Refused at boot if either condition fails.
///
///   * [`SignaturePolicy::Warn`] (staging) — missing signature logs
///     a warning, present-but-invalid signature still refuses. Used
///     during the staging window between writing a bundle and
///     publishing it through the signing CI.
///
///   * [`SignaturePolicy::Skip`] (Tier 1 dev only) — signature
///     ignored entirely. Refused at config-validate time when
///     `EFE_DEPLOY_TIER=production` (audit fix anticipation, not
///     yet wired since deploy_tier is not in M12 config — to add
///     in D17).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SignaturePolicy {
    /// Manifest signature required + must verify.
    Required,
    /// Missing → warn ; present invalid → reject.
    Warn,
    /// Signature ignored. Forbidden in production tier.
    Skip,
}

impl SignaturePolicy {
    /// String form for env / log labels.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            SignaturePolicy::Required => "required",
            SignaturePolicy::Warn => "warn",
            SignaturePolicy::Skip => "skip",
        }
    }

    /// Parse from env var. Architecture § 5.5.
    pub fn from_str(s: &str) -> Result<Self, ConfigError> {
        match s {
            "required" | "true" | "1" => Ok(SignaturePolicy::Required),
            "warn" => Ok(SignaturePolicy::Warn),
            "skip" | "false" | "0" => Ok(SignaturePolicy::Skip),
            other => Err(ConfigError::invalid(
                "embedder.bundle.require_signature",
                &format!(
                    "unknown value {other:?} \
                     (expected 'required' / 'warn' / 'skip')"
                ),
            )),
        }
    }
}

/// Sprint **D16** — embedder bundling configuration.
///
/// Off-by-default ; opt-in via `EFE_EMBEDDER_BACKEND=bundle`.
///
/// **Validation invariants** (enforced by [`ServerConfig::validate`]
/// when `backend = Bundle`) — cf. architecture § 14.2 :
///
/// * `bundle_path` non-empty.
/// * `trust_store_path` non-empty when `require_signature ≠ Skip`.
/// * `load_timeout_s ∈ [1, 600]` (1 sec is unrealistic ; 10 min
///   covers the worst-case CDN cold-pull on a 500 MiB bundle).
/// * `encode_timeout_ms ≥ 10` (sub-10ms is unrealistic for any
///   embedder).
/// * `jwks_backoff_ms ≤ 60_000` (paté).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct EmbedderConfig {
    /// Master switch. Default `Legacy` for back-compat.
    pub backend: EmbedderBackend,
    /// Bundle root directory on disk. Required when `backend = Bundle`.
    /// Architecture default `/etc/efe/bundles/active` matches the Helm
    /// chart's mount path (cf. `deploy/helm/efe/values.yaml`).
    pub bundle_path: String,
    /// Trust store path (Ed25519 pubkeys JSON). Architecture default
    /// `/etc/efe/bundles/pubkeys.json`.
    pub trust_store_path: String,
    /// Signature verification policy.
    pub require_signature: SignaturePolicy,
    /// SBOM requirement (architecture § 6.5). When `true`, loader
    /// refuses bundles without an `SBOM.json` listed in the manifest.
    pub require_sbom: bool,
    /// Operator acknowledgment for non-redistributable licenses
    /// (CC-BY-NC, OpenRAIL-M variants). Loader refuses if a
    /// non-redistributable bundle is loaded without this ACK.
    pub acknowledge_non_redistributable: bool,
    /// Load step timeout (seconds). Caps the worst-case time spent
    /// on file IO + hash check + signature verify + runtime init.
    pub load_timeout_s: u64,
    /// Per-encode timeout (milliseconds). Caps the worst-case
    /// inference latency. Mostly a safety net ; the hot path is
    /// expected to stay well under this.
    pub encode_timeout_ms: u64,
}

impl Default for EmbedderConfig {
    fn default() -> Self {
        Self {
            backend: EmbedderBackend::Legacy,
            // **Architecture § 7.4** — canonical mount path. The
            // Helm chart pre-creates the directory ; the operator
            // populates it via initContainer (OCI pull) OR PVC
            // pre-population (air-gap).
            bundle_path: "/etc/efe/bundles/active".to_string(),
            trust_store_path: "/etc/efe/bundles/pubkeys.json".to_string(),
            require_signature: SignaturePolicy::Required,
            require_sbom: true,
            acknowledge_non_redistributable: false,
            load_timeout_s: 60,
            encode_timeout_ms: 500,
        }
    }
}

/// Sprint **D12-bis** — catalog backend selector + tunable policy.
///
/// `backend = Memory` (default) keeps state in-process — lost on
/// every restart. `backend = Postgres` (D14) persists state in a
/// Postgres database via [`CatalogPostgresConfig`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CatalogConfig {
    /// Storage impl. `Memory` (default) or `Postgres` (D14).
    pub backend: CatalogBackend,
    /// Tunable quotas (mirror of `policy::DEFAULT_*` constants).
    pub policy: CatalogPolicyConfig,
    /// Sprint **D14** — Postgres connection parameters. Only consulted
    /// when `backend = Postgres` ; ignored otherwise. Sensible defaults
    /// allow boot-time validation to succeed even when the operator
    /// runs Memory mode.
    pub postgres: CatalogPostgresConfig,
    /// Sprint **D14-bis** — HMAC-chained tamper-evident audit log
    /// configuration. Off by default ; enable via
    /// `EFE_AUDIT_SIGNING_ENABLED=true`.
    pub audit_signing: AuditSigningConfig,
}

impl Default for CatalogConfig {
    fn default() -> Self {
        Self {
            backend: CatalogBackend::Memory,
            policy: CatalogPolicyConfig::default(),
            postgres: CatalogPostgresConfig::default(),
            audit_signing: AuditSigningConfig::default(),
        }
    }
}

/// Catalog backend selector. Reserved for D14 swap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CatalogBackend {
    /// In-memory `RwLock<BTreeMap>` (D12 MVP). Default. State lost
    /// on restart.
    Memory,
    /// Postgres-backed (D14). Rejected in v0.1.13 with a clear
    /// error at startup.
    Postgres,
}

/// Per-tenant quota tunables. Mirrors `policy::CatalogPolicy`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CatalogPolicyConfig {
    /// Max number of `(tenant, name)` tuples per tenant. Default 100.
    pub max_names_per_tenant: u32,
    /// Max number of versions per `(tenant, name)`. Default 1000.
    pub max_versions_per_name: u32,
    /// Max number of classes per catalog. Default 256.
    pub max_classes_per_catalog: u32,
    /// Max number of canonical sentences per class. Default 64.
    pub max_sentences_per_class: u32,
}

impl Default for CatalogPolicyConfig {
    fn default() -> Self {
        Self {
            max_names_per_tenant: 100,
            max_versions_per_name: 1000,
            max_classes_per_catalog: 256,
            max_sentences_per_class: 64,
        }
    }
}

impl CatalogPolicyConfig {
    /// Sprint **D12-bis** audit fix C1 — convert this configuration to
    /// the runtime [`crate::catalog::CatalogPolicy`] consumed by
    /// `InMemoryRegistry`. Without this conversion the runtime would
    /// silently force the binary defaults (cf. audit fix C1 in
    /// `runtime::Server::build`).
    #[must_use]
    pub fn to_policy(&self) -> crate::catalog::CatalogPolicy {
        crate::catalog::CatalogPolicy {
            max_names_per_tenant: self.max_names_per_tenant,
            max_versions_per_name: self.max_versions_per_name,
            max_classes_per_catalog: self.max_classes_per_catalog,
            max_sentences_per_class: self.max_sentences_per_class,
        }
    }
}

impl From<&CatalogPolicyConfig> for crate::catalog::CatalogPolicy {
    fn from(cfg: &CatalogPolicyConfig) -> Self {
        cfg.to_policy()
    }
}

// =====================================================================
// Sprint D14 — Postgres backend configuration
// =====================================================================

/// SSL mode for the Postgres connection. Mirrors `sslmode` values
/// supported by libpq / sqlx ; the runtime renders this enum to the
/// DSN query string.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PgSslMode {
    /// No TLS at all. Tier 1 dev only.
    Disable,
    /// Use TLS if the server supports it. Default — fallback to
    /// plaintext silently.
    Prefer,
    /// TLS required ; connection fails if the server cannot
    /// negotiate.
    Require,
    /// TLS required + verify the server certificate against the
    /// system CA bundle.
    VerifyCa,
    /// TLS required + verify CA + match the hostname.
    VerifyFull,
}

impl PgSslMode {
    /// String form expected by the Postgres DSN (`sslmode=...` query
    /// parameter).
    #[must_use]
    pub const fn as_dsn_value(&self) -> &'static str {
        match self {
            Self::Disable => "disable",
            Self::Prefer => "prefer",
            Self::Require => "require",
            Self::VerifyCa => "verify-ca",
            Self::VerifyFull => "verify-full",
        }
    }
}

/// Sprint **D14** — Postgres connection + pool tuning for the
/// catalog backend.
///
/// All fields are configurable via `EFE_PG_*` env vars (cf. the
/// architecture document § 25 annexe E for the full matrix). The
/// `password` field is intentionally a `String` here for ergonomics ;
/// it is wrapped in [`secrecy::SecretString`] inside the runtime
/// pool builder to ensure it does not leak into Debug output.
///
/// **Validation invariants** (enforced by [`ServerConfig::validate`]
/// when `backend = Postgres`) :
///
/// * `host` non-empty.
/// * `port` ∈ [1, 65535].
/// * `database` non-empty.
/// * `user` non-empty.
/// * `pool_max_connections >= 1`.
/// * `pool_min_connections <= pool_max_connections`.
/// * `pool_acquire_timeout_ms >= 100`.
/// * `pool_max_lifetime_s >= 60`.
/// * `pool_idle_timeout_s >= 60`.
/// * `password` non-empty (only when SSL mode requires authentication
///   — left to the operator to validate ; we don't refuse empty
///   passwords because some local-dev setups use trust auth).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CatalogPostgresConfig {
    /// Hostname or IP, e.g. `efe-pg.efe-catalog.svc.cluster.local`.
    pub host: String,
    /// TCP port. Default 5432.
    pub port: u16,
    /// Database name.
    pub database: String,
    /// User.
    pub user: String,
    /// Password. May be empty for trust auth (local dev).
    pub password: String,
    /// SSL mode.
    pub ssl_mode: PgSslMode,
    /// Pool : maximum concurrent connections.
    pub pool_max_connections: u32,
    /// Pool : minimum kept-warm connections.
    pub pool_min_connections: u32,
    /// Pool : acquire timeout, in milliseconds.
    pub pool_acquire_timeout_ms: u64,
    /// Pool : maximum lifetime of a connection, in seconds.
    pub pool_max_lifetime_s: u64,
    /// Pool : idle timeout before reaping, in seconds.
    pub pool_idle_timeout_s: u64,
    /// Pool : ping each connection before handing it out.
    pub pool_test_before_acquire: bool,
}

impl Default for CatalogPostgresConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 5432,
            database: "efe_catalog_db".to_string(),
            user: "efe_catalog".to_string(),
            password: String::new(),
            ssl_mode: PgSslMode::Prefer,
            pool_max_connections: 8,
            pool_min_connections: 2,
            pool_acquire_timeout_ms: 5000,
            pool_max_lifetime_s: 1800,
            pool_idle_timeout_s: 600,
            pool_test_before_acquire: true,
        }
    }
}

impl CatalogPostgresConfig {
    /// Render the DSN consumed by `sqlx::PgPool::connect`.
    ///
    /// Format : `postgres://user[:password]@host:port/database?sslmode=…`.
    /// The password is URL-encoded ; if empty, the `:password` segment
    /// is omitted entirely (some PG configs distinguish empty password
    /// from absent password — we want the latter for trust auth).
    #[must_use]
    pub fn dsn(&self) -> String {
        let user_enc = url_encode(&self.user);
        let user_part = if self.password.is_empty() {
            user_enc
        } else {
            format!("{}:{}", user_enc, url_encode(&self.password))
        };
        format!(
            "postgres://{}@{}:{}/{}?sslmode={}",
            user_part,
            self.host,
            self.port,
            url_encode(&self.database),
            self.ssl_mode.as_dsn_value(),
        )
    }

    /// Render a redacted DSN suitable for logging — the password is
    /// replaced by `***`. Used by tracing instrumentation that wants
    /// to surface the connection target without leaking secrets.
    #[must_use]
    pub fn dsn_redacted(&self) -> String {
        let user_enc = url_encode(&self.user);
        let user_part = if self.password.is_empty() {
            user_enc
        } else {
            format!("{}:***", user_enc)
        };
        format!(
            "postgres://{}@{}:{}/{}?sslmode={}",
            user_part,
            self.host,
            self.port,
            url_encode(&self.database),
            self.ssl_mode.as_dsn_value(),
        )
    }
}

/// Minimal RFC 3986 percent-encoding for the DSN userinfo / path
/// segments. We only encode the characters that have special meaning
/// inside a DSN (`@`, `:`, `/`, `?`, `#`, ` `, `%`). For all other
/// characters we trust the operator to produce ASCII-clean values
/// (this is consistent with the `EFE_PG_USER` / `EFE_PG_DATABASE`
/// validation rules — we reject non-ASCII and shell metacharacters).
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char);
            }
            other => {
                out.push('%');
                out.push_str(&format!("{:02X}", other));
            }
        }
    }
    out
}

// =====================================================================
// Sprint D14-bis — Audit signing (HMAC-chained tamper-evident log)
// =====================================================================

/// Sprint **D14-bis** — HMAC-chained audit log signing configuration.
///
/// **Off by default**. Enabling this turns the legacy
/// `tracing::info!(target = "audit", …)` sink (D12) into the chained
/// implementation `HmacChainedAuditSink` (D14-bis), which signs every
/// audit event with HMAC-SHA-256 and persists it in the
/// `audit_events` SQL table.
///
/// **Validation invariants** (enforced by [`ServerConfig::validate`]
/// when `enabled = true`) :
///
/// * `keys_file` non-empty.
/// * `self_verify_every_s ∈ {0} ∪ [60, 86400]` (0 disables ; values < 60
///   are wasteful, > 24 h is a stale-detection too-long).
/// * Backend must be `Postgres` (the sink uses the PG pool).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AuditSigningConfig {
    /// Master switch. When false, the audit emitter uses the
    /// D12 `TracingAuditSink` (no chain, no signature).
    pub enabled: bool,
    /// Path to the file containing the HMAC keys (one per line, each
    /// base64-url-encoded, exactly 32 raw bytes after decode). The
    /// **first** non-comment line is the active signing key ; the
    /// rest are kept for verification of historical events (key
    /// rotation pattern reused verbatim from M12 HS256 secrets).
    pub keys_file: String,
    /// When true, every event signed by `HmacChainedAuditSink` is
    /// **also** forwarded to the legacy `TracingAuditSink` so that
    /// existing Loki / Splunk pipelines continue to receive the audit
    /// stream. Default `true` — preserves observability continuity.
    pub chain_to_tracing: bool,
    /// Periodic self-verify cadence, in seconds. The runtime spawns
    /// a background task that re-verifies the chain end-to-end every
    /// `self_verify_every_s` seconds and increments
    /// `efe_audit_chain_break_detected_total` on any tamper detection.
    /// Set to `0` to disable. Default 3600 (1 h).
    pub self_verify_every_s: u64,
}

impl Default for AuditSigningConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            keys_file: "/etc/efe/secrets/audit-hmac/hmac.keys".to_string(),
            chain_to_tracing: true,
            self_verify_every_s: 3600,
        }
    }
}

impl ServerConfig {
    /// Build a configuration from process environment variables.
    ///
    /// Every variable is **optional** — missing variables fall back to
    /// [`ServerConfig::default`]. Malformed variables produce a
    /// [`ConfigError`] with the offending variable name, so operators
    /// get an immediate fail-fast with an actionable error.
    ///
    /// # Variables read
    ///
    /// See `M12_ARCHITECTURE.md` §8.1 / Annexe F for the full matrix.
    /// In short:
    ///
    /// * `EFE_LISTEN_ADDR` — socket address (e.g. `0.0.0.0:50051`).
    /// * `EFE_REQUEST_TIMEOUT_MS` — u64 milliseconds.
    /// * `EFE_MAX_CONCURRENT_STREAMS` — usize.
    /// * `EFE_ENABLE_ADMIN` — boolean (`1/true/yes/on` or `0/false/no/off`).
    /// * `EFE_AUTH_*`, `EFE_RATELIMIT_*`, `EFE_IDEMPOTENCY_*`,
    ///   `EFE_SHUTDOWN_*` — nested fields.
    /// * `EFE_JWT_SECRETS_FILE` — path to HS256 key file (loaded eagerly
    ///   if `EFE_AUTH_ENABLED` is true, otherwise left empty).
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_env_with(std::env::var)
    }

    /// Test-friendly variant of [`Self::from_env`] that takes a custom
    /// env lookup function. This lets unit tests hermetically supply
    /// variables without mutating the real process environment (which
    /// would break test parallelism).
    pub fn from_env_with<F>(lookup: F) -> Result<Self, ConfigError>
    where
        F: Fn(&'static str) -> Result<String, std::env::VarError>,
    {
        let mut cfg = Self::default();

        if let Some(v) = read_str(&lookup, "EFE_LISTEN_ADDR")? {
            cfg.listen_addr = v
                .parse()
                .map_err(|e| ConfigError::parse("EFE_LISTEN_ADDR", format!("{e}")))?;
        }
        if let Some(v) = read_u64(&lookup, "EFE_REQUEST_TIMEOUT_MS")? {
            cfg.request_timeout = Duration::from_millis(v);
        }
        if let Some(v) = read_usize(&lookup, "EFE_MAX_CONCURRENT_STREAMS")? {
            cfg.max_concurrent_streams = v;
        }
        if let Some(v) = read_bool(&lookup, "EFE_ENABLE_ADMIN")? {
            cfg.enable_admin = v;
        }

        // --- Auth ---
        if let Some(v) = read_bool(&lookup, "EFE_AUTH_ENABLED")? {
            cfg.auth.enabled = v;
        }
        if let Some(v) = read_str(&lookup, "EFE_JWT_AUDIENCE")? {
            cfg.auth.jwt_audience = v;
        }
        if let Some(v) = read_str(&lookup, "EFE_JWT_ISSUER")? {
            cfg.auth.jwt_issuer = Some(v);
        }
        if let Some(v) = read_bool(&lookup, "EFE_AUTH_ALLOW_INSECURE_DEV")? {
            cfg.auth.allow_insecure_dev = v;
        }
        // Secrets: read from the file pointed to by `EFE_JWT_SECRETS_FILE`
        // — or the default — **only** when auth is actually enabled.
        // Otherwise we do not touch the filesystem, which keeps unit
        // tests hermetic and avoids bogus file-not-found errors in dev.
        if cfg.auth.enabled {
            let secrets_file = read_str(&lookup, "EFE_JWT_SECRETS_FILE")?
                .unwrap_or_else(|| DEFAULT_SECRETS_FILE.to_string());
            cfg.auth.hs256_secrets = load_hs256_secrets(Path::new(&secrets_file))?;
        }

        // --- Rate limit ---
        if let Some(v) = read_bool(&lookup, "EFE_RATELIMIT_ENABLED")? {
            cfg.ratelimit.enabled = v;
        }
        if let Some(v) = read_u32(&lookup, "EFE_RATELIMIT_CAPACITY")? {
            cfg.ratelimit.default_capacity = v;
        }
        if let Some(v) = read_f32(&lookup, "EFE_RATELIMIT_REFILL")? {
            cfg.ratelimit.default_refill_rate = v;
        }
        if let Some(v) = read_u32(&lookup, "EFE_RATELIMIT_ANON_CAP")? {
            cfg.ratelimit.anonymous_capacity = v;
        }
        if let Some(v) = read_f32(&lookup, "EFE_RATELIMIT_ANON_REFILL")? {
            cfg.ratelimit.anonymous_refill_rate = v;
        }

        // --- Idempotency ---
        if let Some(v) = read_bool(&lookup, "EFE_IDEMPOTENCY_ENABLED")? {
            cfg.idempotency.enabled = v;
        }
        if let Some(v) = read_usize(&lookup, "EFE_IDEMPOTENCY_CAPACITY")? {
            cfg.idempotency.cache_capacity = v;
        }
        if let Some(v) = read_u64(&lookup, "EFE_IDEMPOTENCY_TTL_MS")? {
            cfg.idempotency.ttl = Duration::from_millis(v);
        }

        // --- Shutdown ---
        // `EFE_SHUTDOWN_DRAIN_MS` is the public-facing name (matches
        // the M12 architecture doc) — internally it maps to
        // `grace_period`. The older `EFE_SHUTDOWN_GRACE_MS` alias is
        // still accepted for backwards compatibility with pre-M12
        // Helm charts.
        let drain_ms = read_u64(&lookup, "EFE_SHUTDOWN_DRAIN_MS")?
            .or(read_u64(&lookup, "EFE_SHUTDOWN_GRACE_MS")?);
        if let Some(v) = drain_ms {
            cfg.shutdown.grace_period = Duration::from_millis(v);
        }
        if let Some(v) = read_u64(&lookup, "EFE_SHUTDOWN_FORCE_ABORT_MS")? {
            cfg.shutdown.force_abort_after = Duration::from_millis(v);
        }

        // --- Sprint D12-bis : Catalog ---
        if let Some(v) = read_str(&lookup, "EFE_CATALOG_BACKEND")? {
            cfg.catalog.backend = match v.as_str() {
                "memory" | "Memory" => CatalogBackend::Memory,
                "postgres" | "Postgres" => CatalogBackend::Postgres,
                other => {
                    return Err(ConfigError::parse(
                        "EFE_CATALOG_BACKEND",
                        format!("{other:?} ; expected `memory` or `postgres`"),
                    ));
                }
            };
        }
        if let Some(v) = read_u32(&lookup, "EFE_CATALOG_MAX_NAMES_PER_TENANT")? {
            cfg.catalog.policy.max_names_per_tenant = v;
        }
        if let Some(v) = read_u32(&lookup, "EFE_CATALOG_MAX_VERSIONS_PER_NAME")? {
            cfg.catalog.policy.max_versions_per_name = v;
        }
        if let Some(v) = read_u32(&lookup, "EFE_CATALOG_MAX_CLASSES_PER_CATALOG")? {
            cfg.catalog.policy.max_classes_per_catalog = v;
        }
        if let Some(v) = read_u32(&lookup, "EFE_CATALOG_MAX_SENTENCES_PER_CLASS")? {
            cfg.catalog.policy.max_sentences_per_class = v;
        }

        // --- Sprint D14 — Postgres backend ---
        if let Some(v) = read_str(&lookup, "EFE_PG_HOST")? {
            cfg.catalog.postgres.host = v;
        }
        if let Some(v) = read_u16(&lookup, "EFE_PG_PORT")? {
            cfg.catalog.postgres.port = v;
        }
        if let Some(v) = read_str(&lookup, "EFE_PG_DATABASE")? {
            cfg.catalog.postgres.database = v;
        }
        if let Some(v) = read_str(&lookup, "EFE_PG_USER")? {
            cfg.catalog.postgres.user = v;
        }
        if let Some(v) = read_str(&lookup, "EFE_PG_PASSWORD")? {
            cfg.catalog.postgres.password = v;
        }
        if let Some(v) = read_str(&lookup, "EFE_PG_SSL_MODE")? {
            cfg.catalog.postgres.ssl_mode = match v.as_str() {
                "disable" | "Disable" => PgSslMode::Disable,
                "prefer" | "Prefer" => PgSslMode::Prefer,
                "require" | "Require" => PgSslMode::Require,
                "verify-ca" | "VerifyCa" | "verify_ca" => PgSslMode::VerifyCa,
                "verify-full" | "VerifyFull" | "verify_full" => PgSslMode::VerifyFull,
                other => {
                    return Err(ConfigError::parse(
                        "EFE_PG_SSL_MODE",
                        format!(
                            "{other:?} ; expected one of: disable | prefer | require | \
                             verify-ca | verify-full"
                        ),
                    ));
                }
            };
        }
        if let Some(v) = read_u32(&lookup, "EFE_PG_POOL_MAX_CONNECTIONS")? {
            cfg.catalog.postgres.pool_max_connections = v;
        }
        if let Some(v) = read_u32(&lookup, "EFE_PG_POOL_MIN_CONNECTIONS")? {
            cfg.catalog.postgres.pool_min_connections = v;
        }
        if let Some(v) = read_u64(&lookup, "EFE_PG_POOL_ACQUIRE_TIMEOUT_MS")? {
            cfg.catalog.postgres.pool_acquire_timeout_ms = v;
        }
        if let Some(v) = read_u64(&lookup, "EFE_PG_POOL_MAX_LIFETIME_S")? {
            cfg.catalog.postgres.pool_max_lifetime_s = v;
        }
        if let Some(v) = read_u64(&lookup, "EFE_PG_POOL_IDLE_TIMEOUT_S")? {
            cfg.catalog.postgres.pool_idle_timeout_s = v;
        }
        if let Some(v) = read_bool(&lookup, "EFE_PG_POOL_TEST_BEFORE_ACQUIRE")? {
            cfg.catalog.postgres.pool_test_before_acquire = v;
        }

        // --- Sprint D15 — OIDC backend ---
        if let Some(v) = read_bool(&lookup, "EFE_OIDC_ENABLED")? {
            cfg.auth.oidc.enabled = v;
        }
        if let Some(v) = read_str(&lookup, "EFE_OIDC_ISSUERS_FILE")? {
            cfg.auth.oidc.issuers_file = v;
        }
        if let Some(v) = read_u64(&lookup, "EFE_OIDC_JWKS_TTL_S")? {
            cfg.auth.oidc.jwks_ttl_s = v;
        }
        if let Some(v) = read_u64(&lookup, "EFE_OIDC_JWKS_FETCH_TIMEOUT_MS")? {
            cfg.auth.oidc.jwks_fetch_timeout_ms = v;
        }
        if let Some(v) = read_u32(&lookup, "EFE_OIDC_JWKS_RETRY_MAX")? {
            cfg.auth.oidc.jwks_retry_max = v;
        }
        if let Some(v) = read_u64(&lookup, "EFE_OIDC_JWKS_BACKOFF_MS")? {
            cfg.auth.oidc.jwks_backoff_ms = v;
        }
        if let Some(v) = read_u64(&lookup, "EFE_OIDC_GRACE_STALE_S")? {
            cfg.auth.oidc.grace_stale_s = v;
        }

        // --- Sprint D14-bis — audit signing ---
        if let Some(v) = read_bool(&lookup, "EFE_AUDIT_SIGNING_ENABLED")? {
            cfg.catalog.audit_signing.enabled = v;
        }
        if let Some(v) = read_str(&lookup, "EFE_AUDIT_HMAC_SECRETS_FILE")? {
            cfg.catalog.audit_signing.keys_file = v;
        }
        if let Some(v) = read_bool(&lookup, "EFE_AUDIT_CHAIN_TO_TRACING")? {
            cfg.catalog.audit_signing.chain_to_tracing = v;
        }
        if let Some(v) = read_u64(&lookup, "EFE_AUDIT_SELF_VERIFY_EVERY_S")? {
            cfg.catalog.audit_signing.self_verify_every_s = v;
        }

        // --- Sprint D16 — Embedder bundling ---
        if let Some(v) = read_str(&lookup, "EFE_EMBEDDER_BACKEND")? {
            cfg.embedder.backend = EmbedderBackend::from_str(&v)?;
        }
        if let Some(v) = read_str(&lookup, "EFE_BUNDLE_PATH")? {
            cfg.embedder.bundle_path = v;
        }
        if let Some(v) = read_str(&lookup, "EFE_BUNDLE_TRUST_STORE")? {
            cfg.embedder.trust_store_path = v;
        }
        if let Some(v) = read_str(&lookup, "EFE_BUNDLE_REQUIRE_SIGNATURE")? {
            cfg.embedder.require_signature = SignaturePolicy::from_str(&v)?;
        }
        if let Some(v) = read_bool(&lookup, "EFE_BUNDLE_REQUIRE_SBOM")? {
            cfg.embedder.require_sbom = v;
        }
        if let Some(v) = read_bool(&lookup, "EFE_BUNDLE_ACK_NONREDIST")? {
            cfg.embedder.acknowledge_non_redistributable = v;
        }
        if let Some(v) = read_u64(&lookup, "EFE_BUNDLE_LOAD_TIMEOUT_S")? {
            cfg.embedder.load_timeout_s = v;
        }
        if let Some(v) = read_u64(&lookup, "EFE_EMBEDDER_ENCODE_TIMEOUT_MS")? {
            cfg.embedder.encode_timeout_ms = v;
        }

        // --- Sprint D18 — Multi-replica + leader election + shutdown ---
        // **Audit fix D18-A1** — read every D18 env var so operator
        // overrides actually reach the runtime.
        // `HOSTNAME` is the K8s pod hostname (set automatically by
        // kubelet) ; `EFE_REPLICA_ID` is the explicit override path.
        if let Some(v) = read_str(&lookup, "EFE_REPLICA_ID")? {
            cfg.multi_replica.replica_id = v;
        } else if let Some(v) = read_str(&lookup, "HOSTNAME")? {
            cfg.multi_replica.replica_id = v;
        }
        if let Some(v) = read_u32(&lookup, "EFE_REPLICAS_MIN")? {
            cfg.multi_replica.replicas_min = v;
        }
        if let Some(v) = read_u32(&lookup, "EFE_REPLICAS_MAX")? {
            cfg.multi_replica.replicas_max = v;
        }
        if let Some(v) = read_bool(&lookup, "EFE_LEADER_ELECTION_ENABLED")? {
            cfg.multi_replica.leader_election_enabled = v;
        }
        if let Some(v) = read_str(&lookup, "EFE_LEADER_LEASE_NAME")? {
            cfg.multi_replica.leader_lease_name = v;
        }
        if let Some(v) = read_u64(&lookup, "EFE_LEADER_LEASE_DURATION_S")? {
            cfg.multi_replica.leader_lease_duration_s = v;
        }
        if let Some(v) = read_u64(&lookup, "EFE_LEADER_RENEW_DEADLINE_S")? {
            cfg.multi_replica.leader_renew_deadline_s = v;
        }
        if let Some(v) = read_u64(&lookup, "EFE_SHUTDOWN_DRAIN_RPC_MS")? {
            cfg.multi_replica.shutdown_drain_rpc_ms = v;
        }
        if let Some(v) = read_u64(&lookup, "EFE_SHUTDOWN_AUDIT_FLUSH_MS")? {
            cfg.multi_replica.shutdown_audit_flush_ms = v;
        }
        if let Some(v) = read_u64(&lookup, "EFE_SHUTDOWN_PG_CLOSE_MS")? {
            cfg.multi_replica.shutdown_pg_close_ms = v;
        }

        // --- Sprint D28 (Φ-D) — GPU runtime ---
        // **Audit fix K-A2** — flow GPU env vars through the config
        // layer so an operator setting them in the Helm ConfigMap
        // actually reaches `Server::build`.
        if let Some(v) = read_bool(&lookup, "EFE_RUNTIME_GPU_ENABLED")? {
            cfg.runtime.gpu_enabled = v;
        }
        if let Some(v) = read_u32(&lookup, "EFE_RUNTIME_GPU_DEVICE_ORDINAL")? {
            cfg.runtime.gpu_device_ordinal = v;
        }
        if let Some(v) = read_u32(&lookup, "EFE_RUNTIME_GPU_MEMORY_HWM_PERCENT")? {
            // Clamp to u8 range ; validate() catches out-of-range.
            cfg.runtime.gpu_memory_hwm_percent = (v.min(255)) as u8;
        }
        if let Some(v) = read_u32(&lookup, "EFE_RUNTIME_GPU_STREAMS")? {
            cfg.runtime.gpu_streams = v;
        }
        if let Some(v) = read_str(&lookup, "EFE_RUNTIME_GPU_SHARD_ORDINALS")? {
            cfg.runtime.gpu_shard_ordinals = v;
        }
        if let Some(v) = read_str(&lookup, "EFE_RUNTIME_GPU_FALLBACK_POLICY")? {
            cfg.runtime.gpu_fallback_policy = v;
        }
        if let Some(v) = read_bool(&lookup, "EFE_RUNTIME_GPU_CHECK_FINITE")? {
            cfg.runtime.gpu_check_finite = v;
        }

        // --- Phase-2 PR #11 : meta-swarm subsystem ---
        if let Some(v) = read_bool(&lookup, "EFE_META_SWARM_ENABLED")? {
            cfg.meta_swarm.enabled = v;
        }
        if let Some(v) = read_str(&lookup, "EFE_META_SWARM_CONTROLLER_ENDPOINT")? {
            cfg.meta_swarm.controller_endpoint = v;
        }
        if let Some(v) = read_u64(&lookup, "EFE_META_SWARM_CONNECT_TIMEOUT_MS")? {
            cfg.meta_swarm.connect_timeout = Duration::from_millis(v);
        }
        if let Some(v) = read_u64(&lookup, "EFE_META_SWARM_CALL_TIMEOUT_MS")? {
            cfg.meta_swarm.call_timeout = Duration::from_millis(v);
        }
        if let Some(v) = read_bool(&lookup, "EFE_META_SWARM_HEALTH_PROBE_AT_STARTUP")? {
            cfg.meta_swarm.health_probe_at_startup = v;
        }

        Ok(cfg)
    }

    /// Check cross-field invariants that cannot be expressed in types
    /// alone. Returns [`ConfigError::Invalid`] with a descriptive
    /// message on the first violation encountered.
    ///
    /// Invariants (M12 §8.3):
    ///
    /// * `listen_addr.port() != 0` — port 0 is reserved for tests that
    ///   construct a `ServerConfig` directly without going through
    ///   `validate()`.
    /// * `max_concurrent_streams ∈ [1, 65_535]`.
    /// * `request_timeout ∈ [100ms, 10min]`.
    /// * `auth.enabled == true ⇒ hs256_secrets.len() >= 1`.
    /// * `shutdown.force_abort_after >= shutdown.grace_period`.
    /// * `ratelimit.default_capacity >= 1` when enabled.
    /// * `ratelimit.default_refill_rate > 0.0` when enabled.
    /// * `idempotency.cache_capacity >= 1` when enabled.
    /// * `idempotency.ttl >= 100ms` when enabled.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.listen_addr.port() == 0 {
            return Err(ConfigError::invalid(
                "listen_addr",
                "port 0 is reserved for tests; set an explicit port (e.g. 50051)",
            ));
        }
        if !(1..=65_535).contains(&self.max_concurrent_streams) {
            return Err(ConfigError::invalid(
                "max_concurrent_streams",
                "must be in [1, 65_535]",
            ));
        }
        if self.request_timeout < Duration::from_millis(100)
            || self.request_timeout > Duration::from_secs(600)
        {
            return Err(ConfigError::invalid(
                "request_timeout",
                "must be in [100ms, 10min]",
            ));
        }
        if self.auth.enabled && self.auth.hs256_secrets.is_empty() {
            return Err(ConfigError::invalid(
                "auth.hs256_secrets",
                "auth is enabled but no HS256 secrets were loaded \
                 (check EFE_JWT_SECRETS_FILE)",
            ));
        }
        if self.shutdown.force_abort_after < self.shutdown.grace_period {
            return Err(ConfigError::invalid(
                "shutdown.force_abort_after",
                "must be >= shutdown.grace_period",
            ));
        }
        if self.ratelimit.enabled {
            if self.ratelimit.default_capacity == 0 {
                return Err(ConfigError::invalid(
                    "ratelimit.default_capacity",
                    "must be >= 1 when rate-limiting is enabled",
                ));
            }
            if !self.ratelimit.default_refill_rate.is_finite()
                || self.ratelimit.default_refill_rate <= 0.0
            {
                return Err(ConfigError::invalid(
                    "ratelimit.default_refill_rate",
                    "must be a positive finite float when rate-limiting is enabled",
                ));
            }
        }
        if self.idempotency.enabled {
            if self.idempotency.cache_capacity == 0 {
                return Err(ConfigError::invalid(
                    "idempotency.cache_capacity",
                    "must be >= 1 when idempotency is enabled",
                ));
            }
            if self.idempotency.ttl < Duration::from_millis(100) {
                return Err(ConfigError::invalid(
                    "idempotency.ttl",
                    "must be >= 100ms when idempotency is enabled",
                ));
            }
        }

        // --- Sprint D12-bis audit fix M2 — catalog policy quotas ---
        //
        // Without these checks an operator could set
        // `EFE_CATALOG_MAX_NAMES_PER_TENANT=0` and the binary would boot
        // happily ; only the *first* register call would fail with an
        // opaque "tenant exceeded names quota" error. We fail fast at
        // boot instead. The lower bound for `max_classes_per_catalog`
        // matches `policy::MIN_CLASSES_PER_CATALOG = 2` — anything below
        // makes every register call fail with InvalidArgument.
        if self.catalog.policy.max_names_per_tenant == 0 {
            return Err(ConfigError::invalid(
                "catalog.policy.max_names_per_tenant",
                "must be >= 1 (catalogs cannot be registered with quota 0)",
            ));
        }
        if self.catalog.policy.max_versions_per_name == 0 {
            return Err(ConfigError::invalid(
                "catalog.policy.max_versions_per_name",
                "must be >= 1 (no version could ever be created with quota 0)",
            ));
        }
        if self.catalog.policy.max_classes_per_catalog
            < crate::catalog::policy::MIN_CLASSES_PER_CATALOG
        {
            return Err(ConfigError::invalid(
                "catalog.policy.max_classes_per_catalog",
                "must be >= 2 (the registry rejects any catalog with < 2 classes)",
            ));
        }
        if self.catalog.policy.max_sentences_per_class == 0 {
            return Err(ConfigError::invalid(
                "catalog.policy.max_sentences_per_class",
                "must be >= 1 (every class needs at least one canonical sentence)",
            ));
        }

        // --- Sprint D14 — Postgres backend invariants ---
        //
        // These checks fire only when the operator has selected
        // `EFE_CATALOG_BACKEND=postgres`. For Memory mode, the
        // CatalogPostgresConfig defaults are unused and we skip
        // validation — the operator should not have to provide a
        // PG host just to run Memory mode.
        if matches!(self.catalog.backend, CatalogBackend::Postgres) {
            let pg = &self.catalog.postgres;
            if pg.host.is_empty() {
                return Err(ConfigError::invalid(
                    "catalog.postgres.host",
                    "must be non-empty when backend = postgres (set EFE_PG_HOST)",
                ));
            }
            if pg.port == 0 {
                return Err(ConfigError::invalid(
                    "catalog.postgres.port",
                    "must be in [1, 65535] (set EFE_PG_PORT, default 5432)",
                ));
            }
            if pg.database.is_empty() {
                return Err(ConfigError::invalid(
                    "catalog.postgres.database",
                    "must be non-empty when backend = postgres (set EFE_PG_DATABASE)",
                ));
            }
            if pg.user.is_empty() {
                return Err(ConfigError::invalid(
                    "catalog.postgres.user",
                    "must be non-empty when backend = postgres (set EFE_PG_USER)",
                ));
            }
            if pg.pool_max_connections == 0 {
                return Err(ConfigError::invalid(
                    "catalog.postgres.pool_max_connections",
                    "must be >= 1 (a pool with zero connections cannot serve requests)",
                ));
            }
            if pg.pool_min_connections > pg.pool_max_connections {
                return Err(ConfigError::invalid(
                    "catalog.postgres.pool_min_connections",
                    "must be <= pool_max_connections",
                ));
            }
            if pg.pool_acquire_timeout_ms < 100 {
                return Err(ConfigError::invalid(
                    "catalog.postgres.pool_acquire_timeout_ms",
                    "must be >= 100 (sub-100ms timeout is unrealistic for any non-local network)",
                ));
            }
            if pg.pool_max_lifetime_s < 60 {
                return Err(ConfigError::invalid(
                    "catalog.postgres.pool_max_lifetime_s",
                    "must be >= 60 (recycling more often than every minute is wasteful)",
                ));
            }
            if pg.pool_idle_timeout_s < 60 {
                return Err(ConfigError::invalid(
                    "catalog.postgres.pool_idle_timeout_s",
                    "must be >= 60 (reaping more often than every minute is wasteful)",
                ));
            }
        }

        // --- Sprint D15 — OIDC invariants ---
        //
        // OIDC is opt-in. When enabled, the issuers file must exist
        // and the cache/timeout/retry parameters must be sensible.
        // The actual content of the issuers file (validate URLs,
        // alg whitelist, tenant strategy etc.) is checked by
        // `auth::issuers::IssuerRegistry::from_file` at runtime
        // build — keeps `validate()` filesystem-free.
        if self.auth.oidc.enabled {
            // **Audit fix C2** : `oidc.enabled = true` is meaningless
            // without `auth.enabled = true` — the runtime would fall
            // through to `AuthGuard::disabled()` and accept every
            // request anonymously, which is the EXACT OPPOSITE of
            // what an operator turning on OIDC expects.
            if !self.auth.enabled {
                return Err(ConfigError::invalid(
                    "auth.oidc.enabled",
                    "EFE_OIDC_ENABLED=true requires EFE_AUTH_ENABLED=true ; \
                     enabling OIDC without auth would silently accept every \
                     request anonymously (audit fix D15-C2)",
                ));
            }
            if self.auth.oidc.issuers_file.is_empty() {
                return Err(ConfigError::invalid(
                    "auth.oidc.issuers_file",
                    "must be non-empty when oidc is enabled (set EFE_OIDC_ISSUERS_FILE)",
                ));
            }
            if !(60..=86_400).contains(&self.auth.oidc.jwks_ttl_s) {
                return Err(ConfigError::invalid(
                    "auth.oidc.jwks_ttl_s",
                    "must be in [60, 86400] (sub-minute wasteful, multi-day misses rotations)",
                ));
            }
            if self.auth.oidc.jwks_fetch_timeout_ms < 100 {
                return Err(ConfigError::invalid(
                    "auth.oidc.jwks_fetch_timeout_ms",
                    "must be >= 100 (sub-100ms is unrealistic for any non-local network)",
                ));
            }
            if self.auth.oidc.jwks_retry_max > 10 {
                return Err(ConfigError::invalid(
                    "auth.oidc.jwks_retry_max",
                    "must be <= 10 (more retries than that compound the IdP outage)",
                ));
            }
            // **Audit fix H7** — the backoff is doubled on each retry ;
            // an operator-set value of 60 000 ms with retry_max=10
            // would produce ~6 hours of waiting. Cap at 60 s initial.
            if self.auth.oidc.jwks_backoff_ms > 60_000 {
                return Err(ConfigError::invalid(
                    "auth.oidc.jwks_backoff_ms",
                    "must be <= 60000 (1 min) ; the backoff doubles per \
                     retry, so the tail latency compounds quickly",
                ));
            }
        }

        // --- Sprint D14-bis — audit signing invariants ---
        //
        // These checks fire only when the operator has explicitly
        // enabled HMAC signing. The default (`enabled=false`) is the
        // D12 `TracingAuditSink` and requires no extra configuration.
        if self.catalog.audit_signing.enabled {
            if !matches!(self.catalog.backend, CatalogBackend::Postgres) {
                return Err(ConfigError::invalid(
                    "catalog.audit_signing.enabled",
                    "EFE_AUDIT_SIGNING_ENABLED=true requires \
                     EFE_CATALOG_BACKEND=postgres (the HMAC sink uses \
                     the PG pool to persist `audit_events`)",
                ));
            }
            if self.catalog.audit_signing.keys_file.is_empty() {
                return Err(ConfigError::invalid(
                    "catalog.audit_signing.keys_file",
                    "must be non-empty when audit signing is enabled \
                     (set EFE_AUDIT_HMAC_SECRETS_FILE)",
                ));
            }
            let v = self.catalog.audit_signing.self_verify_every_s;
            if v != 0 && (v < 60 || v > 86_400) {
                return Err(ConfigError::invalid(
                    "catalog.audit_signing.self_verify_every_s",
                    "must be 0 (disabled) or in [60, 86400] seconds; \
                     sub-minute is wasteful, multi-day is a stale window",
                ));
            }
        }

        // --- Sprint D16 — Embedder bundling invariants ---
        //
        // Architecture § 14.2. Validation runs only when the operator
        // explicitly opts into the bundle path
        // (`EFE_EMBEDDER_BACKEND=bundle`) ; the legacy path imposes
        // no extra invariants beyond M12.
        if matches!(self.embedder.backend, EmbedderBackend::Bundle) {
            if self.embedder.bundle_path.trim().is_empty() {
                return Err(ConfigError::invalid(
                    "embedder.bundle_path",
                    "must be non-empty when EFE_EMBEDDER_BACKEND=bundle \
                     (set EFE_BUNDLE_PATH to the directory mounted by \
                     the K8s Secret/PVC, typically /etc/efe/bundles/active)",
                ));
            }
            // The trust store is only required when the operator wants
            // signatures verified. `Skip` mode is for Tier 1 dev only ;
            // the trust store is then optional but the validator still
            // refuses an empty path because it'd silently skip the
            // verify step at boot — confusing for ops debugging.
            if !matches!(self.embedder.require_signature, SignaturePolicy::Skip)
                && self.embedder.trust_store_path.trim().is_empty()
            {
                return Err(ConfigError::invalid(
                    "embedder.trust_store_path",
                    "must be non-empty when require_signature ≠ skip \
                     (set EFE_BUNDLE_TRUST_STORE to the pubkeys.json \
                     file mounted by the K8s Secret)",
                ));
            }
            if !(1..=600).contains(&self.embedder.load_timeout_s) {
                return Err(ConfigError::invalid(
                    "embedder.load_timeout_s",
                    "must be in [1, 600] seconds (sub-second is unrealistic ; \
                     >10 min hides slow IO bugs as silent boot stalls)",
                ));
            }
            if self.embedder.encode_timeout_ms < 10 {
                return Err(ConfigError::invalid(
                    "embedder.encode_timeout_ms",
                    "must be >= 10 ms (sub-10ms encode is unreachable for \
                     any production-quality embedder, even GPU-accelerated)",
                ));
            }
        }

        // --- Sprint D18 invariants ---
        // Architecture § 8.2 : `renew_deadline < lease_duration` (else
        // we'd renew AFTER the lease expired, which is impossible).
        if self.multi_replica.leader_election_enabled
            && self.multi_replica.leader_renew_deadline_s
                >= self.multi_replica.leader_lease_duration_s
        {
            return Err(ConfigError::invalid(
                "multi_replica.leader_renew_deadline_s",
                "must be strictly less than leader_lease_duration_s \
                 (cf. D18 architecture § 8.2)",
            ));
        }
        // `replicas_min <= replicas_max`.
        if self.multi_replica.replicas_min > self.multi_replica.replicas_max {
            return Err(ConfigError::invalid(
                "multi_replica.replicas_min",
                "must be <= replicas_max",
            ));
        }
        // Each phase timeout > 0.
        if self.multi_replica.shutdown_drain_rpc_ms == 0 {
            return Err(ConfigError::invalid(
                "multi_replica.shutdown_drain_rpc_ms",
                "must be > 0",
            ));
        }
        if self.multi_replica.shutdown_audit_flush_ms == 0 {
            return Err(ConfigError::invalid(
                "multi_replica.shutdown_audit_flush_ms",
                "must be > 0",
            ));
        }
        if self.multi_replica.shutdown_pg_close_ms == 0 {
            return Err(ConfigError::invalid(
                "multi_replica.shutdown_pg_close_ms",
                "must be > 0",
            ));
        }

        // --- Sprint D28 (Φ-D) — GPU runtime invariants ---
        if !(1..=95).contains(&self.runtime.gpu_memory_hwm_percent) {
            return Err(ConfigError::invalid(
                "runtime.gpu_memory_hwm_percent",
                "must be in [1, 95]",
            ));
        }
        if self.runtime.gpu_enabled && self.runtime.gpu_streams == 0 {
            return Err(ConfigError::invalid(
                "runtime.gpu_streams",
                "must be > 0 when gpu_enabled is true",
            ));
        }
        if !matches!(
            self.runtime.gpu_fallback_policy.as_str(),
            "strict" | "cpu_on_error"
        ) {
            return Err(ConfigError::invalid(
                "runtime.gpu_fallback_policy",
                "must be one of `strict` or `cpu_on_error`",
            ));
        }

        // --- Phase-2 PR #11 : meta-swarm subsystem ---
        if self.meta_swarm.enabled {
            // Parse the endpoint eagerly so a typo doesn't survive
            // boot.  We discard the parsed Endpoint here ; the actual
            // gRPC client is built later in `MetaSwarmClient::connect`.
            tonic::transport::Endpoint::from_shared(self.meta_swarm.controller_endpoint.clone())
                .map_err(|e| {
                    ConfigError::invalid(
                        "meta_swarm.controller_endpoint",
                        format!(
                            "EFE_META_SWARM_CONTROLLER_ENDPOINT = {:?} : {e}",
                            self.meta_swarm.controller_endpoint,
                        ),
                    )
                })?;
            if self.meta_swarm.connect_timeout.as_millis() == 0 {
                return Err(ConfigError::invalid(
                    "meta_swarm.connect_timeout",
                    "EFE_META_SWARM_CONNECT_TIMEOUT_MS must be > 0 when enabled",
                ));
            }
            if self.meta_swarm.call_timeout.as_millis() == 0 {
                return Err(ConfigError::invalid(
                    "meta_swarm.call_timeout",
                    "EFE_META_SWARM_CALL_TIMEOUT_MS must be > 0 when enabled",
                ));
            }
        }

        Ok(())
    }
}

/// JWT auth config.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AuthConfig {
    /// Global auth toggle.
    pub enabled: bool,
    /// Expected JWT `aud`.
    pub jwt_audience: String,
    /// Expected JWT `iss` (optional).
    pub jwt_issuer: Option<String>,
    /// Dev escape: allow the unary health endpoint without auth (always true).
    pub allow_insecure_dev: bool,
    /// HS256 shared secrets used to validate tokens. Multiple keys are
    /// accepted to support zero-downtime rotation; the most recent key
    /// is kept first.
    pub hs256_secrets: Vec<Vec<u8>>,
    /// Sprint **D15** — OIDC backend configuration. When `enabled`,
    /// EFE accepts JWT signed by the configured issuers (RS256/ES256
    /// asymmetric). HS256 remains supported in parallel for the
    /// gradual-migration path (cf. § 9 of the architecture doc).
    pub oidc: OidcConfig,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            jwt_audience: "efe.v1".into(),
            jwt_issuer: None,
            allow_insecure_dev: true,
            hs256_secrets: Vec::new(),
            oidc: OidcConfig::default(),
        }
    }
}

// =====================================================================
// Sprint D15 — OIDC integration
// =====================================================================

/// Sprint **D15** — OIDC backend configuration.
///
/// Off by default. Enabling this turns EFE into an OAuth 2.0 resource
/// server (RFC 6749 § 1.1) that validates JWT access tokens issued by
/// any OIDC-compliant IdP (Auth0, Okta, Keycloak, Cognito, Azure AD,
/// Google Workspace).
///
/// Required for **multi-tenant production deployments** where each
/// tenant brings its own IdP (separation of signing role per
/// SOC 2 CC6.1 / NIST 800-53 IA-2). HS256 (the M12 path) shares the
/// HMAC key with EFE and cannot satisfy that control.
///
/// **Validation invariants** (enforced by [`ServerConfig::validate`]
/// when `enabled = true`) :
///
/// * `issuers_file` non-empty.
/// * `jwks_ttl_s ∈ [60, 86 400]` (sub-minute is wasteful, multi-day
///   misses rotations).
/// * `jwks_fetch_timeout_ms ≥ 100` (sub-100ms is unrealistic for
///   any non-local network).
/// * `jwks_retry_max ∈ [0, 10]`.
/// * `grace_stale_s ≥ 0` (0 disables stale-while-revalidate).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OidcConfig {
    /// Master switch.
    pub enabled: bool,
    /// Path to the issuers descriptor file. JSON array of
    /// [`crate::auth::issuers::IssuerConfig`] entries (cf. § 13.2 of
    /// the architecture doc for the schema).
    pub issuers_file: String,
    /// JWKS cache TTL, in seconds. Default 300 (5 min).
    pub jwks_ttl_s: u64,
    /// Per-fetch HTTP timeout, in milliseconds. Default 5000.
    pub jwks_fetch_timeout_ms: u64,
    /// Maximum retry attempts on JWKS fetch failure. Default 3.
    pub jwks_retry_max: u32,
    /// Initial backoff, in milliseconds. Doubles per retry.
    pub jwks_backoff_ms: u64,
    /// Grace stale period, in seconds. When a JWKS fetch fails
    /// AND the cache is older than `jwks_ttl_s` but younger than
    /// `jwks_ttl_s + grace_stale_s`, we keep using the cache and
    /// log a warning. Past the grace window, we hard-reject any
    /// new tokens with `Unauthenticated`.
    pub grace_stale_s: u64,
}

impl Default for OidcConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            // **Audit fix C3 + M7** : the path MUST match the Helm
            // chart's mount path (`deploy/helm/efe/values.yaml::oidc.issuersFile`)
            // and the `efe-onboard preflight` CLI default. The pre-audit
            // value `/etc/efe/oidc/issuers.json` drifted in three places
            // and produced silent "issuers file not found" errors when
            // the operator launched the binary outside Helm. After M7
            // the canonical mount path moved to `/etc/efe/oidc/` to
            // avoid shadowing other secret mounts under `/etc/efe/`.
            issuers_file: "/etc/efe/oidc/oidc-issuers.json".to_string(),
            jwks_ttl_s: 300,
            jwks_fetch_timeout_ms: 5000,
            jwks_retry_max: 3,
            jwks_backoff_ms: 1000,
            grace_stale_s: 3600,
        }
    }
}

/// Rate-limit config.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RateLimitConfig {
    /// Global toggle.
    pub enabled: bool,
    /// Bucket capacity (burst tolerance).
    pub default_capacity: u32,
    /// Tokens added per second.
    pub default_refill_rate: f32,
    /// Bucket capacity for unauthenticated callers.
    pub anonymous_capacity: u32,
    /// Refill for unauthenticated callers.
    pub anonymous_refill_rate: f32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            default_capacity: 200,
            default_refill_rate: 100.0,
            anonymous_capacity: 50,
            anonymous_refill_rate: 10.0,
        }
    }
}

/// Idempotency cache config.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct IdempotencyConfig {
    /// Global toggle.
    pub enabled: bool,
    /// Maximum cached entries.
    pub cache_capacity: usize,
    /// Entry TTL.
    pub ttl: Duration,
}

impl Default for IdempotencyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cache_capacity: 10_000,
            ttl: Duration::from_secs(10),
        }
    }
}

/// Graceful shutdown config.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ShutdownConfig {
    /// Max time to drain inflight calls after SIGTERM.
    pub grace_period: Duration,
    /// Hard abort deadline after SIGTERM (grace_period + N).
    pub force_abort_after: Duration,
}

impl Default for ShutdownConfig {
    fn default() -> Self {
        Self {
            grace_period: Duration::from_secs(30),
            force_abort_after: Duration::from_secs(45),
        }
    }
}

// =====================================================================
// Errors
// =====================================================================

/// Configuration parsing / validation error.
///
/// The error variants are intentionally fine-grained so that operators
/// reading the logs can fix the offending variable without having to
/// re-read the entire configuration documentation.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// An environment variable contained an unparseable value.
    #[error("invalid value for {var}: {msg}")]
    Parse {
        /// Variable name (e.g. `EFE_LISTEN_ADDR`).
        var: &'static str,
        /// Human-readable parse error.
        msg: String,
    },
    /// A cross-field invariant was violated.
    #[error("configuration invariant violated ({field}): {msg}")]
    Invalid {
        /// Dotted field path (e.g. `auth.hs256_secrets`).
        field: &'static str,
        /// Human-readable explanation.
        msg: String,
    },
    /// The HS256 secrets file could not be read.
    #[error("could not read secrets file {path}: {source}")]
    SecretsIo {
        /// Path that was attempted.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// The HS256 secrets file was empty or only contained whitespace.
    #[error("secrets file {path} is empty; expected at least one base64 key")]
    SecretsEmpty {
        /// Path that was attempted.
        path: PathBuf,
    },
}

impl ConfigError {
    fn parse(var: &'static str, msg: impl Into<String>) -> Self {
        Self::Parse {
            var,
            msg: msg.into(),
        }
    }
    fn invalid(field: &'static str, msg: impl Into<String>) -> Self {
        Self::Invalid {
            field,
            msg: msg.into(),
        }
    }
}

// =====================================================================
// Parsing helpers
// =====================================================================
//
// These are private helpers that turn `Result<String, VarError>` into
// `Result<Option<T>, ConfigError>`, which is the shape the calling
// code wants: `None` when the variable is not set, `Some(t)` when it
// is set and parses, `Err(...)` when it's set but malformed.

fn read_str<F>(f: &F, var: &'static str) -> Result<Option<String>, ConfigError>
where
    F: Fn(&'static str) -> Result<String, std::env::VarError>,
{
    match f(var) {
        Ok(v) if v.is_empty() => Ok(None), // treat "" as unset (k8s env defaults)
        Ok(v) => Ok(Some(v)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(e) => Err(ConfigError::parse(var, format!("{e}"))),
    }
}

fn read_bool<F>(f: &F, var: &'static str) -> Result<Option<bool>, ConfigError>
where
    F: Fn(&'static str) -> Result<String, std::env::VarError>,
{
    match read_str(f, var)? {
        None => Ok(None),
        Some(s) => Ok(Some(parse_bool(&s).ok_or_else(|| {
            ConfigError::parse(
                var,
                format!("expected one of {{1,true,yes,on,0,false,no,off}}, got {s:?}"),
            )
        })?)),
    }
}

fn read_u64<F>(f: &F, var: &'static str) -> Result<Option<u64>, ConfigError>
where
    F: Fn(&'static str) -> Result<String, std::env::VarError>,
{
    match read_str(f, var)? {
        None => Ok(None),
        Some(s) => s
            .parse::<u64>()
            .map(Some)
            .map_err(|e| ConfigError::parse(var, format!("{e}"))),
    }
}

fn read_u32<F>(f: &F, var: &'static str) -> Result<Option<u32>, ConfigError>
where
    F: Fn(&'static str) -> Result<String, std::env::VarError>,
{
    match read_str(f, var)? {
        None => Ok(None),
        Some(s) => s
            .parse::<u32>()
            .map(Some)
            .map_err(|e| ConfigError::parse(var, format!("{e}"))),
    }
}

fn read_u16<F>(f: &F, var: &'static str) -> Result<Option<u16>, ConfigError>
where
    F: Fn(&'static str) -> Result<String, std::env::VarError>,
{
    match read_str(f, var)? {
        None => Ok(None),
        Some(s) => s
            .parse::<u16>()
            .map(Some)
            .map_err(|e| ConfigError::parse(var, format!("{e}"))),
    }
}

fn read_usize<F>(f: &F, var: &'static str) -> Result<Option<usize>, ConfigError>
where
    F: Fn(&'static str) -> Result<String, std::env::VarError>,
{
    match read_str(f, var)? {
        None => Ok(None),
        Some(s) => s
            .parse::<usize>()
            .map(Some)
            .map_err(|e| ConfigError::parse(var, format!("{e}"))),
    }
}

fn read_f32<F>(f: &F, var: &'static str) -> Result<Option<f32>, ConfigError>
where
    F: Fn(&'static str) -> Result<String, std::env::VarError>,
{
    match read_str(f, var)? {
        None => Ok(None),
        Some(s) => s
            .parse::<f32>()
            .map(Some)
            .map_err(|e| ConfigError::parse(var, format!("{e}"))),
    }
}

/// Parse a boolean following the permissive but explicit set described
/// in M12 §8.2. Unrecognised inputs return `None` so the caller can
/// produce a descriptive error with the offending value.
fn parse_bool(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// Load HS256 verification keys from a file.
///
/// The file format is one base64-url-encoded key per line; blank
/// lines and `#`-prefixed comment lines are ignored. The order is
/// preserved — the first non-blank line is the **active signing key**
/// and the remainder are verification-only keys kept during rotation
/// (M12 §15.4).
fn load_hs256_secrets(path: &Path) -> Result<Vec<Vec<u8>>, ConfigError> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    let raw = std::fs::read_to_string(path).map_err(|e| ConfigError::SecretsIo {
        path: path.to_path_buf(),
        source: e,
    })?;
    let mut keys: Vec<Vec<u8>> = Vec::new();
    for (lineno, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let bytes = URL_SAFE_NO_PAD.decode(trimmed).map_err(|e| {
            // We deliberately do NOT echo the offending key material
            // in the error — only its line number — so logs never
            // leak even a malformed secret.
            ConfigError::parse(
                "EFE_JWT_SECRETS_FILE",
                format!("line {} is not valid base64-url: {e}", lineno + 1),
            )
        })?;
        keys.push(bytes);
    }
    if keys.is_empty() {
        return Err(ConfigError::SecretsEmpty {
            path: path.to_path_buf(),
        });
    }
    Ok(keys)
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
// Several validation tests deliberately construct a default-then-mutate
// `ServerConfig` to isolate a single invariant violation (e.g. a bad
// port or a too-small timeout). `#[non_exhaustive]` forbids the
// struct-update shorthand from outside the crate anyway, so suppressing
// this one lint inside the test module is the cleanest path.
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use std::env::VarError;

    /// Build an env lookup closure from a `&[(&str, &str)]` slice.
    /// Keeps tests hermetic (no `std::env::set_var`).
    fn env(
        entries: &'static [(&'static str, &'static str)],
    ) -> impl Fn(&'static str) -> Result<String, VarError> {
        move |var| {
            entries
                .iter()
                .find(|(k, _)| *k == var)
                .map(|(_, v)| v.to_string())
                .ok_or(VarError::NotPresent)
        }
    }

    #[test]
    fn default_config_builds() {
        let cfg = ServerConfig::default();
        assert_eq!(cfg.max_concurrent_streams, 1024);
        assert!(cfg.enable_admin);
        assert_eq!(cfg.listen_addr.port(), 50051);
    }

    #[test]
    fn default_config_validates() {
        let cfg = ServerConfig::default();
        cfg.validate().expect("default config must be valid");
    }

    #[test]
    fn config_serde_roundtrip() {
        let cfg = ServerConfig::default();
        let s = serde_json::to_string(&cfg).unwrap();
        let back: ServerConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(back.max_concurrent_streams, cfg.max_concurrent_streams);
    }

    #[test]
    fn from_env_with_no_vars_equals_default() {
        let cfg = ServerConfig::from_env_with(env(&[])).expect("from_env ok");
        let d = ServerConfig::default();
        assert_eq!(cfg.listen_addr, d.listen_addr);
        assert_eq!(cfg.max_concurrent_streams, d.max_concurrent_streams);
        assert_eq!(cfg.request_timeout, d.request_timeout);
        assert_eq!(cfg.enable_admin, d.enable_admin);
    }

    #[test]
    fn from_env_parses_basic_overrides() {
        let cfg = ServerConfig::from_env_with(env(&[
            ("EFE_LISTEN_ADDR", "0.0.0.0:60001"),
            ("EFE_REQUEST_TIMEOUT_MS", "15000"),
            ("EFE_MAX_CONCURRENT_STREAMS", "256"),
            ("EFE_ENABLE_ADMIN", "false"),
        ]))
        .expect("from_env ok");
        assert_eq!(cfg.listen_addr.port(), 60001);
        assert_eq!(cfg.request_timeout, Duration::from_secs(15));
        assert_eq!(cfg.max_concurrent_streams, 256);
        assert!(!cfg.enable_admin);
    }

    #[test]
    fn from_env_rejects_malformed_int() {
        let err = ServerConfig::from_env_with(env(&[("EFE_REQUEST_TIMEOUT_MS", "not-a-number")]))
            .expect_err("must fail on bogus int");
        match err {
            ConfigError::Parse { var, .. } => assert_eq!(var, "EFE_REQUEST_TIMEOUT_MS"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn from_env_rejects_malformed_bool() {
        let err = ServerConfig::from_env_with(env(&[("EFE_AUTH_ENABLED", "peut-etre")]))
            .expect_err("must fail on bogus bool");
        assert!(matches!(
            err,
            ConfigError::Parse {
                var: "EFE_AUTH_ENABLED",
                ..
            }
        ));
    }

    #[test]
    fn from_env_accepts_permissive_bools() {
        for (raw, expected) in [
            ("1", true),
            ("true", true),
            ("TRUE", true),
            ("yes", true),
            ("on", true),
            ("0", false),
            ("false", false),
            ("FALSE", false),
            ("no", false),
            ("off", false),
        ] {
            let entries: [(&'static str, &'static str); 1] = [("EFE_ENABLE_ADMIN", raw)];
            let cfg = ServerConfig::from_env_with(move |v| {
                if v == "EFE_ENABLE_ADMIN" {
                    Ok(raw.to_string())
                } else {
                    Err(VarError::NotPresent)
                }
            })
            .expect("from_env ok");
            assert_eq!(cfg.enable_admin, expected, "raw={raw}");
            let _ = entries; // silence unused
        }
    }

    #[test]
    fn empty_string_is_treated_as_unset() {
        // Kubernetes ConfigMap often materialises absent keys as "".
        // We want default behaviour in that case, not an error.
        let cfg = ServerConfig::from_env_with(env(&[("EFE_JWT_ISSUER", "")])).expect("from_env ok");
        assert_eq!(cfg.auth.jwt_issuer, None);
    }

    #[test]
    fn shutdown_drain_ms_maps_to_grace_period() {
        let cfg = ServerConfig::from_env_with(env(&[("EFE_SHUTDOWN_DRAIN_MS", "7000")]))
            .expect("from_env ok");
        assert_eq!(cfg.shutdown.grace_period, Duration::from_secs(7));
    }

    #[test]
    fn shutdown_grace_ms_alias_is_honoured() {
        let cfg = ServerConfig::from_env_with(env(&[("EFE_SHUTDOWN_GRACE_MS", "9000")]))
            .expect("from_env ok");
        assert_eq!(cfg.shutdown.grace_period, Duration::from_secs(9));
    }

    #[test]
    fn validate_rejects_port_zero() {
        let mut cfg = ServerConfig::default();
        cfg.listen_addr = "127.0.0.1:0".parse().unwrap();
        let err = cfg.validate().expect_err("port 0 should be rejected");
        assert!(matches!(
            err,
            ConfigError::Invalid {
                field: "listen_addr",
                ..
            }
        ));
    }

    #[test]
    fn validate_rejects_force_abort_less_than_grace() {
        let mut cfg = ServerConfig::default();
        cfg.shutdown.grace_period = Duration::from_secs(30);
        cfg.shutdown.force_abort_after = Duration::from_secs(10);
        let err = cfg.validate().expect_err("abort < grace must fail");
        assert!(matches!(
            err,
            ConfigError::Invalid {
                field: "shutdown.force_abort_after",
                ..
            }
        ));
    }

    #[test]
    fn validate_rejects_auth_without_secrets() {
        let mut cfg = ServerConfig::default();
        cfg.auth.enabled = true; // but hs256_secrets stays empty
        let err = cfg.validate().expect_err("auth w/o secrets must fail");
        assert!(matches!(
            err,
            ConfigError::Invalid {
                field: "auth.hs256_secrets",
                ..
            }
        ));
    }

    #[test]
    fn validate_accepts_auth_with_secrets() {
        let mut cfg = ServerConfig::default();
        cfg.auth.enabled = true;
        cfg.auth.hs256_secrets = vec![b"some-key".to_vec()];
        cfg.validate().expect("auth with secrets must validate");
    }

    #[test]
    fn validate_rejects_request_timeout_below_100ms() {
        let mut cfg = ServerConfig::default();
        cfg.request_timeout = Duration::from_millis(50);
        let err = cfg.validate().expect_err("< 100ms must fail");
        assert!(matches!(
            err,
            ConfigError::Invalid {
                field: "request_timeout",
                ..
            }
        ));
    }

    #[test]
    fn validate_rejects_ratelimit_zero_capacity_when_enabled() {
        let mut cfg = ServerConfig::default();
        cfg.ratelimit.enabled = true;
        cfg.ratelimit.default_capacity = 0;
        let err = cfg.validate().expect_err("capacity=0 must fail");
        assert!(matches!(
            err,
            ConfigError::Invalid {
                field: "ratelimit.default_capacity",
                ..
            }
        ));
    }

    #[test]
    fn validate_allows_ratelimit_zero_capacity_when_disabled() {
        // Disabled rate-limit: misconfigured fields are irrelevant
        // because the layer is not activated. We keep the config
        // structurally sound, not semantically opinionated.
        let mut cfg = ServerConfig::default();
        cfg.ratelimit.enabled = false;
        cfg.ratelimit.default_capacity = 0;
        cfg.validate().expect("disabled ratelimit must validate");
    }

    #[test]
    fn validate_rejects_idempotency_ttl_too_low() {
        let mut cfg = ServerConfig::default();
        cfg.idempotency.enabled = true;
        cfg.idempotency.ttl = Duration::from_millis(50);
        let err = cfg.validate().expect_err("TTL < 100ms must fail");
        assert!(matches!(
            err,
            ConfigError::Invalid {
                field: "idempotency.ttl",
                ..
            }
        ));
    }

    #[test]
    fn load_hs256_secrets_reads_file_ok() {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let k1 = URL_SAFE_NO_PAD.encode(b"primary-key-material-abc");
        let k2 = URL_SAFE_NO_PAD.encode(b"secondary-verification-key");
        std::fs::write(tmp.path(), format!("# comment\n{k1}\n\n{k2}\n")).unwrap();
        let keys = load_hs256_secrets(tmp.path()).expect("ok");
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0], b"primary-key-material-abc");
        assert_eq!(keys[1], b"secondary-verification-key");
    }

    #[test]
    fn load_hs256_secrets_rejects_missing_file() {
        let err = load_hs256_secrets(Path::new("/nonexistent/efe/secrets")).expect_err("must fail");
        assert!(matches!(err, ConfigError::SecretsIo { .. }));
    }

    #[test]
    fn load_hs256_secrets_rejects_empty_file() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "# only a comment\n\n").unwrap();
        let err = load_hs256_secrets(tmp.path()).expect_err("empty must fail");
        assert!(matches!(err, ConfigError::SecretsEmpty { .. }));
    }

    #[test]
    fn load_hs256_secrets_rejects_non_base64_line_without_leaking_material() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "not%base64\n").unwrap();
        let err = load_hs256_secrets(tmp.path()).expect_err("malformed must fail");
        // Critical security invariant: the offending key MUST NOT
        // appear in the error message.
        let msg = format!("{err}");
        assert!(!msg.contains("not%base64"), "leaked secret: {msg}");
    }

    // =================================================================
    // Phase-2 PR #11 — MetaSwarmConfig
    // =================================================================

    #[test]
    fn meta_swarm_defaults_are_off() {
        let cfg = MetaSwarmConfig::default();
        assert!(!cfg.enabled, "default MUST be off (PR #11 AD2)");
        assert_eq!(cfg.controller_endpoint, "http://127.0.0.1:50080");
        assert_eq!(cfg.connect_timeout, Duration::from_secs(5));
        assert_eq!(cfg.call_timeout, Duration::from_secs(20));
        assert!(cfg.health_probe_at_startup);
    }

    #[test]
    fn meta_swarm_from_env_no_vars_uses_defaults() {
        let cfg = ServerConfig::from_env_with(env(&[])).expect("from_env ok");
        let d = MetaSwarmConfig::default();
        assert_eq!(cfg.meta_swarm.enabled, d.enabled);
        assert_eq!(cfg.meta_swarm.controller_endpoint, d.controller_endpoint);
        assert_eq!(cfg.meta_swarm.connect_timeout, d.connect_timeout);
        assert_eq!(cfg.meta_swarm.call_timeout, d.call_timeout);
        assert_eq!(
            cfg.meta_swarm.health_probe_at_startup,
            d.health_probe_at_startup,
        );
    }

    #[test]
    fn meta_swarm_from_env_parses_all_vars() {
        let cfg = ServerConfig::from_env_with(env(&[
            ("EFE_META_SWARM_ENABLED", "true"),
            (
                "EFE_META_SWARM_CONTROLLER_ENDPOINT",
                "http://10.0.0.5:50080",
            ),
            ("EFE_META_SWARM_CONNECT_TIMEOUT_MS", "3000"),
            ("EFE_META_SWARM_CALL_TIMEOUT_MS", "12500"),
            ("EFE_META_SWARM_HEALTH_PROBE_AT_STARTUP", "false"),
        ]))
        .expect("from_env ok");
        assert!(cfg.meta_swarm.enabled);
        assert_eq!(cfg.meta_swarm.controller_endpoint, "http://10.0.0.5:50080");
        assert_eq!(cfg.meta_swarm.connect_timeout, Duration::from_millis(3000));
        assert_eq!(cfg.meta_swarm.call_timeout, Duration::from_millis(12500));
        assert!(!cfg.meta_swarm.health_probe_at_startup);
    }

    #[test]
    fn meta_swarm_lenient_bool_parsing() {
        for v in ["true", "TRUE", "1", "yes", "Yes", "on"] {
            let cfg = ServerConfig::from_env_with(env(Box::leak(Box::new([(
                "EFE_META_SWARM_ENABLED",
                v,
            )]))))
            .unwrap();
            assert!(cfg.meta_swarm.enabled, "{v:?} should parse as true");
        }
        for v in ["false", "0", "no", "off", "OFF"] {
            let cfg = ServerConfig::from_env_with(env(Box::leak(Box::new([(
                "EFE_META_SWARM_ENABLED",
                v,
            )]))))
            .unwrap();
            assert!(!cfg.meta_swarm.enabled, "{v:?} should parse as false");
        }
    }

    #[test]
    fn meta_swarm_validate_rejects_malformed_endpoint_when_enabled() {
        let cfg = ServerConfig::from_env_with(env(&[
            ("EFE_META_SWARM_ENABLED", "true"),
            ("EFE_META_SWARM_CONTROLLER_ENDPOINT", "not a valid uri"),
        ]))
        .expect("from_env succeeds (lazy parse)");
        let err = cfg
            .validate()
            .expect_err("malformed endpoint must be rejected");
        assert!(
            matches!(err, ConfigError::Invalid { field, .. } if field == "meta_swarm.controller_endpoint"),
            "{err:?}",
        );
    }

    #[test]
    fn meta_swarm_validate_accepts_malformed_endpoint_when_disabled() {
        // The validator must be lazy : a bad endpoint with
        // ENABLED=false costs nothing.
        let cfg = ServerConfig::from_env_with(env(&[
            ("EFE_META_SWARM_ENABLED", "false"),
            ("EFE_META_SWARM_CONTROLLER_ENDPOINT", "obvious garbage"),
        ]))
        .expect("from_env ok");
        cfg.validate()
            .expect("validate must skip endpoint check when disabled");
    }

    #[test]
    fn meta_swarm_validate_rejects_zero_connect_timeout_when_enabled() {
        let cfg = ServerConfig::from_env_with(env(&[
            ("EFE_META_SWARM_ENABLED", "true"),
            ("EFE_META_SWARM_CONNECT_TIMEOUT_MS", "0"),
        ]))
        .expect("from_env ok");
        let err = cfg.validate().expect_err("zero connect timeout must fail");
        assert!(
            matches!(err, ConfigError::Invalid { field, .. } if field == "meta_swarm.connect_timeout"),
            "{err:?}",
        );
    }

    #[test]
    fn meta_swarm_validate_rejects_zero_call_timeout_when_enabled() {
        let cfg = ServerConfig::from_env_with(env(&[
            ("EFE_META_SWARM_ENABLED", "true"),
            ("EFE_META_SWARM_CALL_TIMEOUT_MS", "0"),
        ]))
        .expect("from_env ok");
        let err = cfg.validate().expect_err("zero call timeout must fail");
        assert!(
            matches!(err, ConfigError::Invalid { field, .. } if field == "meta_swarm.call_timeout"),
            "{err:?}",
        );
    }
}
