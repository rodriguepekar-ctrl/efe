//! Server bootstrap: assemble services, interceptors, and `tonic`
//! transport into a runnable unit.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tonic::transport::Server as TonicServer;

use crate::auth::{
    load_issuers_file, AuthGuard, IssuerRegistry, JwksCache, LoadParams, OidcValidator, TokenStore,
};
use crate::catalog::{
    audit_hmac::HmacChainedAuditSink, audit_keys::load_keys_file, AuditEmitter,
    AuditEmitter as CatalogAuditEmitter, AuditSink, CatalogPolicy, CatalogRegistry,
    InMemoryRegistry, PostgresCatalogRegistry,
};
use crate::config::{CatalogBackend, ServerConfig};
use crate::idempotency::IdempotencyCache;
use crate::multi_replica::{
    IdempotencyStore, LeaderLock, MockIdempotencyStore, MockLeaderLock, MockRateLimitStore,
    RateLimitStore, ShutdownSequence, ShutdownTimeouts,
};
use crate::proto::v1::swarm_service_server::SwarmServiceServer;
use crate::proto::v1_admin::admin_service_server::AdminServiceServer;
use crate::proto::v1_catalog::catalog_service_server::CatalogServiceServer;
// Sprint **D34** + **D37** — Mode-2 and Mode-3 service stubs.
use crate::proto::v1_mode2::mode2_service_server::Mode2ServiceServer;
use crate::proto::v1_mode3::mode3_service_server::Mode3ServiceServer;
use crate::ratelimit::BucketRegistry;
use crate::services::{
    admin::AdminServiceImpl, catalog::CatalogServiceImpl, swarm::SwarmServiceImpl,
    DynExecuteBackend, Mode2ServiceImpl, Mode3ServiceImpl,
};
use crate::shutdown::Shutdown;
use efe_core::alignment::AlignmentRegistry;

/// Builder for [`Server`].
#[derive(Default)]
pub struct ServerBuilder {
    config: Option<ServerConfig>,
    backend: Option<DynExecuteBackend>,
    registry: Option<Arc<dyn AlignmentRegistry>>,
    catalog_registry: Option<Arc<dyn CatalogRegistry>>,
    token_store: Option<Arc<TokenStore>>,
}

impl ServerBuilder {
    /// Supply the config (otherwise defaults).
    #[must_use]
    pub fn config(mut self, cfg: ServerConfig) -> Self {
        self.config = Some(cfg);
        self
    }

    /// Supply the execute backend. Required.
    #[must_use]
    pub fn backend(mut self, backend: DynExecuteBackend) -> Self {
        self.backend = Some(backend);
        self
    }

