//! Phase-2 PR #11 — integration tests for the
//! `EFE_META_SWARM_ENABLED` plumbing.
//!
//! Scope : config parsing, validation, `MetaSwarmClient` construction
//! error paths, **and** `ServerBuilder::build` end-to-end wiring
//! (the runtime-level plumbing-only doctrine).  No real meta-swarm
//! controller is spawned ; the happy-path gRPC traffic is exercised
//! by PR #10's `integration_multiprocess_*` suite on the controller
//! side.
//!
//! Tests covered :
//!   * `disabled_meta_swarm_config_matches_default`                       (AC8)
//!   * `enabled_meta_swarm_config_validates_with_well_formed_endpoint`    (AC6 / AC9)
//!   * `enabled_meta_swarm_config_rejects_malformed_endpoint`             (AC6 / AC10)
//!   * `client_connect_fails_fast_against_closed_port`                    (AC6 / AC10)
//!   * `server_builder_disabled_yields_none_meta_swarm_client`            (AC8 / AC11)
//!   * `server_builder_enabled_unreachable_controller_fails_fast`         (AC10 / AC12)

use std::env::VarError;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use efe_core::{Request as DomainRequest, Response as DomainResponse, Result as EfeResult};
use efe_meta_swarm_proto::controller::{MetaSwarmService, MetaSwarmServiceServer};
use efe_meta_swarm_proto::meta_swarm::{
    HealthRequest, HealthResponse, InfoRequest, InfoResponse, IterateChunk, IterateRequest,
    IterateResponse, ReloadConfigRequest, ReloadConfigResponse, ResetRequest, ResetResponse,
    StepOnceRequest, StepOnceResponse,
};
use efe_server::services::ExecuteBackend;
use efe_server::{
    ConfigError, MetaSwarmClient, MetaSwarmClientError, MetaSwarmMetrics, Server, ServerConfig,
};
use prometheus::Registry;

/// Minimal stub backend so we can call `ServerBuilder::build()`
/// without dragging Phase-1's Mode-1 SwarmController into the test
/// crate.  The `execute` method is never invoked in these tests ;
/// they only exercise the builder's plumbing wiring.
struct StubBackend;

#[async_trait]
impl ExecuteBackend for StubBackend {
    async fn execute(&self, _req: DomainRequest) -> EfeResult<DomainResponse> {
        Err(efe_core::EfeError::InvalidConfig(
            "StubBackend is not meant to be invoked in PR #11 plumbing tests".into(),
        ))
    }
}

// ─────────────────────────────────────────────────────────────────────
// Mock `MetaSwarmService` gRPC server.
//
// Lets us exercise the **happy path** (Health returns SERVING) and
// the **unhealthy controller** path (Health returns NOT_SERVING)
// without spawning the real `efe-meta-swarm-server` controller
// binary.  Every method except `health` returns `Unimplemented` —
// PR #11 only consumes `Health`.
// ─────────────────────────────────────────────────────────────────────

struct MockMetaSwarmService {
    /// Status code the mock returns in `HealthResponse.status`.
    /// `1` (STATUS_SERVING) for happy path ;
    /// `2` (STATUS_NOT_SERVING) for the unhealthy path.
    health_status: i32,
}

#[tonic::async_trait]
impl MetaSwarmService for MockMetaSwarmService {
    type IterateStreamStream = futures::stream::Empty<Result<IterateChunk, tonic::Status>>;

    async fn iterate(
        &self,
        _req: tonic::Request<IterateRequest>,
    ) -> Result<tonic::Response<IterateResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("mock"))
    }

    async fn iterate_stream(
        &self,
        _req: tonic::Request<IterateRequest>,
    ) -> Result<tonic::Response<Self::IterateStreamStream>, tonic::Status> {
        Err(tonic::Status::unimplemented("mock"))
    }

    async fn step_once(
        &self,
        _req: tonic::Request<StepOnceRequest>,
    ) -> Result<tonic::Response<StepOnceResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("mock"))
    }

    async fn reset(
        &self,
        _req: tonic::Request<ResetRequest>,
    ) -> Result<tonic::Response<ResetResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("mock"))
    }

    async fn reload_config(
        &self,
        _req: tonic::Request<ReloadConfigRequest>,
    ) -> Result<tonic::Response<ReloadConfigResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("mock"))
    }

    async fn health(
        &self,
        _req: tonic::Request<HealthRequest>,
    ) -> Result<tonic::Response<HealthResponse>, tonic::Status> {
        Ok(tonic::Response::new(HealthResponse {
            status: self.health_status,
            inflight: 0,
            reload_gen: 0,
        }))
    }

    async fn info(
        &self,
        _req: tonic::Request<InfoRequest>,
    ) -> Result<tonic::Response<InfoResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("mock"))
    }
}

