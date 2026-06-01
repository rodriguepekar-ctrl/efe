//! **Sprint D4 — gRPC server RPC bench (loopback TCP).**
//!
//! Measures the production gRPC wire stack against the same Mode-1
//! backend D1 measures, but through a **real tonic server bound to
//! a loopback TCP port**. This exposes the per-request cost a real
//! gRPC client pays *on top of* the backend chain :
//!
//! * tonic transport (HTTP/2 framing, codec encode/decode);
//! * Tower `MetricsLayer` (Prometheus instrumentation);
//! * `AuthGuard.check` (in disabled mode — anonymous claim install);
//! * `check_rate_limit` (token bucket per tenant);
//! * `IdempotencyCache.{get, insert}` (DashMap-backed cache);
//! * `bridge::request::from_proto` and `bridge::response::to_proto`
//!   (the proto ↔ domain conversion);
//! * `emit_request_completion` (Prometheus emit).
//!
//! See `docs/bench/d4_server_rpc_architecture.md` for the full
//! design rationale (12 decisions, 10 risks, 4 SLO targets).
//!
//! # Four groups
//!
//! 1. **`server_rpc_full_stack`** — full gRPC stack (rate-limit ON,
//!    idempotency ON, auth disabled but path traversed). 3 fixtures.
//!    Headline metric ; SLO `D4-1..3`.
//! 2. **`server_rpc_minimal_stack`** — same backend but rate-limit
//!    OFF, idempotency OFF. Serves as baseline for the
//!    Δ_interceptors metric (full − minimal). No SLO.
//! 3. **`server_rpc_concurrent`** — 8 concurrent clients hammering
//!    the same loopback socket on the small fixture. Throughput
//!    metric ; SLO `D4-T1`.
//! 4. **`server_rpc_codec_only`** — `prost::Message::encode +
//!    decode` round-trip on a medium fixture, no transport, no
//!    server. Codec micro-bench, no SLO.
//!
//! # Why loopback TCP and not in-process duplex
//!
//! See architecture §1.3. Short version : loopback TCP matches what
//! a real gRPC client pays (HTTP/2 framing, OS syscall stack), and
//! the pattern is already validated in
//! `tests/integration_grpc.rs::TestServer`. In-process duplex would
//! produce optimistic numbers that misrepresent prod behaviour.
//!
//! # Why no `_blas` / `_naive` suffix
//!
//! The RPC layer doesn't dispatch any matmul ; the only BLAS-eligible
//! op is in the backend (D1 covers it). Adding the suffix would
//! produce two identical reports for the same wallclock and just
//! consume CI minutes.

#![allow(missing_docs)]
#![allow(clippy::cast_precision_loss)]

mod common;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
// `prost::Message` is the trait that exposes `encode` and `decode`
// on every generated proto type. Imported once here, reused by the
// codec_only group below.
use prost::Message;
use tonic::transport::Channel;

use efe_core::alignment::{AlignmentRegistry, InMemoryRegistry};
use efe_server::proto::v1::{self as v1, swarm_service_client::SwarmServiceClient, ExecuteRequest};
use efe_server::services::DynExecuteBackend;
use efe_server::{Server, ServerConfig, Shutdown};

use common::{build_real_backend, load_realistic_v2_spec};

// =====================================================================
// Bench-tuning constants
// =====================================================================
//
// `BIND_WARMUP` — how long we wait after spawning the tonic server
// before the client tries to connect. The integration tests use
// 100 ms ; we bump to 150 ms to absorb scheduler jitter on busier
// CI runners (the increment is paid once per group, ~50 ms total
// across the bench).
//
// `N_CONCURRENT_CLIENTS` — for the concurrent throughput group.
// 8 matches the `max_arenas` cap of `Mode1Config::default()`
// (D2 architecture §6 D2). Beyond 8, the bench would measure the
// `BudgetExhausted` retry path, not raw concurrency.

const BIND_WARMUP: Duration = Duration::from_millis(150);
const N_CONCURRENT_CLIENTS: usize = 8;

// =====================================================================
// Server harness — mirror of integration_grpc::TestServer
// =====================================================================