    /// Supply the alignment registry (enables the admin service).
    #[must_use]
    pub fn registry(mut self, registry: Arc<dyn AlignmentRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Sprint **D12-bis** — supply a custom `CatalogRegistry` (e.g.
    /// pre-populated for tests). When omitted, [`Self::build`]
    /// auto-instantiates one from `config.catalog.backend` :
    ///
    ///   * `Memory` (default) → [`InMemoryRegistry::new()`].
    ///   * `Postgres` → returns `Err` (D14 wire-up pending).
    #[must_use]
    pub fn catalog_registry(mut self, registry: Arc<dyn CatalogRegistry>) -> Self {
        self.catalog_registry = Some(registry);
        self
    }

    /// Supply a preconstructed token store.
    #[must_use]
    pub fn token_store(mut self, ts: Arc<TokenStore>) -> Self {
        self.token_store = Some(ts);
        self
    }

    /// Build the server. Fails if required pieces are missing or if the
    /// config cannot be validated.
    ///
    /// **Sprint D14** : `build` is now async because constructing the
    /// Postgres-backed [`CatalogRegistry`] involves a network connect.
    /// For Memory mode the await is a no-op (the future resolves
    /// synchronously inside the call). All callsites must be in a
    /// tokio context — they were already, since `serve` is async.
    pub async fn build(self) -> anyhow::Result<Server> {
        let config = self.config.unwrap_or_default();
        let backend = self
            .backend
            .ok_or_else(|| anyhow::anyhow!("ServerBuilder.backend is required"))?;

        // **Audit fix D15-H5** — register D15 instruments eagerly so
        // the `D15-onboarding` Grafana dashboard panels render zero-
        // valued series at boot (instead of "no data"). Cost is ~52
        // empty Prometheus series — negligible. Without this, the
        // panels are blank until the first OIDC token arrives, which
        // misleads operators into thinking metrics aren't wired.
        efe_telemetry::metrics::register_all_d15();

        // **Audit fix D16-C1** — same eager-register pattern for D16.
        // Cost is ~3 150 zero-valued series at maximum scale (5
        // bundles × 100 tenants per § 13.2). Negligible vs the D13
        // 50 000-series budget.
        efe_telemetry::metrics::register_all_d16();

        // **Audit fix D18-A3** — register the 9 D18 instruments
        // eagerly so the Grafana D18 dashboard panels render
        // zero-valued series at boot rather than "no data". Cost
        // is ~5 + (200 tenants × 2 outcomes) ≈ 405 series at the
        // maximum spec ; negligible vs the D13 50_000 budget.
        efe_telemetry::metrics::register_all_d18();

        // **Audit fix D19-P1.A1** — same eager-register pattern for
        // the 12 P1-Foundation instruments (+ 2 auxiliary counters).
        // Without this, the Grafana `efe-p1-slo.json` dashboard shows
        // "no data" on the 10 latency panels at boot, which makes
        // P1 surface look broken when it's actually just unused yet.
        // Cost : 12 histograms × 3 modes × (1 to 5 extra label values)
        // ≈ 110 series at maximum spec ; negligible vs D13 50 000
        // budget. Cf. `docs/EFE_P1_SURFACE_ARCHITECTURE.md` § 4.2.
        efe_telemetry::metrics::register_all_p1();

        // **Sprint D19 P2-Foundation** — same eager-register pattern
        // for the 10 P2-activation instruments. Without this, the
        // Grafana `efe-p2-funnel.json` dashboard shows "no data"
        // on the 8 funnel panels at boot. Cost : 10 metrics × ~ 1 500
        // séries au pic maximum ; négligeable vs D13 50 000 budget.
        // Cf. `docs/EFE_P2_SELF_SERVICE_ARCHITECTURE.md` § 4.2.
        efe_telemetry::metrics::register_all_p2();

        // **Sprint Φ-C D26 (Bundle embedder réel)** — eager-register
        // the 8 bundle instruments. Without this, le dashboard
        // bundle (= à venir D27) afficherait "no data" sur les
        // panels de inference / recall / size / verify. Cost :
        // 8 metrics × ~ 1 000 séries au pic ; négligeable.
        // Cf. `docs/EFE_BUNDLE_EMBEDDER_REAL_ARCHITECTURE.md` § 11.4.
        efe_telemetry::metrics::register_all_bundle();

        // ──────────────────────────────────────────────────────────
        // **Sprint M5 extension D34** — eager-register the 4
        // nano-agent instruments AND pre-populate the
        // `consensus_weight` gauge with the canonical values from
        // the `NanoAgentTemplate` trait (NaSafety = 2.0, etc.).
        // Without this, the Grafana `d34_templates.json` dashboard
        // shows "no data" until the first swarm executes — bad
        // operator experience.
        // Cost : 8 templates × 200 tenants × 2 counters + 8 weights
        // + 8 × 8 kernel routings = ~3 272 series ; ~6.5 % of the
        // 50 000-series budget.
        // ──────────────────────────────────────────────────────────
        efe_telemetry::metrics::register_all_nano_agent();

        // ──────────────────────────────────────────────────────────
        // **Sprint M5 extension D34** — eager-register the 5 Mode-2
        // instruments. Cost : 3 routers × 200 tenants × 2 counters
        // + 3 × 8 latency buckets + 3 × 5 anchors buckets + 1 collision
        // counter = ~1 246 series ; ~2.5 % of the budget.
        // ──────────────────────────────────────────────────────────
        efe_telemetry::metrics::register_all_mode2();

        // ──────────────────────────────────────────────────────────
        // **Sprint M5 extension D37** — eager-register the 4 Mode-3
        // instruments. Cost : 2 modes × 200 tenants + 3 aggregators
        // × 200 tenants + 200 tenants + 3 × 5 buckets = ~1 215
        // series ; ~2.4 % of the budget.
        // ──────────────────────────────────────────────────────────
        efe_telemetry::metrics::register_all_mode3();

        // **Audit fix D18-A1** — pin the `efe_replica_id` gauge so
        // dashboards can partition panels by pod. The gauge stays
        // at 1 for the lifetime of the process ; on rolling
        // restart the new pod's HOSTNAME publishes a new label.
        efe_telemetry::metrics::d18_instruments::replica_id()
            .with_label_values(&[&config.multi_replica.replica_id])
            .set(1);
        tracing::info!(
            target: "efe.multi_replica",
            replica_id = %config.multi_replica.replica_id,
            replicas_min = config.multi_replica.replicas_min,
            replicas_max = config.multi_replica.replicas_max,
            leader_election = config.multi_replica.leader_election_enabled,
            "D18 multi-replica wiring active"
        );

        // **Audit fix D18-A1** — instantiate the multi_replica
        // building blocks so they participate in the rest of
        // `serve` (leader-election loop, shutdown sequence, health
        // check, idempotency + rate-limit shared stores).
        // The PG-backed production impls require sqlx wiring that
        // lands in D18-bis ; for v0.1.19 we fall back to the
        // canonical Mock impls so unit + integration tests
        // exercise the full flow without Postgres.
        let multi_replica_handles = build_multi_replica_handles(&config.multi_replica)?;

        // **Audit fix K-A1 (Φ-D)** — register the 6 GPU instruments
        // eagerly when the runtime opts into GPU
        // (`EFE_RUNTIME_GPU_ENABLED=true`) so the Grafana D29 GPU
        // dashboard panels render zero-valued series at boot.
        // Recurrent dead-counter defect class (D17-AC2 / D18-A3).
        if config.runtime.gpu_enabled {
            efe_telemetry::metrics::register_all_gpu();

            // **Audit fix Φ-D.A3** — set le `efe_gpu_driver_version_info`
            // au boot (= prod caller pour le helper livré Φ-D D30).
            // Sans cet appel, le panel #10 du dashboard
            // `efe-gpu-kernels.json` reste "no data" — et l'alerte
            // GPU_DRIVER_DRIFT ne peut pas détecter de drift car
            // aucune série n'existe pour comparer.
            //
            // Sans la feature `cuda`, on enregistre un fingerprint
            // "empty" qui contient au moins le `kernels_sha` baked
            // au build (= traçabilité même en CPU build).
            let fp = efe_kernels::fingerprint::GpuFingerprint::probe();
            efe_telemetry::metrics::gpu_instruments::note_driver_version(
                fp.driver_version.as_deref().unwrap_or("none"),
                fp.cuda_runtime_version.as_deref().unwrap_or("none"),
                fp.cuda_compile_version.as_deref().unwrap_or("none"),
                fp.nccl_version.as_deref().unwrap_or("none"),
                fp.efe_kernels_cuda_sha256.as_deref().unwrap_or("unset"),
            );

            // Probe the GPU topology and log it. When the `cuda`
            // feature is OFF, this is a noop ; when ON, it surfaces
            // the detected GPUs + NCCL availability for ops debugging.
            #[cfg(feature = "cuda")]
            {
                let topo = efe_kernels::detect_gpu_topology();
                tracing::info!(
                    target: "efe.runtime.gpu",
                    n_gpus = topo.n_gpus(),
                    nccl_available = topo.nccl_available,
                    total_vram_gib = topo.total_vram_bytes() / (1024 * 1024 * 1024),
                    device_ordinal = config.runtime.gpu_device_ordinal,
                    streams = config.runtime.gpu_streams,
                    memory_hwm_percent = config.runtime.gpu_memory_hwm_percent,
                    fallback_policy = %config.runtime.gpu_fallback_policy,
                    check_finite = config.runtime.gpu_check_finite,
                    "D28 GPU runtime active"
                );
                if !topo.has_gpu() {
                    tracing::warn!(
                        target: "efe.runtime.gpu",
                        "EFE_RUNTIME_GPU_ENABLED=true but no GPU detected — \
                         falling back to CpuBackend ; check NVIDIA driver + \
                         visibility (CUDA_VISIBLE_DEVICES)."
                    );
                }
            }
            #[cfg(not(feature = "cuda"))]
            {
                tracing::warn!(
                    target: "efe.runtime.gpu",
                    "EFE_RUNTIME_GPU_ENABLED=true but `cuda` feature was not \
                     compiled in — `efe-kernels::CudaBackend` unavailable. \
                     Rebuild `efe-server` with --features cuda or unset the \
                     env var. Falling back to CpuBackend."
                );
            }
        }

        // **Audit fix D16-C1** — wire the embedder bundle subsystem
        // when the operator opts in via `EFE_EMBEDDER_BACKEND=bundle`.
        // The pre-audit code never read these env vars at runtime,
        // so an operator could set them and the bundle would never
        // load. Critical operator-surprise bug.
        //
        // **Audit fix S-A1 (D27)** — `build_embedder_registry` now
        // returns a `BundleWiring` triple (registry, active-bundle
        // slot, optional trust store) so `serve()` can wire all
        // three into `AdminServiceImpl::with_bundle_reload`.
        let bundle_wiring = build_embedder_registry(&config).await?;
        let embedder_registry = bundle_wiring.registry;
        let active_bundle_slot = bundle_wiring.active_bundle_slot;
        let embedder_trust_store = bundle_wiring.trust_store;

        // Build the AuthGuard.
        //
        // Sprint **D15** — three branches :
        //   1. `auth.enabled = false`           → disabled (anonymous).
        //   2. `auth.enabled & !oidc.enabled`   → HS256 (M10 path).
        //   3. `auth.enabled &  oidc.enabled`   → OIDC multi-issuer.
        //
        // The OIDC branch loads `oidc-issuers.json` and builds one
        // `JwksCache` per issuer up front. The actual `OidcValidator`
        // (and the corresponding `AuthGuard`) is constructed AFTER
        // the catalog backend section below — so we can wire the
        // `TenantStatusGate` (audit fix D15-C1) when a Postgres pool
        // is available. JWKS periodic refresh tasks are spawned later
        // in `serve` once we have a Shutdown handle.
        let mut oidc_caches: Vec<JwksCache> = Vec::new();
        let mut oidc_registry: Option<IssuerRegistry> = None;
        if config.auth.enabled && config.auth.oidc.enabled {
            let params = LoadParams::from_oidc_config(&config.auth.oidc);
            let registry = load_issuers_file(
                std::path::Path::new(&config.auth.oidc.issuers_file),
                &params,
            )
            .map_err(|e| anyhow::anyhow!("OIDC issuers file load failed: {e}"))?;
            let n_issuers = registry.len();
            tracing::info!(
                target: "efe.auth.oidc",
                n_issuers,
                issuers_file = %config.auth.oidc.issuers_file,
                "OIDC enabled — IssuerRegistry loaded"
            );
            oidc_caches.extend(registry.iter().map(|e| e.jwks_cache.clone()));
            efe_telemetry::metrics::d15_instruments::oidc_issuers_loaded().set(n_issuers as i64);
            oidc_registry = Some(registry);
        }

        let rate_limiter = if config.ratelimit.enabled {
            Some(Arc::new(BucketRegistry::new(&config.ratelimit)))
        } else {
            None
        };

        let idempotency = if config.idempotency.enabled {
            Some(Arc::new(IdempotencyCache::new(
                config.idempotency.cache_capacity,
                config.idempotency.ttl,
            )))
        } else {
            None
        };

        // Sprint **D12-bis** — resolve catalog backend + propagate the
        // `config.catalog.policy` quotas. **Audit D12-bis fix C1** : prior
        // to this fix the runtime ignored `EFE_CATALOG_MAX_*` env vars
        // entirely — `InMemoryRegistry::new()` hardcoded `CatalogPolicy::default()`.
        // We now build a `CatalogPolicy` from config and inject it into the
        // registry so the operator's Helm values actually take effect.
        //
        // **Sprint D14** — open the Postgres branch. `connect` builds
        // the pool (fail-fast on bad credentials / unreachable host) ;
        // `run_migrations` applies forward-only migrations from
        // `crates/efe-server/migrations/` (fail-fast on schema drift).
        //
        // When the caller supplies their own `catalog_registry` (test
        // pre-population path), we honour it as-is — they own the policy
        // for that registry.
        let catalog_policy = CatalogPolicy::from(&config.catalog.policy);
        let catalog_audit = Arc::new(CatalogAuditEmitter::new());

        // Audit fix C1+C2 — capture audit-signing handles here so
        // `serve` can wire the shutdown-aware periodic self-verify
        // and the drain-at-shutdown for in-flight emit tasks.
        let mut audit_signing_for_serve: Option<AuditSigningHandles> = None;

        // -----------------------------------------------------------------
        // Sprint **D14-bis** — build the audit sink for the registry.
        //
        // When `audit_signing.enabled = true`, the registry uses
        // `HmacChainedAuditSink` which signs every event and persists
        // it to the `audit_events` table. The legacy `TracingAuditSink`
        // is the fallback (D12 behaviour).
        //
        // The HMAC sink requires a Postgres pool ; we build it AFTER
        // the Postgres branch below has built the pool. To avoid the
        // ordering dance, we start by deferring the audit emitter
        // construction until we know the backend.
        // -----------------------------------------------------------------
        // **Audit fix D15-C1** — capture the Postgres pool when the
        // catalog backend is Postgres, so we can build a `TenantStore`
        // and wire it as the `OidcValidator`'s `TenantStatusGate`.
        let mut pg_pool_for_tenants: Option<sqlx::postgres::PgPool> = None;

        let catalog_registry: Arc<dyn CatalogRegistry> = match self.catalog_registry {
            Some(r) => r,
            None => match config.catalog.backend {
                CatalogBackend::Memory => Arc::new(InMemoryRegistry::with_policy_and_audit(
                    catalog_policy.clone(),
                    CatalogAuditEmitter::new(),
                )),
                CatalogBackend::Postgres => {
                    let pg = PostgresCatalogRegistry::connect(
                        &config.catalog.postgres,
                        catalog_policy.clone(),
                        CatalogAuditEmitter::new(),
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!("Postgres catalog backend init failed: {e}"))?;
                    pg.run_migrations().await.map_err(|e| {
                        anyhow::anyhow!("Postgres catalog migrations failed at boot: {e}")
                    })?;
                    // Eagerly register D14 instruments so the
                    // Prometheus `/metrics` endpoint reports the new
                    // gauges and counter immediately, even before the
                    // first `register` RPC.
                    efe_telemetry::metrics::register_all_d14();

                    // **Audit fix D15-C1** — stash the pool so the
                    // OIDC branch (constructed below) can wire a
                    // `TenantStore` gate.
                    pg_pool_for_tenants = Some(pg.pool().clone());

                    // ---------------------------------------------------------------
                    // Sprint **D14-bis** — wire the HmacChainedAuditSink.
                    //
                    // Construct the sink AFTER the registry's pool exists. We
                    // re-build the registry with the new sink so the trait
                    // object emits via HMAC. The pool is shared (Arc<PgPool>
                    // internally) — no double-connection.
                    // ---------------------------------------------------------------
                    if config.catalog.audit_signing.enabled {
                        let pool = pg.pool().clone();
                        let loaded = load_keys_file(std::path::Path::new(
                            &config.catalog.audit_signing.keys_file,
                        ))
                        .map_err(|e| anyhow::anyhow!("audit HMAC keys file load failed: {e}"))?;
                        let n_keys = loaded.all.len();
                        tracing::info!(
                            target: "efe.audit.hmac",
                            active_key_id = %loaded.active_id,
                            n_keys,
                            "audit signing enabled — HmacChainedAuditSink active"
                        );
                        // Construct the concrete sink first so we can
                        // grab `keys()` for the periodic verifier and
                        // a clone for the shutdown drain path
                        // (audit fix C1+C2).
                        let sink_concrete = HmacChainedAuditSink::new(
                            pool.clone(),
                            loaded,
                            config.catalog.audit_signing.chain_to_tracing,
                        );
                        // Stash for `serve` : it spawns the periodic
                        // self-verify with shutdown handling AND calls
                        // `drain_pending` at shutdown (audit fix C1+C2).
                        audit_signing_for_serve = Some(AuditSigningHandles {
                            sink: sink_concrete.clone(),
                            self_verify_every_s: config.catalog.audit_signing.self_verify_every_s,
                        });
                        let sink: Arc<dyn AuditSink> = Arc::new(sink_concrete);

                        // Re-register the D14-bis instruments + initial gauges.
                        efe_telemetry::metrics::register_all_d14bis();
                        efe_telemetry::metrics::d14bis_instruments::audit_keys_active()
                            .set(n_keys as i64);

                        // Re-build the registry with the new sink.
                        let pg_with_sink = PostgresCatalogRegistry::from_pool(
                            pool,
                            catalog_policy.clone(),
                            AuditEmitter::with_sink(sink),
                        );
                        Arc::new(pg_with_sink)
                    } else {
                        Arc::new(pg)
                    }
                }
            },
        };

        // **Audit fix D15-C1** — now that the Postgres pool (if any)
        // exists, build the `AuthGuard`. Three branches :
        //   1. auth disabled → AuthGuard::disabled()
        //   2. OIDC → OidcValidator + optional TenantStatusGate
        //   3. HS256 (legacy)
        //
        // The OIDC branch wires a `TenantStore` gate iff a Postgres
        // pool is available. Without the gate (e.g. Memory catalog
        // backend), the validator behaves as the pre-audit version
        // — operators are warned at boot via `tracing::warn!`.
        let auth: Arc<AuthGuard> = if config.auth.enabled {
            if let Some(registry) = oidc_registry {
                let validator = if let Some(pool) = &pg_pool_for_tenants {
                    let gate: Arc<dyn crate::auth::TenantStatusGate> =
                        Arc::new(crate::auth::TenantStore::from_pool(pool.clone()));
                    tracing::info!(
                        target: "efe.auth.oidc",
                        "tenant status gate wired (TenantStore) — \
                         suspended tenants will be rejected"
                    );
                    Arc::new(OidcValidator::with_tenant_gate(Arc::new(registry), gate))
                } else {
                    tracing::warn!(
                        target: "efe.auth.oidc",
                        "no Postgres pool available — TenantStore gate NOT \
                         wired ; tokens for any tenant resolved by an issuer \
                         will validate regardless of registry status. \
                         Set EFE_CATALOG_BACKEND=postgres for the audit-fix \
                         D15-C1 enforcement."
                    );
                    Arc::new(OidcValidator::new(Arc::new(registry)))
                };
                Arc::new(AuthGuard::enabled_oidc(validator))
            } else {
                let store = match self.token_store.clone() {
                    Some(ts) => ts,
                    None => Arc::new(TokenStore::from_hs256(
                        &config.auth.hs256_secrets,
                        &config.auth.jwt_audience,
                        config.auth.jwt_issuer.as_deref(),
                    )?),
                };
                Arc::new(AuthGuard::enabled(store))
            }
        } else {
            Arc::new(AuthGuard::disabled())
        };

        // ──────────────────────────────────────────────────────────
        // **Phase-2 PR #11** — meta-swarm subsystem plumbing.
        //
        // When `EFE_META_SWARM_ENABLED=true`, build a long-lived
        // gRPC client to the meta-swarm controller and hold it on
        // the `Server` for the duration of the process.  PR #11 does
        // NOT route any request through this client ; that wires in
        // PR #15.  The plumbing-only doctrine (PR #11 architecture
        // § 1.3) requires that disabled-mode behaviour be
        // byte-identical to pre-PR-#11 deployments — when
        // `enabled=false`, `meta_swarm_client` is `None` and no
        // metric series are registered.
        //
        // Fail-fast (PR #11 AD3) : if the connect or the optional
        // Health probe fails, propagate the anyhow error.  `main.rs`
        // wraps `build().await` with `.context("failed to build
        // server")?` and exits non-zero ; operators get an
        // immediate, actionable error at boot rather than a
        // degraded server.
        let meta_swarm_client = if config.meta_swarm.enabled {
            let metrics =
                crate::services::meta_swarm::MetaSwarmMetrics::register_into_default_registry()
                    .map_err(|e| anyhow::anyhow!("register meta-swarm metrics : {e}"))?;
            let client =
                crate::services::meta_swarm::MetaSwarmClient::connect(&config.meta_swarm, metrics)
                    .await
                    .map_err(|e| anyhow::anyhow!("meta-swarm connect : {e}"))?;
            tracing::info!(
                endpoint = %client.endpoint(),
                "meta-swarm subsystem enabled and connected",
            );
            Some(Arc::new(client))
        } else {
            tracing::info!("meta-swarm subsystem disabled (EFE_META_SWARM_ENABLED=false)");
            None
        };

        Ok(Server {
            config,
            backend,
            registry: self.registry,
            catalog_registry,
            catalog_audit,
            audit_signing: audit_signing_for_serve,
            auth,
            oidc_caches,
            embedder_registry,
            active_bundle_slot,
            embedder_trust_store,
            rate_limiter,
            idempotency,
            multi_replica: multi_replica_handles,
            meta_swarm_client,
        })
    }
}

// =====================================================================
// Sprint D18 — Multi-replica handles factory
// =====================================================================

/// Sprint **D18 audit fix A1** — bundle the multi_replica components
/// instantiated by `Server::build`. Each field maps to one of the 5
/// D18 sub-modules.
///
/// The PG-backed production impls (`PgIdempotencyStore`,
/// `PgRateLimitStore`, `K8sLeaseLock`) currently return Err per the
/// v0.1.19 stub. The Mock impls (`MockIdempotencyStore`,
/// `MockRateLimitStore`, `MockLeaderLock`) are wired by default so
/// unit + integration tests exercise the full code path. D18-bis
/// will swap in the production impls once sqlx + kube-rs land.
pub(crate) struct MultiReplicaHandles {
    /// Shared idempotency cache (Mock for v0.1.19 ; PG in D18-bis).
    pub(crate) idempotency_store: Arc<dyn IdempotencyStore>,
    /// Shared rate-limit store (Mock for v0.1.19 ; PG in D18-bis).
    pub(crate) ratelimit_store: Arc<dyn RateLimitStore>,
    /// Leader-lock implementation. `Some(_)` iff `leader_election_enabled`.
    pub(crate) leader_lock: Option<Arc<dyn LeaderLock>>,
    /// Pre-built phase timeouts for the 3-phase shutdown sequence.
    pub(crate) shutdown_timeouts: ShutdownTimeouts,
}

impl std::fmt::Debug for MultiReplicaHandles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiReplicaHandles")
            .field("leader_lock_present", &self.leader_lock.is_some())
            .field("shutdown_timeouts", &self.shutdown_timeouts)
            .finish_non_exhaustive()
    }
}