/// Spawn an in-process mock `MetaSwarmService` server.  Returns the
/// bound address + a JoinHandle whose drop terminates the server's
/// tokio task (when the test's runtime tears down).
///
/// Bind pattern : `127.0.0.1:0` (OS-chosen port) → drop the listener →
/// re-bind via tonic.  The TOCTOU window is microseconds on localhost.
async fn spawn_mock_controller(health_status: i32) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    drop(listener);

    let svc = MockMetaSwarmService { health_status };
    let handle = tokio::spawn(async move {
        let _ = tonic::transport::Server::builder()
            .add_service(MetaSwarmServiceServer::new(svc))
            .serve(addr)
            .await;
    });

    // Wait up to ~ 1 s for the bind to land.
    for _ in 0..40 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return (addr, handle);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("mock controller did not come up at {addr}");
}

/// Build an env lookup closure from a `&[(&str, &str)]` slice.  Keeps
/// tests hermetic (no `std::env::set_var`).
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

/// Allocate a localhost port then immediately drop the listener.
/// The port is guaranteed closed (ECONNREFUSED) for the brief
/// window before any other process binds it.  Same bind-drop-rebind
/// pattern as PR #10's `ProcessHarness::allocate_port`.
async fn closed_port() -> std::net::SocketAddr {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = l.local_addr().expect("local_addr");
    drop(l);
    addr
}

/// **AC8** — with `EFE_META_SWARM_ENABLED=false` (and no other
/// `EFE_META_SWARM_*` overrides), the parsed config is byte-identical
/// to the default.  This is the load-bearing assertion for the
/// "plumbing-only" doctrine (PR #11 architecture § 1.3) : flipping
/// the flag off must produce no observable difference from pre-PR-#11
/// deployments.
#[test]
fn disabled_meta_swarm_config_matches_default() {
    let cfg = ServerConfig::from_env_with(env(&[("EFE_META_SWARM_ENABLED", "false")]))
        .expect("from_env ok");
    let d = ServerConfig::default();
    assert_eq!(cfg.meta_swarm.enabled, d.meta_swarm.enabled);
    assert_eq!(
        cfg.meta_swarm.controller_endpoint,
        d.meta_swarm.controller_endpoint,
    );
    assert_eq!(cfg.meta_swarm.connect_timeout, d.meta_swarm.connect_timeout,);
    assert_eq!(cfg.meta_swarm.call_timeout, d.meta_swarm.call_timeout);
    assert_eq!(
        cfg.meta_swarm.health_probe_at_startup,
        d.meta_swarm.health_probe_at_startup,
    );
    cfg.validate().expect("disabled config must validate");
}

/// **AC6 / AC9** — with `EFE_META_SWARM_ENABLED=true` and a
/// well-formed endpoint, the parsed config validates.  Note we
/// do **not** require the endpoint to be reachable here ;
/// reachability is checked later, when `MetaSwarmClient::connect`
/// is called at runtime.
#[test]
fn enabled_meta_swarm_config_validates_with_well_formed_endpoint() {
    let cfg = ServerConfig::from_env_with(env(&[
        ("EFE_META_SWARM_ENABLED", "true"),
        (
            "EFE_META_SWARM_CONTROLLER_ENDPOINT",
            "http://10.0.0.5:50080",
        ),
        ("EFE_META_SWARM_CONNECT_TIMEOUT_MS", "3500"),
        ("EFE_META_SWARM_CALL_TIMEOUT_MS", "15000"),
        ("EFE_META_SWARM_HEALTH_PROBE_AT_STARTUP", "false"),
    ]))
    .expect("from_env ok");
    assert!(cfg.meta_swarm.enabled);
    assert_eq!(cfg.meta_swarm.controller_endpoint, "http://10.0.0.5:50080");
    assert_eq!(cfg.meta_swarm.connect_timeout, Duration::from_millis(3500));
    assert_eq!(cfg.meta_swarm.call_timeout, Duration::from_millis(15000));
    assert!(!cfg.meta_swarm.health_probe_at_startup);
    cfg.validate()
        .expect("enabled config with valid endpoint validates");
}