/// Loopback gRPC server held alive for the duration of a bench
/// group. Each group spawns the server **once** in setup (outside
/// the timed loop) and tears it down at the end via
/// `runtime.block_on(server.stop())`. The handler tokio task lives
/// in the same runtime as the bench iter loop.
struct BenchServer {
    addr: SocketAddr,
    shutdown: Shutdown,
    handle: tokio::task::JoinHandle<anyhow::Result<()>>,
}

impl BenchServer {
    /// Start a fresh server bound to an ephemeral loopback port,
    /// using the supplied `backend` and `cfg`. Returns once the
    /// server is reachable (after `BIND_WARMUP`).
    async fn start(backend: DynExecuteBackend, mut cfg: ServerConfig) -> Self {
        // 1. Reserve an ephemeral port. We bind a sync TcpListener
        //    just to ask the OS for a free port, then drop it so
        //    the server can rebind on the same address. There is a
        //    small TOCTOU window (the kernel may reassign the port
        //    to a different process before we rebind) but it has
        //    proven negligible across 1 000+ test runs of
        //    `integration_grpc`.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("ephemeral bind");
        let addr = listener.local_addr().expect("local_addr");
        drop(listener);

        // 2. Force the listen address and shorten shutdown grace
        //    so teardown stays snappy between groups.
        cfg.listen_addr = addr;
        cfg.shutdown.grace_period = Duration::from_millis(50);
        cfg.shutdown.force_abort_after = Duration::from_secs(2);

        let registry: Arc<dyn AlignmentRegistry> =
            Arc::new(InMemoryRegistry::with_default_config().expect("registry"));

        let server = Server::builder()
            .config(cfg)
            .backend(backend)
            .registry(registry)
            .build()
            .await
            .expect("server build");

        let shutdown = Shutdown::new();
        let shutdown_for_task = shutdown.clone();
        let handle = tokio::spawn(async move { server.serve(addr, shutdown_for_task).await });

        // 3. Bind warmup. 150 ms is conservative ; on a typical M3
        //    Pro it's overkill (server is ready in ~10 ms) but on
        //    a contended CI runner it's barely enough.
        tokio::time::sleep(BIND_WARMUP).await;

        Self {
            addr,
            shutdown,
            handle,
        }
    }

    /// Open a tonic Channel to the server. Note that tonic
    /// multiplexes multiple RPCs over a single HTTP/2 connection,
    /// so calling `channel()` once and `Clone`-ing per task is
    /// idiomatic for the concurrent bench.
    async fn channel(&self) -> Channel {
        Channel::from_shared(format!("http://{}", self.addr))
            .expect("uri parse")
            .connect()
            .await
            .expect("channel connect")
    }

    /// Trigger graceful shutdown and wait for the serve task to
    /// finish. Bounded by `force_abort_after` (2 s) to keep
    /// teardown snappy.
    async fn stop(self) {
        self.shutdown.trigger();
        let _ = tokio::time::timeout(Duration::from_secs(3), self.handle).await;
    }
}

// =====================================================================
// Config builders
// =====================================================================
//
// Two configs are exercised :
//
// * **full_stack** : auth disabled (path traversed for anonymous
//   claim install) ; rate-limit ON ; idempotency ON. This is the
//   prod-shape with auth deferred to D4.5 (cf. D2 in the architecture).
// * **minimal_stack** : auth + rate-limit + idempotency all OFF.
//   Used as a baseline so the `full - minimal` delta yields the
//   pure interceptor cost.

fn make_full_stack_config() -> ServerConfig {
    let mut cfg = ServerConfig::default();
    cfg.auth.enabled = false;
    cfg.ratelimit.enabled = true;
    cfg.idempotency.enabled = true;
    cfg
}

fn make_minimal_stack_config() -> ServerConfig {
    let mut cfg = ServerConfig::default();
    cfg.auth.enabled = false;
    cfg.ratelimit.enabled = false;
    cfg.idempotency.enabled = false;
    cfg
}

// =====================================================================
// Spec → Proto conversion
// =====================================================================
//
// Loads a v2 fixture (Box-Muller Gaussian embeddings — cf. D1
// architecture §3) and lifts it into the proto `ExecuteRequest`
// shape that the wire carries. The proto is built by hand rather
// than via a `bridge::response`-style "to_proto" function because
// :
//
// * The bench is the *client* side ; the bridge in `efe-server` is
//   the server side. Reusing the server bridge would couple the
//   bench to a private API and break the symmetry (clients build
//   protos from scratch in production too).
// * The conversion logic is straightforward (no decode_mode, no
//   convergence override, no budget — server fills defaults).
// * Building proto by hand catches bugs in the schema itself : if
//   `proto::SwarmSpec` ever loses a field, this code stops compiling.