/// **Audit fix D18-A1** — factory called by `ServerBuilder::build`.
/// Returns the bundled handles or a config error if the timeouts
/// don't validate against the K8s grace period budget.
fn build_multi_replica_handles(
    cfg: &crate::config::MultiReplicaConfig,
) -> anyhow::Result<MultiReplicaHandles> {
    let idempotency_store: Arc<dyn IdempotencyStore> = Arc::new(MockIdempotencyStore::new());

    let ratelimit_store: Arc<dyn RateLimitStore> = Arc::new(MockRateLimitStore::new());

    let leader_lock: Option<Arc<dyn LeaderLock>> = if cfg.leader_election_enabled {
        // v0.1.19 ships the Mock impl in production-mode boot too,
        // because the `K8sLeaseLock` stub returns Err on every
        // call (so leader-only tasks would fail closed). Once
        // D18-bis wires kube-rs we'll swap to `K8sLeaseLock` here.
        tracing::warn!(
            target: "efe.multi_replica",
            replica_id = %cfg.replica_id,
            lease_name = %cfg.leader_lease_name,
            "leader-election enabled but K8sLeaseLock is a v0.1.19 stub ; \
             falling back to MockLeaderLock until D18-bis ships kube-rs"
        );
        Some(Arc::new(MockLeaderLock::available(cfg.replica_id.clone())))
    } else {
        None
    };

    let shutdown_timeouts = ShutdownTimeouts {
        drain_rpc: Duration::from_millis(cfg.shutdown_drain_rpc_ms),
        audit_flush: Duration::from_millis(cfg.shutdown_audit_flush_ms),
        pg_close: Duration::from_millis(cfg.shutdown_pg_close_ms),
    };

    Ok(MultiReplicaHandles {
        idempotency_store,
        ratelimit_store,
        leader_lock,
        shutdown_timeouts,
    })
}

