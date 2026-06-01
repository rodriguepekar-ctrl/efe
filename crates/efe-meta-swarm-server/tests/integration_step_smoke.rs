//! PR #9 — single-binary participant smoke.
//!
//! Spawns one real [`ParticipantService`] on a random localhost port
//! and drives `Step` / `Reset` / `State` through the gRPC stack.
//! Verifies :
//!   * happy path Step returns a `new_state` of the right shape
//!   * NaN in `pulled_from_meta` is rejected with `FAILED_PRECONDITION`
//!   * dim mismatch is rejected
//!   * Reset bumps `reset_gen`
//!   * Reset with bad `initial.dim` is rejected
//!   * Health survives the cycle
//!   * Step latency for `dim = 8` is reasonable (< 50 ms p99 loose bound)
//!
//! [`ParticipantService`]: efe_meta_swarm_server::participant::ParticipantService

mod common;

use std::time::{Duration, Instant};

use efe_meta_swarm_proto::meta_swarm::{
    HealthRequest, ResetParticipantRequest, StateRequest, StepRequest, TensorF32,
};
use efe_meta_swarm_proto::participant::MetaParticipantServiceClient;
use tonic::transport::Endpoint;
use tonic::Request;

async fn connect(
    addr: std::net::SocketAddr,
) -> MetaParticipantServiceClient<tonic::transport::Channel> {
    let channel = Endpoint::from_shared(format!("http://{addr}"))
        .unwrap()
        .timeout(Duration::from_secs(5))
        .connect_timeout(Duration::from_secs(2))
        .connect()
        .await
        .expect("connect");
    MetaParticipantServiceClient::new(channel)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn step_round_trip_returns_new_state() {
    let (addr, _h) = common::spawn_participant_simple("p-1", 8, 0.6, 11).await;
    let mut client = connect(addr).await;

    let resp = client
        .step(Request::new(StepRequest {
            pulled_from_meta: Some(TensorF32 {
                dim: 8,
                data: vec![0.1; 8],
            }),
            step_index: 7,
        }))
        .await
        .expect("step")
        .into_inner();

    let new_state = resp.new_state.expect("new_state present");
    assert_eq!(new_state.dim, 8);
    assert_eq!(new_state.data.len(), 8);
    assert!(resp.local_residual >= 0.0);

    // step_index_last must reflect 7 after the call.
    let st = client
        .state(Request::new(StateRequest {}))
        .await
        .expect("state")
        .into_inner();
    assert_eq!(st.step_index_last, 7);
    let current = st.current.expect("current present");
    assert_eq!(current.dim, 8);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn step_rejects_missing_pulled() {
    let (addr, _h) = common::spawn_participant_simple("p-1", 8, 0.6, 11).await;
    let mut client = connect(addr).await;
    let err = client
        .step(Request::new(StepRequest {
            pulled_from_meta: None,
            step_index: 0,
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn step_rejects_nan_in_pulled() {
    let (addr, _h) = common::spawn_participant_simple("p-1", 8, 0.6, 11).await;
    let mut client = connect(addr).await;
    let mut data = vec![0.1_f32; 8];
    data[3] = f32::NAN;
    let err = client
        .step(Request::new(StepRequest {
            pulled_from_meta: Some(TensorF32 { dim: 8, data }),
            step_index: 0,
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    assert!(err.message().contains("not finite"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn step_rejects_dim_mismatch() {
    let (addr, _h) = common::spawn_participant_simple("p-1", 8, 0.6, 11).await;
    let mut client = connect(addr).await;
    let err = client
        .step(Request::new(StepRequest {
            pulled_from_meta: Some(TensorF32 {
                dim: 7,
                data: vec![0.0; 7],
            }),
            step_index: 0,
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reset_default_bumps_reset_gen() {
    let (addr, _h) = common::spawn_participant_simple("p-1", 8, 0.6, 11).await;
    let mut client = connect(addr).await;

    let r1 = client
        .reset(Request::new(ResetParticipantRequest::default()))
        .await
        .expect("reset 1")
        .into_inner();
    assert_eq!(r1.reset_gen, 1);

    let r2 = client
        .reset(Request::new(ResetParticipantRequest::default()))
        .await
        .expect("reset 2")
        .into_inner();
    assert_eq!(r2.reset_gen, 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reset_with_initial_sets_state() {
    // Architecture § 19.2 spec : when `initial` is provided and
    // well-formed, `Reset` writes it as the new state ; a subsequent
    // `State` call MUST return a tensor equal (element-wise) to the
    // requested initial.
    let (addr, _h) = common::spawn_participant_simple("p-1", 8, 0.6, 11).await;
    let mut client = connect(addr).await;

    let initial = vec![0.25_f32; 8];
    let resp = client
        .reset(Request::new(ResetParticipantRequest {
            initial: Some(TensorF32 {
                dim: 8,
                data: initial.clone(),
            }),
        }))
        .await
        .expect("reset")
        .into_inner();
    assert_eq!(resp.reset_gen, 1);

    let st = client
        .state(Request::new(StateRequest {}))
        .await
        .expect("state")
        .into_inner();
    let current = st.current.expect("current present");
    assert_eq!(current.dim, 8);
    // Element-wise equality : Reset(initial=...) writes the tensor
    // verbatim ; State must observe the same bit-pattern.
    for (i, (a, b)) in current.data.iter().zip(initial.iter()).enumerate() {
        assert!(
            (*a - *b).abs() < f32::EPSILON,
            "state[{i}] = {a} ≠ initial[{i}] = {b}",
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reset_with_initial_validates_dim() {
    let (addr, _h) = common::spawn_participant_simple("p-1", 8, 0.6, 11).await;
    let mut client = connect(addr).await;
    let err = client
        .reset(Request::new(ResetParticipantRequest {
            initial: Some(TensorF32 {
                dim: 4,
                data: vec![0.0; 4],
            }),
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_returns_serving_through_step_cycle() {
    let (addr, _h) = common::spawn_participant_simple("p-1", 8, 0.6, 11).await;
    let mut client = connect(addr).await;

    // Run a Step → Reset → Step cycle, then check health.
    let _ = client
        .step(Request::new(StepRequest {
            pulled_from_meta: Some(TensorF32 {
                dim: 8,
                data: vec![0.1; 8],
            }),
            step_index: 1,
        }))
        .await
        .expect("step");
    let _ = client
        .reset(Request::new(ResetParticipantRequest::default()))
        .await
        .expect("reset");
    let _ = client
        .step(Request::new(StepRequest {
            pulled_from_meta: Some(TensorF32 {
                dim: 8,
                data: vec![0.05; 8],
            }),
            step_index: 2,
        }))
        .await
        .expect("step 2");

    let h = client
        .health(Request::new(HealthRequest {}))
        .await
        .expect("health")
        .into_inner();
    assert_eq!(h.status, 1); // SERVING
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn step_stream_returns_unimplemented_over_grpc() {
    // PR #21 will wire `StepStream` (bidi).  Until then, the server
    // must reject the call with UNIMPLEMENTED + a message that
    // references PR #21, so operators can grep for the deferred ID.
    let (addr, _h) = common::spawn_participant_simple("p-1", 8, 0.6, 11).await;
    let mut client = connect(addr).await;

    // An empty outbound stream is sufficient — the handler returns
    // before reading the first message.
    let outbound = futures::stream::empty::<StepRequest>();
    let err = client.step_stream(outbound).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unimplemented);
    assert!(
        err.message().contains("PR #21"),
        "unexpected error message : {}",
        err.message(),
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn step_latency_under_50ms_for_dim_8() {
    let (addr, _h) = common::spawn_participant_simple("p-1", 8, 0.6, 11).await;
    let mut client = connect(addr).await;

    // Warm up.
    let _ = client
        .step(Request::new(StepRequest {
            pulled_from_meta: Some(TensorF32 {
                dim: 8,
                data: vec![0.1; 8],
            }),
            step_index: 0,
        }))
        .await
        .expect("warmup");

    // 5 measurements ; the test is loose (50 ms p99) so we don't
    // need a full benchmark harness.  Real-world latency on a 2024
    // server CPU is ~ 300 µs.
    let mut samples = Vec::with_capacity(5);
    for i in 1..=5 {
        let start = Instant::now();
        let _ = client
            .step(Request::new(StepRequest {
                pulled_from_meta: Some(TensorF32 {
                    dim: 8,
                    data: vec![0.1; 8],
                }),
                step_index: i,
            }))
            .await
            .expect("step");
        samples.push(start.elapsed());
    }
    let p99 = samples.iter().max().unwrap();
    assert!(
        *p99 < Duration::from_millis(50),
        "p99 latency = {p99:?} > 50 ms",
    );
}
