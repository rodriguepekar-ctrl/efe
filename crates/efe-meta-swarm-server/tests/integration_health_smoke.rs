//! End-to-end smoke test : spawn the controller binary, connect via
//! gRPC, call `Health`, assert `STATUS_SERVING`.
//!
//! This is the **load-bearing pin** for the binary's basic
//! correctness.  If a future PR breaks the startup sequence
//! (e.g. moves the gRPC listener after the metrics exporter), this
//! test turns RED.
//!
//! PR #8 updates : the controller now loads a manifest at startup,
//! so every test must point `EFE_META_SWARM_MANIFEST` at a real file.
//! `Iterate` is now wired ; an empty request returns
//! `FAILED_PRECONDITION` (was `UNIMPLEMENTED` in PR #6).

use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use efe_meta_swarm_proto::controller::MetaSwarmServiceClient;
use efe_meta_swarm_proto::meta_swarm::{HealthRequest, IterateRequest};
use tempfile::NamedTempFile;
use tonic::transport::Endpoint;

const MINIMAL_MANIFEST: &str = "\
version: v0.2.0
meta:
  dim: 16
  safety:
    kind: box
    radius: 10.0
participants:
  - id: supplier-scoring
    dim: 8
    kappa: 0.6
    seed: 11
";

struct ProcessGuard {
    child: Child,
    _manifest: NamedTempFile,
}

impl Drop for ProcessGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn_controller(grpc_port: u16, metrics_port: u16) -> ProcessGuard {
    // Write a minimal valid manifest to a temp file ; PR #8 makes the
    // controller refuse to start without one.
    let mut manifest = NamedTempFile::new().expect("tempfile");
    manifest
        .write_all(MINIMAL_MANIFEST.as_bytes())
        .expect("write manifest");
    let manifest_path = manifest.path().to_path_buf();

    // CARGO_BIN_EXE_controller : cargo sets this for the test target.
    let bin = env!("CARGO_BIN_EXE_controller");
    let child = Command::new(bin)
        .env(
            "EFE_META_SWARM_CONTROLLER_ADDR",
            format!("127.0.0.1:{grpc_port}"),
        )
        .env(
            "EFE_META_SWARM_CONTROLLER_METRICS",
            format!("127.0.0.1:{metrics_port}"),
        )
        .env("EFE_META_SWARM_MANIFEST", &manifest_path)
        .env("EFE_META_SWARM_LOG", "warn")
        // Make sure stdout/stderr don't interfere with test output.
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn controller binary");
    ProcessGuard {
        child,
        _manifest: manifest,
    }
}

async fn wait_until_healthy(endpoint: &str) -> bool {
    for _ in 0..40 {
        if let Ok(ep) = Endpoint::from_shared(endpoint.to_string()) {
            let ep = ep
                .timeout(Duration::from_millis(200))
                .connect_timeout(Duration::from_millis(200));
            if let Ok(channel) = ep.connect().await {
                let mut client = MetaSwarmServiceClient::new(channel);
                if let Ok(resp) = client.health(HealthRequest {}).await {
                    if resp.into_inner().status == 1 {
                        return true;
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    false
}

#[tokio::test(flavor = "multi_thread")]
async fn controller_health_returns_serving() {
    let grpc_port = 53080;
    let metrics_port = 53091;
    let _guard = spawn_controller(grpc_port, metrics_port);

    let endpoint = format!("http://127.0.0.1:{grpc_port}");
    assert!(
        wait_until_healthy(&endpoint).await,
        "controller did not become healthy within 10 s",
    );

    let channel = Endpoint::from_shared(endpoint)
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut client = MetaSwarmServiceClient::new(channel);
    let resp = client.health(HealthRequest {}).await.unwrap();
    let h = resp.into_inner();
    assert_eq!(h.status, 1, "expected STATUS_SERVING");
    assert_eq!(h.inflight, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn controller_iterate_empty_returns_failed_precondition() {
    // PR #8 wires `Iterate` ; an empty request lacks `iteration` and
    // is rejected by `validate_request` → FAILED_PRECONDITION.
    // (Previously, PR #6 returned UNIMPLEMENTED here.)
    let grpc_port = 53082;
    let metrics_port = 53093;
    let _guard = spawn_controller(grpc_port, metrics_port);

    let endpoint = format!("http://127.0.0.1:{grpc_port}");
    assert!(wait_until_healthy(&endpoint).await);

    let channel = Endpoint::from_shared(endpoint)
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut client = MetaSwarmServiceClient::new(channel);
    let err = client.iterate(IterateRequest::default()).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    assert!(
        err.message().contains("iteration"),
        "unexpected error message : {}",
        err.message()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn metrics_endpoint_responds() {
    let grpc_port = 53084;
    let metrics_port = 53095;
    let _guard = spawn_controller(grpc_port, metrics_port);

    let url = format!("http://127.0.0.1:{metrics_port}/metrics");
    for _ in 0..40 {
        if let Ok(resp) = reqwest::get(&url).await {
            if resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                // PR #6 ships an empty registry ; the body is empty.
                // (PR #17 will register 14 metrics ; this test will
                // need to be updated then.)
                assert!(body.is_empty() || body.contains("# HELP"));
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("metrics endpoint did not become reachable within 10 s");
}