// =====================================================================
// Sprint D16 — embedder bundle wiring
// =====================================================================

/// **Audit fix D16-C1** — build the [`EmbedderRegistry`] when the
/// operator opts into `EFE_EMBEDDER_BACKEND=bundle`. Pre-audit code
/// never consulted these env vars at runtime, so the bundle path
/// was effectively broken.
///
/// On `Bundle` mode :
///   1. Load the trust store from `bundle.trust_store_path`.
///   2. Eagerly load the bundle at `bundle.path` (fail boot on
///      sig/hash/policy violations — operator wants to know NOW).
///   3. Update the `efe_embedder_active_bundles` gauge.
///
/// On `Legacy` mode : returns an empty registry. Existing M10 path
/// (Python `LocalSTEmbedder`) takes over via the gRPC handler stack.
///
/// **Audit fix S-A1** — pre-fix this function returned only the
/// registry. The trust store + the active bundle handle were dropped
/// after the initial load. The Sprint D27 `AdminService.ReloadBundle`
/// path needs all three (registry, slot, trust) so the function now
/// returns a [`BundleWiring`] tuple.
async fn build_embedder_registry(config: &ServerConfig) -> anyhow::Result<BundleWiring> {
    use crate::config::EmbedderBackend;
    use crate::embedder::{
        load_trust_store_file, ActiveBundleSlot, EmbedderRegistry, LoadPolicy, TrustStore,
    };
    use std::sync::Arc;

    let registry = EmbedderRegistry::new();
    let active_bundle_slot = Arc::new(ActiveBundleSlot::new());

    if !matches!(config.embedder.backend, EmbedderBackend::Bundle) {
        tracing::info!(
            target: "efe.embedder",
            backend = config.embedder.backend.as_str(),
            "embedder backend = legacy ; skipping D16 bundle preload"
        );
        return Ok(BundleWiring {
            registry,
            active_bundle_slot,
            trust_store: None,
        });
    }

    // Load the trust store unless skip-policy.
    let trust = if matches!(
        config.embedder.require_signature,
        crate::config::SignaturePolicy::Skip
    ) {
        Arc::new(TrustStore { keys: vec![] })
    } else {
        let path = std::path::Path::new(&config.embedder.trust_store_path);
        let ts = load_trust_store_file(path).map_err(|e| {
            anyhow::anyhow!(
                "embedder trust store load failed at {}: {e}",
                config.embedder.trust_store_path
            )
        })?;
        tracing::info!(
            target: "efe.embedder",
            n_keys = ts.len(),
            trust_store = %config.embedder.trust_store_path,
            "embedder trust store loaded"
        );
        Arc::new(ts)
    };

    let policy = LoadPolicy {
        signature: config.embedder.require_signature,
        require_sbom: config.embedder.require_sbom,
        ack_non_redistributable: config.embedder.acknowledge_non_redistributable,
    };
    let bundle_root = std::path::PathBuf::from(&config.embedder.bundle_path);
    let load_timeout = std::time::Duration::from_secs(config.embedder.load_timeout_s);

    // Read the manifest just enough to extract the bundle_id (the
    // registry key). We can't trust this id against tampering yet
    // (signature verifies inside ensure_loaded), but the registry
    // key is just a string handle — the security proof comes from
    // the byte-exact manifest fingerprint that the catalog handler
    // checks at register/evolve time.
    let manifest_path = bundle_root.join("manifest.json");
    let manifest_bytes = tokio::fs::read(&manifest_path).await.map_err(|e| {
        anyhow::anyhow!(
            "embedder bundle manifest unreadable at {}: {e}",
            manifest_path.display()
        )
    })?;
    let pre_manifest: crate::embedder::Manifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| anyhow::anyhow!("embedder bundle manifest parse failed: {e}"))?;
    let bundle_id = pre_manifest.bundle_id.clone();

    // Eager load — fail boot on any error. Operator wants to know
    // immediately if the bundle is broken.
    let result = registry
        .ensure_loaded_with_timeout(
            &bundle_id,
            bundle_root.clone(),
            policy,
            trust.clone(),
            load_timeout,
        )
        .await;
    match result {
        Ok(bundle) => {
            efe_telemetry::metrics::d16_instruments::embedder_active_bundles()
                .set(registry.len().await as i64);
            // **Audit fix S-A1** — populate the active-bundle slot at
            // boot so AdminService.ReloadBundle has a non-empty
            // initial state to swap from. The slot's `set_initial`
            // also stamps `last_reload_ts_ms` to now() so the
            // dashboard panel "time since last reload" doesn't show
            // 1970-01-01 on a freshly-booted pod.
            active_bundle_slot
                .set_initial(std::sync::Arc::new(bundle))
                .await;
            tracing::info!(
                target: "efe.embedder",
                bundle_id = %bundle_id,
                bundle_path = %bundle_root.display(),
                "embedder bundle loaded + verified at boot"
            );
            Ok(BundleWiring {
                registry,
                active_bundle_slot,
                trust_store: Some(trust),
            })
        }
        Err(e) => Err(anyhow::anyhow!(
            "embedder bundle load failed at boot ({}): {}",
            bundle_root.display(),
            e
        )),
    }
}