/// **AC6 / AC10** — with `EFE_META_SWARM_ENABLED=true` and a
/// malformed endpoint URI, `validate()` rejects the config with a
/// `ConfigError::Invalid` whose `field` points at the offending
/// member.  This catches typos before any gRPC machinery boots
/// (PR #11 AD12).
#[test]
fn enabled_meta_swarm_config_rejects_malformed_endpoint() {
    let cfg = ServerConfig::from_env_with(env(&[
        ("EFE_META_SWARM_ENABLED", "true"),
        (
            "EFE_META_SWARM_CONTROLLER_ENDPOINT",
            "not://valid uri space",
        ),
    ]))
    .expect("from_env succeeds (lazy parse)");
    let err = cfg.validate().expect_err("malformed endpoint must fail");
    match err {
        ConfigError::Invalid { field, msg } => {
            assert_eq!(field, "meta_swarm.controller_endpoint");
            assert!(
                msg.contains("not://valid uri space"),
                "diagnostic must echo the offending value : {msg}",
            );
        }
        other => panic!("expected ConfigError::Invalid, got {other:?}"),
    }
}

/// **AC6 / AC10** — `MetaSwarmClient::connect` against a closed
/// localhost port returns `MetaSwarmClientError::ConnectFailed`
/// within `connect_timeout`.  This is the fail-fast doctrine
/// (PR #11 AD3) end-to-end : `efe-server` boot would propagate
/// this error and exit 1 before serving any user request.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_connect_fails_fast_against_closed_port() {
    let addr = closed_port().await;
    // `MetaSwarmConfig` is `#[non_exhaustive]` ; construct via
    // `default()` then mutate so external test binaries don't
    // break on future field additions.
    let mut cfg = efe_server::MetaSwarmConfig::default();
    cfg.enabled = true;
    cfg.controller_endpoint = format!("http://{addr}");
    cfg.connect_timeout = Duration::from_millis(500);
    cfg.call_timeout = Duration::from_secs(20);
    cfg.health_probe_at_startup = true;
    let metrics = MetaSwarmMetrics::register(&Registry::new()).expect("metrics");
    let start = std::time::Instant::now();
    let err = MetaSwarmClient::connect(&cfg, metrics)
        .await
        .expect_err("must fail against closed port");
    let elapsed = start.elapsed();

    // Either : ConnectFailed (TCP refused) — most common —
    // or : HealthRpcFailed — if connect_lazy was used somewhere.
    // We accept both as legitimate "the controller isn't there".
    assert!(
        matches!(
            err,
            MetaSwarmClientError::ConnectFailed(_) | MetaSwarmClientError::HealthRpcFailed(_)
        ),
        "unexpected error variant : {err:?}",
    );

    // Fail-fast budget : must return well before
    // 2 × connect_timeout = 1 s.  We give 3 s headroom on slow CI.
    assert!(
        elapsed < Duration::from_secs(3),
        "fail-fast took {elapsed:?} > 3 s budget",
    );
}