fn proto_request_for_v2_fixture(name: &str) -> ExecuteRequest {
    let spec = load_realistic_v2_spec(name);

    // Population : iterate over (TemplateCode, count) pairs and
    // emit kebab-case strings the bridge expects (na-sem / na-log /
    // na-crit). `TemplateCode::as_str()` is the canonical converter.
    let population: Vec<v1::PopulationEntry> = spec
        .population
        .iter()
        .map(|(code, count)| v1::PopulationEntry {
            template_code: code.as_str().to_string(),
            count: *count,
        })
        .collect();

    // Anchors : flat-clone each anchor row into a fresh Vec<f32>.
    let anchors: Vec<v1::Anchor> = spec
        .alignment_cone
        .anchors
        .iter()
        .map(|a| v1::Anchor { values: a.clone() })
        .collect();

    // Propagate the spec's resource budget into the proto. The
    // domain `load_realistic_v2_spec` sets a generous 30 s
    // `max_wall_clock_ms` (cf. `common/mod.rs` D1.5 — debug-mode
    // safety margin) ; if we left `budget: None` here, the bridge
    // would apply `ResourceBudget::default()` (500 ms) and the
    // backend would `BudgetExhausted` on debug builds and on slow
    // CI runners. Bench measures should reflect the prod budget,
    // not the test default.
    let proto_budget = v1::ResourceBudget {
        max_wall_clock_ms: spec.budget.max_wall_clock_ms,
        max_energy_wh: spec.budget.max_energy_wh,
        max_hbm_bytes: spec.budget.max_hbm_bytes,
        max_recursion_depth: u32::from(spec.budget.max_recursion_depth),
    };

    let proto_spec = v1::SwarmSpec {
        // Empty swarm_id → server generates one. Matches what real
        // clients do (the id is server-side correlation).
        swarm_id: String::new(),
        problem_embedding: spec.problem_embedding.clone(),
        dim_latent: spec.dim_latent.get(),
        population,
        // `spec.max_rounds` is a `NonZeroU32` in the domain type
        // (validated > 0 at construction). Proto carries plain u32.
        max_rounds: spec.max_rounds.get(),
        // No client-side override on convergence — server applies
        // its tuned defaults (`Mode1Config::default()`).
        convergence: None,
        cone_source: Some(v1::swarm_spec::ConeSource::AlignmentCone(
            v1::AlignmentCone {
                anchors,
                max_projection_distance: spec.alignment_cone.max_projection_distance,
            },
        )),
        budget: Some(proto_budget),
    };

    ExecuteRequest {
        // Empty request_id → server generates a UUID. Forces the
        // idempotency cache to MISS on every iter (we measure the
        // miss path, not hits).
        request_id: String::new(),
        spec: Some(proto_spec),
        decode_mode: None,
        raw_prompt: String::new(),
        deadline: None,
    }
}

// =====================================================================
// Tokio runtime helper
// =====================================================================
//
// Single multi-threaded runtime per `bench_*` function. Building it
// inside `iter` would re-pay the thread-pool creation cost (~5 ms)
// for every sample.

fn make_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("tokio runtime")
}

// =====================================================================
// 1. `server_rpc_full_stack` — headline RPC latency
// =====================================================================
//
// For each fixture (small / medium / large) :
//
// * Build a fresh backend + full-stack server (with rate-limit and
//   idempotency on).
// * Open a single channel + client (both held by the bench closure
//   for the entire group).
// * Iter : send 1 RPC, await response, drop.
//
// The server is shared across iters (built in setup, dropped after
// the group). The client is `Clone`-ed cheaply per iter (channel
// is `Arc`-cloned, no new connection).