/// **Audit fix S-A1** — bundle wiring triple consumed by
/// `Server::build`. The active-bundle slot is always returned (an
/// empty slot is cheap when no bundle is loaded). The trust store is
/// `Some(_)` only when bundle backend is enabled.
struct BundleWiring {
    registry: crate::embedder::EmbedderRegistry,
    active_bundle_slot: Arc<crate::embedder::ActiveBundleSlot>,
    trust_store: Option<Arc<crate::embedder::TrustStore>>,
}

/// Sprint **D14-bis** — handles wired by `build` and consumed by
/// `serve` to start the audit-signing background tasks (periodic
/// self-verify with shutdown.notified() + drain pending emit tasks
/// at shutdown).
struct AuditSigningHandles {
    sink: HmacChainedAuditSink,
    self_verify_every_s: u64,
}

/// Assembled, ready-to-serve gRPC server.
pub struct Server {
    config: ServerConfig,
    backend: DynExecuteBackend,
    registry: Option<Arc<dyn AlignmentRegistry>>,
    /// Sprint **D12-bis** — catalog admin storage. The policy quotas
    /// configured via `config.catalog.policy` are injected at construction
    /// time (cf. `ServerBuilder::build` audit fix C1) — the registry is
    /// the single source of truth for quota enforcement.
    catalog_registry: Arc<dyn CatalogRegistry>,
    /// Sprint **D12-bis** — audit emitter for the catalog handler.
    catalog_audit: Arc<CatalogAuditEmitter>,
    /// Sprint **D14-bis** — audit-signing handles ; `Some(_)` when
    /// `EFE_AUDIT_SIGNING_ENABLED=true`. Consumed by `serve` to spawn
    /// the shutdown-aware periodic self-verify and to drain pending
    /// sign tasks at shutdown (audit fixes C1+C2).
    audit_signing: Option<AuditSigningHandles>,
    auth: Arc<AuthGuard>,
    /// Sprint **D15** — JWKS caches kept aside so `serve` can spawn
    /// one `periodic_refresh` task per issuer once it has the
    /// shutdown handle. Empty when OIDC is disabled.
    oidc_caches: Vec<JwksCache>,
    /// Sprint **D16** — embedder bundle registry. Populated at
    /// `build` time when `EFE_EMBEDDER_BACKEND=bundle`. Empty
    /// (no-op) on the legacy path.
    #[allow(dead_code)] // exposed via Server::embedder_registry()
    embedder_registry: crate::embedder::EmbedderRegistry,
    /// **Sprint D27** (M4 § 11) — atomic active-bundle slot. Always
    /// constructed at server boot (an empty slot is cheap) ; populated
    /// when the bundle backend loads its first bundle. Cloned into
    /// `AdminServiceImpl::with_bundle_reload` in `serve()` so the
    /// `ReloadBundle` RPC can swap the contents at runtime.
    active_bundle_slot: Arc<crate::embedder::ActiveBundleSlot>,
    /// **Sprint D27** (M4 § 11) — Ed25519 trust store. `Some(_)` only
    /// when the bundle backend is enabled AND signature verification
    /// is configured. Forwarded to `AdminServiceImpl` so the reload
    /// path can verify replacement bundles against the same keys
    /// that gated the original load.
    ///
    /// **Audit fix S-A1** — pre-fix the trust store was loaded into
    /// `build_embedder_registry` and dropped after the initial bundle
    /// load. Without it, the ReloadBundle RPC would have had nothing
    /// to verify the new bundle's signature against.
    embedder_trust_store: Option<Arc<crate::embedder::TrustStore>>,
    rate_limiter: Option<Arc<BucketRegistry>>,
    idempotency: Option<Arc<IdempotencyCache>>,
    /// Sprint **D18 audit fix A1** — multi_replica subsystem
    /// handles. Always `Some(_)` ; the inner `leader_lock` is
    /// `None` only when leader-election is disabled.
    #[allow(dead_code)] // surfaced via accessor + serve()
    multi_replica: MultiReplicaHandles,
    /// **Phase-2 PR #11** — meta-swarm gRPC client.  `Some(_)` iff
    /// `EFE_META_SWARM_ENABLED=true` at boot ; `None` otherwise.
    ///
    /// In PR #11 the client is held but NOT consumed by any request
    /// path (plumbing-only doctrine).  PR #15 will route specific
    /// multi-swarm coordination calls through it.
    ///
    /// Cf. `docs/EFE_A1_PHASE2_PR11_ARCHITECTURE.md` § 11.
    meta_swarm_client: Option<Arc<crate::services::meta_swarm::MetaSwarmClient>>,
}