/// **AC8 / AC11** — `ServerBuilder::build()` with the meta-swarm
/// subsystem disabled produces a `Server` whose `meta_swarm_client`
/// accessor returns `None`.  This is the runtime-level plumbing-only
/// guarantee : with `enabled=false`, the subsystem is never even
/// constructed, and pre-PR-#11 deployments remain unaffected.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_builder_disabled_yields_none_meta_swarm_client() {
    let mut cfg = ServerConfig::default();
    // Use an explicit port so listen_addr.port() != 0 (validate()
    // rejects port 0).  ServerConfig::default() already has 50051.
    assert_eq!(cfg.listen_addr.port(), 50051);
    assert!(!cfg.meta_swarm.enabled, "default must be off");
    // Force-disable just to be explicit.
    cfg.meta_swarm.enabled = false;

    let server = Server::builder()
        .config(cfg)
        .backend(Arc::new(StubBackend) as Arc<dyn ExecuteBackend>)
        .build()
        .await
        .expect("build succeeds with meta-swarm disabled");

    assert!(
        server.meta_swarm_client().is_none(),
        "meta_swarm_client must be None when subsystem is disabled",
    );
}

/// **AC10 / AC12** — `ServerBuilder::build()` with the meta-swarm
/// subsystem enabled and a closed-port controller returns an error
/// (fail-fast doctrine, PR #11 AD3).  `main.rs` wraps this with
/// `anyhow::Context` and exits 1 ; operators discover the misconfig
/// at boot instead of after the first user request.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_builder_enabled_unreachable_controller_fails_fast() {
    let addr = closed_port().await;
    let mut cfg = ServerConfig::default();
    cfg.meta_swarm.enabled = true;
    cfg.meta_swarm.controller_endpoint = format!("http://{addr}");
    cfg.meta_swarm.connect_timeout = Duration::from_millis(500);
    cfg.meta_swarm.call_timeout = Duration::from_secs(20);
    cfg.meta_swarm.health_probe_at_startup = true;

    let start = std::time::Instant::now();
    let err = Server::builder()
        .config(cfg)
        .backend(Arc::new(StubBackend) as Arc<dyn ExecuteBackend>)
        .build()
        .await
        .expect_err("build must fail-fast when controller is unreachable");
    let elapsed = start.elapsed();

    // The anyhow error chain must mention the meta-swarm subsystem
    // so operators can grep the failure cause from logs.
    let chain = format!("{err:#}");
    assert!(
        chain.contains("meta-swarm connect"),
        "error chain must reference 'meta-swarm connect' for grep-ability ; got : {chain}",
    );

    // Fail-fast budget : ≤ 3 s on slow CI (connect_timeout = 500 ms +
    // generous slack).
    assert!(
        elapsed < Duration::from_secs(3),
        "fail-fast took {elapsed:?} > 3 s budget",
    );
}

/// **Architecture § 9.4 test 3** — `MetaSwarmClient::health_probe`
/// returns `Ok(())` when the (mocked) controller answers
/// `STATUS_SERVING (1)`.  Also asserts that the `health_probes_total`
/// counter increments and `health_probe_errors_total` stays at zero.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_probe_returns_ok_on_serving_mock_controller() {
    // 1 = STATUS_SERVING in the proto enum.
    let (addr, _h) = spawn_mock_controller(1).await;

    let mut cfg = efe_server::MetaSwarmConfig::default();
    cfg.enabled = true;
    cfg.controller_endpoint = format!("http://{addr}");
    cfg.health_probe_at_startup = true;
    cfg.connect_timeout = Duration::from_secs(2);
    cfg.call_timeout = Duration::from_secs(5);

    let metrics = MetaSwarmMetrics::register(&Registry::new()).expect("metrics");
    let client = MetaSwarmClient::connect(&cfg, metrics.clone())
        .await
        .expect("probe should succeed against a SERVING mock");

    assert_eq!(client.endpoint(), format!("http://{addr}"));
    assert!(
        metrics.health_probes_total.get() as u64 >= 1,
        "health_probes_total must increment ; got {}",
        metrics.health_probes_total.get(),
    );
    assert_eq!(
        metrics.health_probe_errors_total.get() as u64,
        0,
        "errors_total must stay at zero on SERVING reply",
    );
}