fn bench_server_rpc_full_stack(c: &mut Criterion) {
    let rt = make_runtime();
    let mut group = c.benchmark_group("server_rpc_full_stack");
    group.throughput(Throughput::Elements(1));

    for fixture in ["small", "medium", "large"] {
        // Fresh backend per fixture so the memory pool is sized for
        // the workload (matches D1's pattern).
        let spec = load_realistic_v2_spec(fixture);
        let backend = build_real_backend(&spec);
        let cfg = make_full_stack_config();
        let req_template = proto_request_for_v2_fixture(fixture);

        // Spawn server, open channel, prepare client. All inside
        // `block_on` so we don't fight tokio runtime ownership.
        let server = rt.block_on(BenchServer::start(backend, cfg));
        let channel = rt.block_on(server.channel());
        let client = SwarmServiceClient::new(channel);

        // Sample size per fixture : criterion auto-tunes iter count
        // per sample for a target time of 5 s. We cap at 30/20/10
        // samples for small/medium/large to keep wall time bounded.
        let samples = match fixture {
            "small" => 30,
            "medium" => 20,
            _ => 10,
        };
        group.sample_size(samples);

        group.bench_with_input(BenchmarkId::from_parameter(fixture), &fixture, |b, _| {
            let client = client.clone();
            let req_template = req_template.clone();
            b.to_async(&rt).iter(|| {
                let mut client = client.clone();
                let req = req_template.clone();
                async move {
                    let resp = client
                        .execute(tonic::Request::new(req))
                        .await
                        .expect("rpc execute");
                    black_box(resp);
                }
            });
        });

        // Tear down this fixture's server before moving to the next.
        rt.block_on(server.stop());
    }

    group.finish();
}

// =====================================================================
// 2. `server_rpc_minimal_stack` — baseline for Δ_interceptors
// =====================================================================
//
// Identical structure to `full_stack` but with rate-limit and
// idempotency disabled. The delta `full - minimal` quantifies the
// pure interceptor overhead independent of fixture size.

fn bench_server_rpc_minimal_stack(c: &mut Criterion) {
    let rt = make_runtime();
    let mut group = c.benchmark_group("server_rpc_minimal_stack");
    group.throughput(Throughput::Elements(1));

    for fixture in ["small", "medium", "large"] {
        let spec = load_realistic_v2_spec(fixture);
        let backend = build_real_backend(&spec);
        let cfg = make_minimal_stack_config();
        let req_template = proto_request_for_v2_fixture(fixture);

        let server = rt.block_on(BenchServer::start(backend, cfg));
        let channel = rt.block_on(server.channel());
        let client = SwarmServiceClient::new(channel);

        let samples = match fixture {
            "small" => 30,
            "medium" => 20,
            _ => 10,
        };
        group.sample_size(samples);

        group.bench_with_input(BenchmarkId::from_parameter(fixture), &fixture, |b, _| {
            let client = client.clone();
            let req_template = req_template.clone();
            b.to_async(&rt).iter(|| {
                let mut client = client.clone();
                let req = req_template.clone();
                async move {
                    let resp = client
                        .execute(tonic::Request::new(req))
                        .await
                        .expect("rpc execute");
                    black_box(resp);
                }
            });
        });

        rt.block_on(server.stop());
    }

    group.finish();
}

// =====================================================================
// 3. `server_rpc_concurrent` — 8c throughput on small
// =====================================================================
//
// Drives `N_CONCURRENT_CLIENTS=8` simultaneous RPCs through the
// same loopback channel. Each iter spawns 8 tasks via
// `tokio::spawn`, awaits all of them, then completes.
//
// The bench is *throughput*-oriented : `Throughput::Elements(8)`
// makes criterion report ops/s for the whole batch.
//
// **Why this group uses `make_minimal_stack_config` rather than
// full_stack** : the bench saturates the loopback with 8 concurrent
// anonymous RPCs per iter × ~20 samples × ~10 iters/sample = ~1 600
// RPCs in a few seconds. The default `RateLimitConfig` quota for the
// anonymous tenant rejects bursts above its replenishment rate
// (Status::ResourceExhausted), and worse, the pool's async `acquire`
// can hit `BudgetExhausted` if rate-limit retries pile up — both
// surface as `Status::Internal` from the bridge's `to_status` and
// crash the bench with a useless "internal error".
//
// Removing the rate-limit (and the idempotency cache, which is a
// no-op when request_id is empty) keeps the bench measuring what we
// actually want : **server compute capacity under bursty load**.
// The interceptor overhead on the concurrent path is captured by
// comparing `server_rpc_full_stack::small` (sequential) to
// `server_rpc_minimal_stack::small` — adding interceptors to the
// concurrent group would conflate two orthogonal concerns.