impl Server {
    /// Start a new builder.
    #[must_use]
    pub fn builder() -> ServerBuilder {
        ServerBuilder::default()
    }

    /// Accessor for tests / introspection.
    #[must_use]
    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    /// **Phase-2 PR #11** — accessor for the meta-swarm client.
    /// `Some(_)` iff `EFE_META_SWARM_ENABLED=true` at boot.
    ///
    /// Used by integration tests to assert the plumbing-only doctrine
    /// (PR #11 architecture § 15).  PR #15 dispatch will use this
    /// accessor from inside the `SwarmServiceImpl` to route specific
    /// requests through the meta-swarm controller.
    #[must_use]
    pub fn meta_swarm_client(&self) -> Option<&Arc<crate::services::meta_swarm::MetaSwarmClient>> {
        self.meta_swarm_client.as_ref()
    }

    /// Serve until `shutdown` is triggered.
    ///
    /// Shutdown semantics:
    /// - `shutdown.notified()` is awaited ; when it fires, the listener
    ///   stops accepting new connections and existing streams drain.
    /// - A watchdog task flips the `tonic-health` reports to
    ///   `NOT_SERVING` so upstream load balancers can stop routing.
    /// - If drain exceeds `config.shutdown.force_abort_after`, the
    ///   serve future is aborted with a warning (streams in flight
    ///   observe `CANCELLED`).
    pub async fn serve(self, addr: SocketAddr, shutdown: Shutdown) -> anyhow::Result<()> {
        // Sprint **D15** — for every JWKS cache that was constructed in
        // `build`, spawn a periodic refresh task. The task does an
        // initial fetch (so the cache is warm before the first token
        // arrives), then re-fetches every `ttl` until shutdown.
        // Tasks observe `shutdown.notified()` and exit cleanly.
        for cache in &self.oidc_caches {
            let cache = cache.clone();
            let interval = Duration::from_secs(self.config.auth.oidc.jwks_ttl_s);
            let shutdown = shutdown.clone();
            tokio::spawn(async move {
                crate::auth::periodic_refresh(cache, interval, shutdown).await;
            });
        }

        // **Audit fix D18-A5** — when leader-election is enabled,
        // spawn the leader-election orchestrator loop. It tries to
        // acquire the lease at boot, then renews on a half-deadline
        // cadence, and exits cleanly on shutdown. Leader-only tasks
        // (idempotency expire_sweep, ratelimit re-balance) are
        // gated behind `leader_lock.is_held()` so they only run on
        // the elected replica.
        if let Some(lock) = self.multi_replica.leader_lock.clone() {
            let renew_every =
                Duration::from_secs(self.config.multi_replica.leader_renew_deadline_s);
            let shutdown_for_leader = shutdown.clone();
            tokio::spawn(async move {
                run_leader_election_loop(lock, renew_every, shutdown_for_leader).await;
            });
        }

        // Sprint **D14-bis audit fix C1+C2** — start the audit-signing
        // background tasks now that we have a `Shutdown` handle.
        // `periodic_self_verify` listens on shutdown.notified() and
        // exits cleanly. `drain_pending` is invoked AFTER the serve
        // future resolves but BEFORE we drop the audit sink, so any
        // in-flight sign+persist task gets a chance to commit.
        if let Some(handles) = &self.audit_signing {
            if handles.self_verify_every_s > 0 {
                let pool = handles.sink.pool().clone();
                let keys = handles.sink.keys();
                let interval = Duration::from_secs(handles.self_verify_every_s);
                let shutdown = shutdown.clone();
                tokio::spawn(crate::catalog::audit_hmac::periodic_self_verify(
                    pool, keys, interval, shutdown,
                ));
            }
        }

        // --- Services ---
        let swarm_svc = SwarmServiceImpl::new(
            self.backend.clone(),
            self.auth.clone(),
            self.idempotency.clone(),
            self.rate_limiter.clone(),
        );

        // --- Reflection v1 ---
        let reflection = tonic_reflection::server::Builder::configure()
            .register_encoded_file_descriptor_set(crate::proto::FILE_DESCRIPTOR_SET)
            .build_v1()?;

        // --- Health service (start SERVING) ---
        let (mut health_reporter, health_service) = tonic_health::server::health_reporter();
        health_reporter
            .set_serving::<SwarmServiceServer<SwarmServiceImpl>>()
            .await;
        let admin_attached = self.registry.is_some();
        if admin_attached {
            health_reporter
                .set_serving::<AdminServiceServer<AdminServiceImpl>>()
                .await;
        }
        // Sprint **D12-bis** — CatalogService is always attached
        // (the in-memory registry is built unconditionally).
        health_reporter
            .set_serving::<CatalogServiceServer<CatalogServiceImpl>>()
            .await;
        // **Audit fix S-A6** — Sprint D34/D37 services are always
        // attached too (cf. router add_service below). Without these
        // set_serving calls the gRPC health probe returned UNKNOWN for
        // the Mode-2/Mode-3 services even though they were responding
        // — load balancers configured to gate on "all SERVING" would
        // refuse to route traffic. Fixed by mirroring the catalog
        // wiring pattern.
        health_reporter
            .set_serving::<Mode2ServiceServer<Mode2ServiceImpl>>()
            .await;
        health_reporter
            .set_serving::<Mode3ServiceServer<Mode3ServiceImpl>>()
            .await;

        // Flip NOT_SERVING as soon as shutdown is requested — allows
        // load balancers to drain the pod before we stop accepting.
        let health_shutdown_task = {
            let shutdown = shutdown.clone();
            tokio::spawn(async move {
                shutdown.notified().await;
                tracing::info!("shutdown requested — flipping health to NOT_SERVING");
                health_reporter
                    .set_not_serving::<SwarmServiceServer<SwarmServiceImpl>>()
                    .await;
                if admin_attached {
                    health_reporter
                        .set_not_serving::<AdminServiceServer<AdminServiceImpl>>()
                        .await;
                }
                health_reporter
                    .set_not_serving::<CatalogServiceServer<CatalogServiceImpl>>()
                    .await;
                // **Audit fix S-A6** — symmetric set_not_serving for
                // Mode-2 / Mode-3 so load balancers correctly drain
                // them on shutdown.
                health_reporter
                    .set_not_serving::<Mode2ServiceServer<Mode2ServiceImpl>>()
                    .await;
                health_reporter
                    .set_not_serving::<Mode3ServiceServer<Mode3ServiceImpl>>()
                    .await;
            })
        };

        // --- Router ---
        // The MetricsLayer (M11) wraps every RPC with Prometheus
        // counters/histograms/gauges. It's transparent to the service
        // stack and relies on the global registry exposed by
        // `efe-telemetry`.
        let router = TonicServer::builder()
            .layer(efe_telemetry::MetricsLayer::new())
            .concurrency_limit_per_connection(self.config.max_concurrent_streams)
            .timeout(self.config.request_timeout)
            .add_service(health_service)
            .add_service(reflection)
            .add_service(SwarmServiceServer::new(swarm_svc));

        // ──────────────────────────────────────────────────────────
        // **Audit fix S-A1** — wire the catalog registry and the
        // bundle-reload triple into AdminServiceImpl. Pre-fix the
        // builder methods existed but were NEVER called from serve(),
        // so all 8 D21 catalog RPCs returned
        // `Status::FailedPrecondition("catalog registry not wired")`
        // in production. Recurrence of the canonical "wiring forgotten"
        // defect class (D17-AC1, D18-A1, K-A1, F-A1, …).
        // ──────────────────────────────────────────────────────────
        let router = if let Some(reg) = self.registry.clone() {
            let mut admin_svc =
                AdminServiceImpl::new(reg, self.auth.clone(), self.rate_limiter.clone());
            // Always wire the catalog registry — it's built unconditionally
            // (Memory backend if Postgres isn't configured).
            admin_svc = admin_svc.with_catalog_registry(self.catalog_registry.clone());
            // Bundle reload only when the trust store + initial bundle
            // are both available. The active-bundle slot is always
            // present (empty when no bundle loaded), so we gate on
            // `embedder_trust_store.is_some()` which proves the bundle
            // backend is enabled AND signature verification is wired.
            if let Some(trust) = self.embedder_trust_store.as_ref() {
                admin_svc = admin_svc.with_bundle_reload(
                    self.active_bundle_slot.clone(),
                    self.embedder_registry.clone(),
                    trust.clone(),
                );
                tracing::info!(
                    target: "efe.admin",
                    "AdminService.ReloadBundle wired (active-bundle slot + trust store)"
                );
            } else {
                tracing::info!(
                    target: "efe.admin",
                    "AdminService.ReloadBundle handler attached but bundle backend \
                     disabled — RPC returns Status::FailedPrecondition until operator \
                     sets EFE_EMBEDDER_BACKEND=bundle"
                );
            }
            router.add_service(AdminServiceServer::new(admin_svc))
        } else {
            router
        };

        // Sprint **D12-bis** — wire the CatalogService. The handler is
        // a thin translation layer between proto and the catalog
        // registry trait ; quota enforcement lives entirely in the
        // registry (audit fix L1 — handler no longer carries a redundant
        // `Arc<CatalogPolicy>`).
        let catalog_svc = CatalogServiceImpl::new(
            self.catalog_registry.clone(),
            self.catalog_audit.clone(),
            self.auth.clone(),
            self.rate_limiter.clone(),
        );
        let router = router.add_service(CatalogServiceServer::new(catalog_svc));

        // ──────────────────────────────────────────────────────────
        // Sprint **D34** (M4 § 13) — wire `Mode2Service`.
        //
        // Three built-in routers are registered (`mock`, `openai`,
        // `anthropic`). The dim is taken from the active embedder
        // bundle when one is loaded ; otherwise we fall back to 384
        // (paraphrase-multilingual-MiniLM-L12-v2 default). When D27
        // wires bundle-driven dims, the value will follow the active
        // bundle automatically.
        //
        // Audit fix S-A5 — the Mode-2 service is **always** wired
        // when the swarm service is present. Operators that don't
        // want Mode-2 should set `EFE_RUNTIME_MODE2_ENABLED=false`
        // (gated by a future config field ; for v1.0 we ship enabled).
        // ──────────────────────────────────────────────────────────
        let mode2_dim = 384_usize;
        let mode2_svc = Mode2ServiceImpl::new(
            self.backend.clone(),
            Mode2ServiceImpl::default_routers(mode2_dim),
            self.auth.clone(),
            self.rate_limiter.clone(),
        );
        let mode2_arc = Arc::new(mode2_svc);
        let router = router.add_service(Mode2ServiceServer::from_arc(mode2_arc.clone()));

        // ──────────────────────────────────────────────────────────
        // Sprint **D37** (M4 § 14) — wire `Mode3Service`.
        //
        // The three built-in aggregators (`mean`, `weighted`,
        // `median`) are registered. Mode-3 holds an `Arc` to the
        // shared backend AND an optional `Arc<Mode2ServiceImpl>` so
        // sub-swarms can fan out into Mode-2 routing on demand.
        // ──────────────────────────────────────────────────────────
        let mode3_svc = Mode3ServiceImpl::new(
            self.backend.clone(),
            Some(mode2_arc),
            Mode3ServiceImpl::default_aggregators(),
            self.auth.clone(),
            self.rate_limiter.clone(),
        );
        let router = router.add_service(Mode3ServiceServer::new(mode3_svc));

        tracing::info!(
            addr = %addr,
            auth_enabled = self.config.auth.enabled,
            admin_enabled = admin_attached,
            catalog_backend = ?self.config.catalog.backend,
            rate_limit = self.rate_limiter.is_some(),
            grace_period_s = self.config.shutdown.grace_period.as_secs(),
            force_abort_s = self.config.shutdown.force_abort_after.as_secs(),
            "efe-server listening"
        );

        // --- Serve with force-abort watchdog ---
        //
        // We run `serve_with_shutdown` in a task and race it against a
        // watchdog that triggers `force_abort_after` milliseconds AFTER
        // the shutdown signal fires. If the server hasn't finished
        // draining by then, we abort.
        let force_abort = self.config.shutdown.force_abort_after;
        let shutdown_for_serve = shutdown.clone();
        let serve_fut = async move {
            router
                .serve_with_shutdown(addr, async move {
                    shutdown_for_serve.notified().await;
                })
                .await
        };
        let shutdown_watch = shutdown.clone();
        let watchdog = async move {
            shutdown_watch.notified().await;
            tokio::time::sleep(force_abort).await;
        };

        tokio::pin!(serve_fut);
        tokio::pin!(watchdog);
        let outcome: anyhow::Result<()> = tokio::select! {
            result = &mut serve_fut => {
                result.map_err(anyhow::Error::from)
            }
            () = &mut watchdog => {
                tracing::warn!(
                    force_abort_s = force_abort.as_secs(),
                    "force-abort deadline reached — dropping in-flight streams"
                );
                // Dropping serve_fut cancels the server.
                Ok(())
            }
        };

        // Wait briefly for the health task so we don't leak it.
        let _ = tokio::time::timeout(Duration::from_millis(100), health_shutdown_task).await;

        // **Audit fix D18-A6** — drive the 3-phase graceful shutdown
        // sequence. Each phase has its own timeout (default
        // 10 s/10 s/30 s) ; observation of `efe_shutdown_phase_seconds`
        // happens inside `ShutdownSequence`. We invoke the phases
        // even on the force-abort path so that as much state as
        // possible flushes before SIGKILL.
        let mut shutdown_seq = ShutdownSequence::new(self.multi_replica.shutdown_timeouts);
        // Phase 1 — drain RPC. Already done above by
        // `serve_with_shutdown` ; we record the phase as a
        // zero-cost completion (the elapsed time lives in the
        // serve_fut path, not here).
        shutdown_seq.run_drain_rpc(async {}).await;

        // Phase 2 — flush AuditEmitter. Drains in-flight HMAC
        // sign+persist tasks. Bounded to phase 2 timeout
        // (audit_flush_ms ; default 10 s).
        let audit_handles = self.audit_signing.as_ref();
        let audit_flush_budget = self.multi_replica.shutdown_timeouts.audit_flush;
        shutdown_seq
            .run_audit_flush(async move {
                if let Some(handles) = audit_handles {
                    handles.sink.drain_pending(audit_flush_budget).await;
                }
            })
            .await;

        // Phase 3 — close pg pool + flush OTel. Pre-D18 v0.1.19,
        // these are no-ops under the Memory catalog backend ;
        // D18-bis will close the actual `sqlx::PgPool` here.
        shutdown_seq.run_pg_close(async {}).await;

        let shutdown_outcome = shutdown_seq.finish();
        if !shutdown_outcome.all_clean {
            tracing::warn!(
                target: "efe.shutdown",
                phases = ?shutdown_outcome.phases,
                total_ms = shutdown_outcome.total.as_millis() as u64,
                "shutdown sequence exceeded one or more phase budgets — \
                 check efe_shutdown_phase_seconds histograms"
            );
        } else {
            tracing::info!(
                target: "efe.shutdown",
                total_ms = shutdown_outcome.total.as_millis() as u64,
                "shutdown sequence clean across all 3 phases"
            );
        }

        outcome
    }
}