/// **Architecture § 9.4 test 4** — `MetaSwarmClient::connect` returns
/// `UnhealthyController(status)` when the (mocked) controller answers
/// with a non-SERVING status code (e.g. `2 = NOT_SERVING`).  This
/// exercises the otherwise-untested `UnhealthyController` error
/// variant and the `health_probe_errors_total` increment.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_probe_returns_err_on_not_serving_mock_controller() {
    // 2 = STATUS_NOT_SERVING.
    let (addr, _h) = spawn_mock_controller(2).await;

    let mut cfg = efe_server::MetaSwarmConfig::default();
    cfg.enabled = true;
    cfg.controller_endpoint = format!("http://{addr}");
    cfg.health_probe_at_startup = true;
    cfg.connect_timeout = Duration::from_secs(2);
    cfg.call_timeout = Duration::from_secs(5);

    let metrics = MetaSwarmMetrics::register(&Registry::new()).expect("metrics");
    let err = MetaSwarmClient::connect(&cfg, metrics.clone())
        .await
        .expect_err("probe must fail against a NOT_SERVING mock");

    assert!(
        matches!(err, MetaSwarmClientError::UnhealthyController(2)),
        "expected UnhealthyController(2), got {err:?}",
    );
    assert!(
        metrics.health_probes_total.get() as u64 >= 1,
        "probes_total must increment ; got {}",
        metrics.health_probes_total.get(),
    );
    assert!(
        metrics.health_probe_errors_total.get() as u64 >= 1,
        "errors_total must increment on UnhealthyController ; got {}",
        metrics.health_probe_errors_total.get(),
    );
}

/// **AC11** — when `EFE_META_SWARM_ENABLED=true` AND
/// `EFE_META_SWARM_HEALTH_PROBE_AT_STARTUP=false`, the server boots
/// **even if the controller is unreachable** (the operator
/// deliberately opted out of fail-fast).  The internal `Channel` is
/// built lazily ; the first RPC dispatch (PR #15) will surface any
/// persistent unreachability via `Unavailable`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_builder_enabled_with_probe_disabled_succeeds_against_closed_port() {
    let addr = closed_port().await;
    let mut cfg = ServerConfig::default();
    cfg.meta_swarm.enabled = true;
    cfg.meta_swarm.controller_endpoint = format!("http://{addr}");
    cfg.meta_swarm.health_probe_at_startup = false; // <-- the key contract
    cfg.meta_swarm.connect_timeout = Duration::from_millis(500);
    cfg.meta_swarm.call_timeout = Duration::from_secs(20);

    let server = Server::builder()
        .config(cfg)
        .backend(Arc::new(StubBackend) as Arc<dyn ExecuteBackend>)
        .build()
        .await
        .expect(
            "build MUST succeed with probe_at_startup=false even \
             against a closed-port controller (AC11)",
        );

    assert!(
        server.meta_swarm_client().is_some(),
        "lazy channel must still produce a Some(_) client",
    );
}

/// **AC12** — a registry without `MetaSwarmMetrics::register` having
/// been called contains **zero** `efe_meta_swarm_*` series.  This is
/// the plumbing-silent-when-off contract : a disabled `efe-server`
/// emits no meta-swarm metric noise.
#[test]
fn metrics_registry_has_zero_meta_swarm_series_before_register() {
    let reg = Registry::new();
    let families = reg.gather();
    let count = families
        .iter()
        .filter(|m| m.get_name().starts_with("efe_meta_swarm_"))
        .count();
    assert_eq!(
        count, 0,
        "fresh registry must not contain any efe_meta_swarm_* series ; got {count}",
    );
}

/// **AC13** — after `MetaSwarmMetrics::register`, the registry contains
/// **exactly 3** `efe_meta_swarm_*` series (probes_total, errors_total,
/// duration_seconds).
#[test]
fn metrics_registry_has_three_meta_swarm_series_after_register() {
    let reg = Registry::new();
    let _m = MetaSwarmMetrics::register(&reg).expect("register");
    let families = reg.gather();
    let names: Vec<&str> = families
        .iter()
        .map(|m| m.get_name())
        .filter(|n| n.starts_with("efe_meta_swarm_"))
        .collect();
    assert_eq!(
        names.len(),
        3,
        "expected exactly 3 efe_meta_swarm_* families ; got {names:?}",
    );
    assert!(names.contains(&"efe_meta_swarm_health_probes_total"));
    assert!(names.contains(&"efe_meta_swarm_health_probe_errors_total"));
    assert!(names.contains(&"efe_meta_swarm_health_probe_duration_seconds"));
}

