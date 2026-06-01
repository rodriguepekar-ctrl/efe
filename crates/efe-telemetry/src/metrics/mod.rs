//! Prometheus metrics: registry, instruments, `/metrics` HTTP sidecar,
//! and the `tower::Layer` consumed by the gRPC server (M10).
//!
//! Sprint **D13** — additive `d13_instruments` module exposes 8
//! observabilité-spécifiques metrics (catalog RPC histograms, audit
//! events counter, alert fires counter, dashboard query duration
//! histogram). Cf. `docs/bench/d13_observability_dashboards_architecture.md`
//! § 5.2.

pub mod d13_instruments;
/// Sprint **D14** — Postgres backend metrics (4 instruments :
/// pool gauges + query latency histogram + tx failures counter +
/// migration version gauge). Cf.
/// `docs/bench/d14_postgres_catalog_backend_architecture.md` § 12.
pub mod d14_instruments;
/// Sprint **D14-bis** — HMAC-chained audit log metrics (6 instruments :
/// signed counter, sign failures counter, chain head gauge, keys
/// active gauge, sign duration histogram, chain break counter).
/// Cf. `docs/bench/d14bis_hmac_audit_chain_architecture.md` § 17.
pub mod d14bis_instruments;
/// Sprint **D15** — OIDC tenant onboarding + IdP integration metrics
/// (7 instruments : tokens_validated counter, jwks_fetch counter,
/// jwks_cache_size gauge, jwks_cache_age_seconds gauge, jwks_stale
/// counter, oidc_issuers_loaded gauge, tenants_total gauge).
/// Cf. `docs/bench/d15_onboarding_idp_architecture.md` § 14.
pub mod d15_instruments;
/// Sprint **D16** — embedder bundling metrics (9 instruments :
/// bundle_load counter + duration, signature/hash verifications,
/// encode counter + duration, tokenizer truncations, active_bundles
/// gauge, resident_bytes gauge). Cf.
/// `docs/bench/d16_embedder_bundling_architecture.md` § 13.
pub mod d16_instruments;
/// Sprint **D13-bis** — OpenTelemetry SDK self-observability metrics
/// (4 instruments : `efe_otel_export_total`,
/// `efe_otel_export_duration_seconds`, `efe_otel_batch_size`,
/// `efe_otel_dropped_total`). Cf.
/// `docs/bench/d13bis_otel_sdk_real_architecture.md` § 12.
pub mod d13bis_instruments;
/// Sprint **D17** — Backup + DR Prometheus instruments (8 :
/// `efe_backup_runs_total`, `efe_backup_duration_seconds`,
/// `efe_backup_size_bytes`, `efe_backup_last_success_timestamp`,
/// `efe_backup_rpo_seconds`, `efe_dr_drill_runs_total`,
/// `efe_dr_drill_duration_seconds`,
/// `efe_dr_drill_last_success_timestamp`). Cf.
/// `docs/bench/d17_backup_dr_architecture.md` § 19.
pub mod d17_instruments;
/// Sprint **D18** — Multi-replica + bench D12 Prometheus instruments
/// (9 : `efe_replica_id`, `efe_idempotency_hits_total`,
/// `efe_idempotency_size`, `efe_ratelimit_decisions_total`,
/// `efe_ratelimit_bucket_available`, `efe_leader_is_leader`,
/// `efe_leader_acquire_attempts_total`, `efe_shutdown_phase_seconds`,
/// `efe_bench_runs_total`). Cf.
/// `docs/bench/d18_multi_replica_bench_d12_architecture.md` § 17.
pub mod d18_instruments;
/// Sprint **D29** — GPU runtime Prometheus instruments (6 :
/// `efe_gpu_kernel_duration_seconds`, `efe_gpu_memory_used_bytes`,
/// `efe_gpu_utilization_ratio`, `efe_gpu_kernel_calls_total`,
/// `efe_gpu_oom_total`, `efe_gpu_nccl_allreduce_seconds`). Cf.
/// `docs/EFE_KERNELS_ARCHITECTURE.md` § 18.
pub mod gpu_instruments;
/// Sprint **D34** (M5 extension) — Nano-agent template instruments
/// (4 : `efe_nano_agent_population_total`,
/// `efe_nano_agent_consensus_weight`,
/// `efe_nano_agent_safety_triggered_total`,
/// `efe_nano_agent_kernel_routed_total`). Cf.
/// `docs/EFE_TELEMETRY_EXTENSION_ARCHITECTURE.md` § 10.3.
pub mod nano_agent_instruments;
/// Sprint **D34** (M5 extension) — Mode-2 backend instruments
/// (5 : `efe_mode2_cache_hit_total`, `efe_mode2_cache_miss_total`,
/// `efe_mode2_router_latency_seconds`, `efe_mode2_anchors_generated`,
/// `efe_mode2_seed_mix_collisions_total`). Cf.
/// `docs/EFE_TELEMETRY_EXTENSION_ARCHITECTURE.md` § 10.4.
pub mod mode2_instruments;
/// Sprint **D37** (M5 extension) — Mode-3 backend instruments
/// (4 : `efe_mode3_subswarms_spawned_total`, `efe_mode3_dispersion`,
/// `efe_mode3_k_clamped_total`, `efe_mode3_aggregation_seconds`).
/// Cf. `docs/EFE_TELEMETRY_EXTENSION_ARCHITECTURE.md` § 11.2.
pub mod mode3_instruments;
/// Sprint **D19 (P1-Foundation)** — Latence opposable instruments
/// (12 histograms par étage + 2 compteurs auxiliaires).
/// Cf. `docs/EFE_P1_SURFACE_ARCHITECTURE.md` § 4.2.
pub mod p1_instruments;
/// Sprint **D19 (P2-Foundation)** — Activation opposable / Self-service
/// instruments (10 instruments couvrant le funnel d'onboarding).
/// Cf. `docs/EFE_P2_SELF_SERVICE_ARCHITECTURE.md` § 4.2.
pub mod p2_instruments;
/// Sprint **Φ-C D26 (Bundle embedder réel)** — instruments
/// d'inférence bundle production-grade (8 instruments couvrant
/// boot, prewarm, recall, taille, inference, verify, revocation).
/// Cf. `docs/EFE_BUNDLE_EMBEDDER_REAL_ARCHITECTURE.md` § 11.4.
pub mod bundle_instruments;
/// Sprint **Φ-C-FFI Completion (D26)** — PyO3 FFI in-process
/// instruments (8 instruments couvrant classify total + duration,
/// GIL held/released, marshal in/out, bundle cache, wheel cold load).
/// Cf. `docs/EFE_PYO3_FFI_COMPLETION_ARCHITECTURE.md` § 12.
pub mod ffi_instruments;
pub mod http_server;
pub mod instruments;
pub mod recorder;
pub mod registry;
pub mod tower_layer;

pub use d13_instruments::register_all_d13;
pub use d14_instruments::register_all_d14;
pub use d14bis_instruments::register_all_d14bis;
pub use d15_instruments::register_all_d15;
pub use d16_instruments::register_all_d16;
pub use d13bis_instruments::register_all_d13bis;
pub use d17_instruments::register_all_d17;
pub use d18_instruments::register_all_d18;
pub use gpu_instruments::register_all_gpu;
pub use nano_agent_instruments::register_all_nano_agent;
pub use mode2_instruments::register_all_mode2;
pub use mode3_instruments::register_all_mode3;
pub use p1_instruments::register_all_p1;
pub use p2_instruments::register_all_p2;
pub use bundle_instruments::register_all_bundle;
pub use ffi_instruments::register_all_ffi;
pub use http_server::{spawn_metrics_server, MetricsServerHandle};
pub use recorder::record_request_completion;
pub use registry::global_registry;
pub use tower_layer::MetricsLayer;
