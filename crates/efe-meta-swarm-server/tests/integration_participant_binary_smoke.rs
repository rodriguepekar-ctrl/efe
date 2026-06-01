//! PR #9 — participant *binary* smoke (process boundary).
//!
//! Mirrors `integration_health_smoke.rs` (which tests the controller
//! binary) but spawns the participant binary instead.  Exercises the
//! complete boot path :
//!
//!   1. env-var config parsed,
//!   2. manifest loaded + validated,
//!   3. own row found by id,
//!   4. `LocalParticipant` built,
//!   5. gRPC server bound,
//!   6. `Step` / `Reset` / `State` round-trip through the wire.
//!
//! Closes acceptance criterion **AC20** (binary smoke, `cargo run`
//! against a manifest fixture).

use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use efe_meta_swarm_proto::meta_swarm::{
    HealthRequest, ResetParticipantRequest, StateRequest, StepRequest, TensorF32,
};
use efe_meta_swarm_proto::participant::MetaParticipantServiceClient;
use tempfile::NamedTempFile;
use tonic::transport::Endpoint;
use tonic::Request;

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
  - id: contract-risk
    dim: 8
    kappa: 0.7
    seed: 22
iteration:
  max_iters: 100
  tolerance: 0.0001
  alpha_default: 0.5
  coupling_gamma: 0.2
  deterministic: true
  max_wall_ms: 15000
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

fn spawn_participant(grpc_port: u16, metrics_port: u16, id: &str) -> ProcessGuard {
    let mut manifest = NamedTempFile::new().expect("tempfile");
    manifest
        .write_all(MINIMAL_MANIFEST.as_bytes())
        .expect("write manifest");
    let manifest_path = manifest.path().to_path_buf();

    let bin = env!("CARGO_BIN_EXE_participant");
    let child = Command::new(bin)
        .env("EFE_META_SWARM_PARTICIPANT_ID", id)
        .env(
            "EFE_META_SWARM_PARTICIPANT_ADDR",
            format!("127.0.0.1:{grpc_port}"),
        )
        .env(
            "EFE_META_SWARM_PARTICIPANT_METRICS",
            format!("127.0.0.1:{metrics_port}"),
        )
        .env("EFE_META_SWARM_MANIFEST", &manifest_path)
        .env("EFE_META_SWARM_LOG", "warn")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn participant binary");
    ProcessGuard {
        child,
        _manifest: manifest,
    }
}

async fn wait_until_responsive(endpoint: &str) -> bool {
    for _ in 0..40 {
        if let Ok(ep) = Endpoint::from_shared(endpoint.to_string()) {
            let ep = ep
                .timeout(Duration::from_millis(200))
                .connect_timeout(Duration::from_millis(200));
            if let Ok(channel) = ep.connect().await {
                let mut client = MetaParticipantServiceClient::new(channel);
                if let Ok(resp) = client.health(Request::new(HealthRequest {})).await {
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

async fn connect(addr: u16) -> MetaParticipantServiceClient<tonic::transport::Channel> {
    let endpoint = Endpoint::from_shared(format!("http://127.0.0.1:{addr}"))
        .unwrap()
        .timeout(Duration::from_secs(5))
        .connect_timeout(Duration::from_secs(2))
        .connect()
        .await
        .expect("connect");
    MetaParticipantServiceClient::new(endpoint)
}

#[tokio::test(flavor = "multi_thread")]
async fn participant_binary_serves_health() {
    let grpc_port = 53180;
    let metrics_port = 53190;
    let _guard = spawn_participant(grpc_port, metrics_port, "supplier-scoring");

    let endpoint = format!("http://127.0.0.1:{grpc_port}");
    assert!(
        wait_until_responsive(&endpoint).await,
        "participant binary did not become healthy within 10 s",
    );

    let channel = Endpoint::from_shared(endpoint)
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut client = MetaParticipantServiceClient::new(channel);
    let h = client
        .health(Request::new(HealthRequest {}))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(h.status, 1, "expected STATUS_SERVING");
}

#[tokio::test(flavor = "multi_thread")]
async fn participant_binary_capabilities_reflects_manifest_row() {
    let grpc_port = 53182;
    let metrics_port = 53192;
    let _guard = spawn_participant(grpc_port, metrics_port, "supplier-scoring");

    let endpoint = format!("http://127.0.0.1:{grpc_port}");
    assert!(wait_until_responsive(&endpoint).await);

    let mut client = connect(grpc_port).await;
    let cap = client
        .capabilities(Request::new(
            efe_meta_swarm_proto::meta_swarm::CapabilitiesRequest {},
        ))
        .await
        .unwrap()
        .into_inner();
    let desc = cap.descriptor.expect("descriptor present");
    assert_eq!(desc.id, "supplier-scoring");
    assert_eq!(desc.dim, 8);
    assert!((desc.kappa - 0.6).abs() < 1e-6);
    assert_eq!(desc.seed, 11);
}

#[tokio::test(flavor = "multi_thread")]
async fn participant_binary_step_round_trip() {
    let grpc_port = 53184;
    let metrics_port = 53194;
    let _guard = spawn_participant(grpc_port, metrics_port, "supplier-scoring");

    let endpoint = format!("http://127.0.0.1:{grpc_port}");
    assert!(wait_until_responsive(&endpoint).await);

    let mut client = connect(grpc_port).await;
    let resp = client
        .step(Request::new(StepRequest {
            pulled_from_meta: Some(TensorF32 {
                dim: 8,
                data: vec![0.2; 8],
            }),
            step_index: 13,
        }))
        .await
        .expect("step")
        .into_inner();

    let new_state = resp.new_state.expect("new_state present");
    assert_eq!(new_state.dim, 8);
    assert_eq!(new_state.data.len(), 8);
    // step_us is the server's self-report ; can be 0 if the clock
    // resolution is coarse but never negative.
    assert!(resp.step_us < 100_000_000); // 100 s ceiling

    // Verify step_index_last was recorded.
    let st = client
        .state(Request::new(StateRequest {}))
        .await
        .expect("state")
        .into_inner();
    assert_eq!(st.step_index_last, 13);
}

#[tokio::test(flavor = "multi_thread")]
async fn participant_binary_reset_then_state() {
    let grpc_port = 53186;
    let metrics_port = 53196;
    let _guard = spawn_participant(grpc_port, metrics_port, "contract-risk");

    let endpoint = format!("http://127.0.0.1:{grpc_port}");
    assert!(wait_until_responsive(&endpoint).await);

    let mut client = connect(grpc_port).await;

    // Run a Step so state moves away from seeded value.
    let _ = client
        .step(Request::new(StepRequest {
            pulled_from_meta: Some(TensorF32 {
                dim: 8,
                data: vec![0.5; 8],
            }),
            step_index: 1,
        }))
        .await
        .expect("step");

    // Reset with custom initial.
    let initial = vec![0.0_f32; 8];
    let r = client
        .reset(Request::new(ResetParticipantRequest {
            initial: Some(TensorF32 {
                dim: 8,
                data: initial.clone(),
            }),
        }))
        .await
        .expect("reset")
        .into_inner();
    assert_eq!(r.reset_gen, 1);

    let st = client
        .state(Request::new(StateRequest {}))
        .await
        .expect("state")
        .into_inner();
    let current = st.current.expect("current present");
    for (i, v) in current.data.iter().enumerate() {
        assert!(
            v.abs() < f32::EPSILON,
            "state[{i}] = {v} ≠ 0.0 (Reset with zero initial did not apply)",
        );
    }
}