/// **PR #12 § 14.4 / AC22** — when the (mocked) controller answers
/// `STATUS_RELOADING (3)` (the brief window during the SIGHUP-driven
/// manifest swap), the `MetaSwarmClient::health_probe` MUST accept
/// it as a transient state and return `Ok(())`.  Without this
/// relaxation, every `efe-server` boot probe that lands inside the
/// reload window would fail-fast for no reason.
///
/// Architecture spec § 14.2 — "operationally equivalent to 'this
/// controller is fine for your purposes' because in-flight Iterates
/// use their pre-reload snapshot".
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_probe_accepts_reloading_as_transient() {
    // 3 = STATUS_RELOADING in the proto enum.
    let (addr, _h) = spawn_mock_controller(3).await;

    let mut cfg = efe_server::MetaSwarmConfig::default();
    cfg.enabled = true;
    cfg.controller_endpoint = format!("http://{addr}");
    cfg.health_probe_at_startup = true;
    cfg.connect_timeout = Duration::from_secs(2);
    cfg.call_timeout = Duration::from_secs(5);

    let metrics = MetaSwarmMetrics::register(&Registry::new()).expect("metrics");
    let client = MetaSwarmClient::connect(&cfg, metrics.clone())
        .await
        .expect("probe should succeed against a RELOADING mock (PR #12 AD13)");

    assert_eq!(client.endpoint(), format!("http://{addr}"));
    // The probe ran once and counted as a success.
    assert!(
        metrics.health_probes_total.get() as u64 >= 1,
        "health_probes_total must increment ; got {}",
        metrics.health_probes_total.get(),
    );
    assert_eq!(
        metrics.health_probe_errors_total.get() as u64,
        0,
        "errors_total MUST stay at zero — RELOADING is accepted, not an error",
    );
}

/// **PR #12 § 14.4 / AC23** — defence-in-depth pair to the test above.
/// `health_probe` MUST still reject `NOT_SERVING (2)` even after the
/// PR #12 relaxation accepted `RELOADING (3)`.  This is the non-regress
/// guarantee : we only widened the accepted set, we did not soften
/// the rejection semantics for legitimately unhealthy controllers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_probe_still_rejects_not_serving_after_pr12_relaxation() {
    // 2 = STATUS_NOT_SERVING.
    let (addr, _h) = spawn_mock_controller(2).await;

    let mut cfg = efe_server::MetaSwarmConfig::default();
    cfg.enabled = true;
    cfg.controller_endpoint = format!("http://{addr}");
    cfg.health_probe_at_startup = true;
    cfg.connect_timeout = Duration::from_secs(2);
    cfg.call_timeout = Duration::from_secs(5);

    let metrics = MetaSwarmMetrics::register(&Registry::new()).expect("metrics");
    let err = MetaSwarmClient::connect(&cfg, metrics.clone())
        .await
        .expect_err("probe must still fail against a NOT_SERVING mock");

    assert!(
        matches!(err, MetaSwarmClientError::UnhealthyController(2)),
        "expected UnhealthyController(2), got {err:?}",
    );
    assert!(metrics.health_probe_errors_total.get() as u64 >= 1);
}

/// **PR #12 § 14.4** — same defence-in-depth check for the legacy
/// `UNSPECIFIED (0)` status.  Future proto code 0 is reserved but
/// MUST NOT be silently accepted as healthy.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_probe_rejects_status_unspecified() {
    let (addr, _h) = spawn_mock_controller(0).await;

    let mut cfg = efe_server::MetaSwarmConfig::default();
    cfg.enabled = true;
    cfg.controller_endpoint = format!("http://{addr}");
    cfg.health_probe_at_startup = true;
    cfg.connect_timeout = Duration::from_secs(2);
    cfg.call_timeout = Duration::from_secs(5);

    let metrics = MetaSwarmMetrics::register(&Registry::new()).expect("metrics");
    let err = MetaSwarmClient::connect(&cfg, metrics.clone())
        .await
        .expect_err("STATUS_UNSPECIFIED (0) must NOT be accepted as healthy");
    assert!(
        matches!(err, MetaSwarmClientError::UnhealthyController(0)),
        "{err:?}",
    );
}