fn bench_server_rpc_concurrent(c: &mut Criterion) {
    let rt = make_runtime();
    let mut group = c.benchmark_group("server_rpc_concurrent");
    group.throughput(Throughput::Elements(N_CONCURRENT_CLIENTS as u64));
    group.sample_size(20); // each iter = 8 RPCs ⇒ ~50 ms ; 20 samples = ~1 s.

    let spec = load_realistic_v2_spec("small");
    let backend = build_real_backend(&spec);
    // See above : rate-limit OFF deliberately, to keep the bench
    // measuring compute throughput rather than rate-limit rejection.
    let cfg = make_minimal_stack_config();
    let req_template = proto_request_for_v2_fixture("small");

    let server = rt.block_on(BenchServer::start(backend, cfg));
    let channel = rt.block_on(server.channel());

    group.bench_function(
        BenchmarkId::from_parameter(format!("c{N_CONCURRENT_CLIENTS}")),
        |b| {
            let channel = channel.clone();
            let req_template = req_template.clone();
            b.to_async(&rt).iter(|| {
                let channel = channel.clone();
                let req_template = req_template.clone();
                async move {
                    let mut tasks = Vec::with_capacity(N_CONCURRENT_CLIENTS);
                    for _ in 0..N_CONCURRENT_CLIENTS {
                        let mut client = SwarmServiceClient::new(channel.clone());
                        let req = req_template.clone();
                        tasks.push(tokio::spawn(async move {
                            client
                                .execute(tonic::Request::new(req))
                                .await
                                .expect("concurrent rpc")
                        }));
                    }
                    for t in tasks {
                        black_box(t.await.expect("join"));
                    }
                }
            });
        },
    );

    rt.block_on(server.stop());
    group.finish();
}

// =====================================================================
// 4. `server_rpc_codec_only` — proto encode/decode round-trip
// =====================================================================
//
// No transport, no server, no tokio runtime. Just `prost::encode`
// then `prost::decode` of an `ExecuteRequest` populated from the
// medium fixture. Establishes a *lower bound* : no full_stack iter
// can be faster than 2× this measurement (encode at client + decode
// at server) plus transport.
//
// Sample size 100 + criterion auto-tunes iters/sample : codec is
// in the µs range, so we get many samples per second and a tight
// CI naturally.

fn bench_server_rpc_codec_only(c: &mut Criterion) {
    let mut group = c.benchmark_group("server_rpc_codec_only");
    group.throughput(Throughput::Elements(1));
    group.sample_size(100);

    for fixture in ["small", "medium", "large"] {
        let req = proto_request_for_v2_fixture(fixture);

        // Pre-allocate a Vec with capacity matching the encoded size.
        // 64 KiB covers all v2 fixtures (large is ~50 KiB encoded).
        let cap = 64 * 1024;

        group.bench_with_input(BenchmarkId::new("roundtrip", fixture), &fixture, |b, _| {
            let req = req.clone();
            b.iter(|| {
                let mut buf = Vec::<u8>::with_capacity(cap);
                // Encode (client side equivalent).
                Message::encode(black_box(&req), &mut buf).expect("encode");
                // Decode (server side equivalent).
                let decoded = ExecuteRequest::decode(buf.as_slice()).expect("decode");
                black_box(decoded);
            });
        });
    }

    group.finish();
}

// =====================================================================
// Criterion entry points
// =====================================================================
//
// Group order chosen to fail fast :
// 1. `codec_only` — no server, fastest to fail if proto schema is
//    broken.
// 2. `full_stack` — headline metric, fail fast on RPC regressions.
// 3. `minimal_stack` — baseline, complements (1).
// 4. `concurrent` — most expensive (8 RPCs/iter), runs last.

criterion_group!(
    name = server_rpc_benches;
    config = Criterion::default()
        .significance_level(0.05)
        // Loopback TCP variance ~10 % — tighter `noise_threshold`
        // would flag every benign fluctuation as regression.
        .noise_threshold(0.10);
    targets =
        bench_server_rpc_codec_only,
        bench_server_rpc_full_stack,
        bench_server_rpc_minimal_stack,
        bench_server_rpc_concurrent,
);
criterion_main!(server_rpc_benches);