// =====================================================================
// Sprint D18 — Leader election loop
// =====================================================================

/// **Audit fix D18-A5** — leader-election orchestrator. Tries to
/// acquire the lease at boot, then renews on `renew_every` cadence
/// (typically half the lease duration). Exits cleanly when
/// `shutdown.notified()` fires. Lease drop on exit is best-effort.
async fn run_leader_election_loop(
    lock: Arc<dyn LeaderLock>,
    renew_every: Duration,
    shutdown: Shutdown,
) {
    // Initial acquisition — best-effort. A failure here means
    // another replica owns the lease ; we'll retry on the next
    // renew tick.
    match lock.try_acquire().await {
        Ok(claim) => {
            tracing::info!(
                target: "efe.multi_replica.leader",
                holder = %claim.holder,
                lease_name = %claim.name,
                lease_duration_s = claim.lease_duration.as_secs(),
                "lease acquired at boot"
            );
        }
        Err(e) => {
            tracing::info!(
                target: "efe.multi_replica.leader",
                error = %e,
                "lease NOT acquired at boot — another replica owns it"
            );
        }
    }

    let mut renewer = tokio::time::interval(renew_every);
    // Skip the immediate first tick so we don't double-renew right
    // after acquisition.
    renewer.tick().await;

    loop {
        tokio::select! {
            () = shutdown.notified() => {
                tracing::info!(
                    target: "efe.multi_replica.leader",
                    "shutdown received — releasing lease (best-effort)"
                );
                // Release is best-effort ; ignore the error.
                let _ = lock.release().await;
                return;
            }
            _ = renewer.tick() => {
                if lock.is_held() {
                    if let Err(e) = lock.renew().await {
                        tracing::warn!(
                            target: "efe.multi_replica.leader",
                            error = %e,
                            "lease renew failed — will attempt re-acquire on next tick"
                        );
                    }
                } else {
                    // We don't hold the lease — try to acquire.
                    if let Ok(claim) = lock.try_acquire().await {
                        tracing::info!(
                            target: "efe.multi_replica.leader",
                            holder = %claim.holder,
                            "lease acquired (handoff from another replica)"
                        );
                    }
                }
            }
        }
    }
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server")
            .field("config", &self.config)
            .field("registry_attached", &self.registry.is_some())
            .field("auth_enabled", &self.auth.is_enabled())
            .field("meta_swarm_enabled", &self.meta_swarm_client.is_some())
            .finish()
    }
}
