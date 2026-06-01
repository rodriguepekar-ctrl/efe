//! PR #20 § 6.2 — gRPC connection pool.
//!
//! Maintains N persistent HTTP/2 connections per target (AD22) ;
//! weighted-random target selection per request (deterministic seed).
//! Wraps `MetaSwarmServiceClient` (controller) and
//! `MetaParticipantServiceClient` (participant) under a single dispatch
//! surface — the workload modules treat the pool as a black box.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rand_xoshiro::Xoshiro256PlusPlus;
use tokio::sync::Mutex as AsyncMutex;
use tonic::transport::Channel;

use crate::config::{LoadgenConfig, TargetSpec};
use crate::determinism::weighted_pick;
use crate::error::LoadgenResult;

/// Proto-ready input for `Iterate` (kept in the engine namespace so
/// workload modules don't depend on proto crate symbols).
#[derive(Clone, Debug)]
pub struct ProtoIterateInput {
    pub initial: Vec<f64>,
    pub request_index: u64,
}

/// Proto-ready input for `Step`.
#[derive(Clone, Debug)]
pub struct ProtoStepInput {
    pub pulled: Vec<f64>,
    pub step_index: u32,
}

#[derive(Clone, Debug)]
pub struct IterateResponseSummary {
    pub iterations: u32,
    pub final_residual: f64,
    pub stop_reason: i32,
    pub target: String,
    pub trace_id: String,
}

#[derive(Clone, Debug)]
pub struct StepResponseSummary {
    pub target: String,
    pub trace_id: String,
}

#[derive(Clone, Debug)]
pub struct StateResponseSummary {
    pub dim: usize,
    pub target: String,
    pub trace_id: String,
}

#[derive(Clone, Debug)]
pub struct SimpleResponseSummary {
    pub target: String,
    pub trace_id: String,
}

/// Lightweight error envelope.
#[derive(Clone, Debug)]
pub struct ClientError {
    pub message: String,
    pub status_code: Option<tonic::Code>,
    pub target: Option<String>,
    pub trace_id: Option<String>,
}

/// Per-target connection bundle.
#[derive(Clone)]
struct TargetBundle {
    url: String,
    weight: f64,
    channels: Vec<Channel>,
    rr: Arc<Mutex<usize>>,
}

impl TargetBundle {
    fn next_channel(&self) -> Channel {
        let mut p = self.rr.lock();
        let i = *p;
        *p = (i + 1) % self.channels.len();
        self.channels[i].clone()
    }
}

/// Multi-target connection pool.
pub struct ClientPool {
    targets: Vec<TargetBundle>,
    per_request_deadline: Option<Duration>,
    target_role: crate::cli::TargetRole,
    selector_rng: Arc<AsyncMutex<Xoshiro256PlusPlus>>,
}

impl ClientPool {
    /// Connect to every target ; opens `conns_per_target` channels.
    pub async fn connect(
        cfg: &LoadgenConfig,
        selector_rng: Xoshiro256PlusPlus,
    ) -> LoadgenResult<Arc<Self>> {
        let mut targets = Vec::with_capacity(cfg.targets.len());
        for spec in &cfg.targets {
            let bundle = Self::connect_target(spec, cfg.conns_per_target, cfg.tls.as_ref()).await?;
            targets.push(bundle);
        }
        Ok(Arc::new(Self {
            targets,
            per_request_deadline: cfg.per_request_deadline,
            target_role: cfg.target_role,
            selector_rng: Arc::new(AsyncMutex::new(selector_rng)),
        }))
    }

    async fn connect_target(
        spec: &TargetSpec,
        conns_per_target: u32,
        tls: Option<&crate::config::TlsConfig>,
    ) -> LoadgenResult<TargetBundle> {
        let n = conns_per_target.max(1) as usize;
        let mut channels = Vec::with_capacity(n);
        for _ in 0..n {
            let ch = super::tls::connect_channel(&spec.url, tls).await?;
            channels.push(ch);
        }
        Ok(TargetBundle {
            url: spec.url.clone(),
            weight: spec.weight,
            channels,
            rr: Arc::new(Mutex::new(0)),
        })
    }

    async fn pick_target(&self) -> TargetBundle {
        let mut rng = self.selector_rng.lock().await;
        let weights: Vec<f64> = self.targets.iter().map(|t| t.weight).collect();
        let idx = weighted_pick(&mut *rng, &weights);
        self.targets[idx].clone()
    }

    fn f64_to_proto_tensor(&self, values: &[f64]) -> efe_meta_swarm_proto::meta_swarm::TensorF32 {
        // Proto Tensor is TensorF32 with `dim` + `data` (repeated float).
        efe_meta_swarm_proto::meta_swarm::TensorF32 {
            dim: values.len() as u32,
            data: values.iter().map(|x| *x as f32).collect(),
        }
    }

    /// Generate a fresh W3C Trace Context `traceparent` header value
    /// per request.  Format : `00-{trace_id_32}-{span_id_16}-01`.
    /// Returns `(trace_id_hex, traceparent_value)`.
    ///
    /// PR #18 § 14 conformance — the controller-side interceptor
    /// extracts this header and uses it as the parent span context.
    fn new_traceparent() -> (String, String) {
        let uuid = uuid::Uuid::new_v4();
        let trace_id_hex = uuid.simple().to_string(); // 32 hex chars
        let span_uuid = uuid::Uuid::new_v4();
        // Take the first 8 bytes of a fresh UUID for the span id (16 hex chars).
        let span_id_hex: String = span_uuid
            .simple()
            .to_string()
            .chars()
            .take(16)
            .collect();
        let traceparent = format!("00-{trace_id_hex}-{span_id_hex}-01");
        (trace_id_hex, traceparent)
    }

    /// Inject W3C `traceparent` + the loadgen convenience header on a
    /// tonic request.  Centralised so every RPC dispatch sets the same
    /// headers (AC11).
    fn inject_headers<T>(req: &mut tonic::Request<T>, trace_id: &str, traceparent: &str) {
        if let Ok(v) = traceparent.parse() {
            req.metadata_mut().insert("traceparent", v);
        }
        if let Ok(v) = trace_id.parse() {
            req.metadata_mut().insert("x-loadgen-trace-id", v);
        }
    }

    // ────────────────────────────────────────────────────────────
    //                       Dispatch methods
    // ────────────────────────────────────────────────────────────

    pub async fn iterate(&self, input: ProtoIterateInput) -> Result<IterateResponseSummary, ClientError> {
        use efe_meta_swarm_proto::meta_swarm::{
            meta_swarm_service_client::MetaSwarmServiceClient, IterateRequest, IterationConfig,
        };
        let bundle = self.pick_target().await;
        let target_url = bundle.url.clone();
        let channel = bundle.next_channel();
        let mut client = MetaSwarmServiceClient::new(channel);
        let proto_tensor = self.f64_to_proto_tensor(&input.initial);
        let req = IterateRequest {
            iteration: Some(IterationConfig {
                max_iters: 50,
                max_wall_ms: 30_000,
                ..Default::default()
            }),
            initial_states: vec![proto_tensor],
            ..Default::default()
        };
        let mut tonic_req = tonic::Request::new(req);
        if let Some(d) = self.per_request_deadline {
            tonic_req.set_timeout(d);
        }
        // W3C Trace Context injection (PR #18 § 14 conformance — AC11).
        let (trace_id, traceparent) = Self::new_traceparent();
        Self::inject_headers(&mut tonic_req, &trace_id, &traceparent);
        match client.iterate(tonic_req).await {
            Ok(resp) => {
                let r = resp.into_inner();
                Ok(IterateResponseSummary {
                    iterations: r.iterations,
                    // proto field is already f64 — no conversion.
                    final_residual: r.final_residual,
                    stop_reason: r.stop_reason,
                    target: target_url,
                    trace_id,
                })
            }
            Err(status) => Err(ClientError {
                message: status.message().to_string(),
                status_code: Some(status.code()),
                target: Some(target_url),
                trace_id: Some(trace_id),
            }),
        }
    }

    pub async fn step(&self, input: ProtoStepInput) -> Result<StepResponseSummary, ClientError> {
        let bundle = self.pick_target().await;
        let target_url = bundle.url.clone();
        let channel = bundle.next_channel();
        let (trace_id, traceparent) = Self::new_traceparent();
        match self.target_role {
            crate::cli::TargetRole::Controller => {
                use efe_meta_swarm_proto::meta_swarm::{
                    meta_swarm_service_client::MetaSwarmServiceClient, StepOnceRequest,
                };
                let mut client = MetaSwarmServiceClient::new(channel);
                // StepOnceRequest has no user-settable fields (proto §
                // `reserved 1 to 99`).  Default is a valid request.
                let req = StepOnceRequest::default();
                let mut tonic_req = tonic::Request::new(req);
                if let Some(d) = self.per_request_deadline {
                    tonic_req.set_timeout(d);
                }
                Self::inject_headers(&mut tonic_req, &trace_id, &traceparent);
                client
                    .step_once(tonic_req)
                    .await
                    .map(|_| StepResponseSummary { target: target_url.clone(), trace_id: trace_id.clone() })
                    .map_err(|s| ClientError {
                        message: s.message().to_string(),
                        status_code: Some(s.code()),
                        target: Some(target_url),
                        trace_id: Some(trace_id),
                    })
            }
            crate::cli::TargetRole::Participant => {
                use efe_meta_swarm_proto::meta_swarm::{
                    meta_participant_service_client::MetaParticipantServiceClient, StepRequest,
                };
                let mut client = MetaParticipantServiceClient::new(channel);
                let proto_tensor = self.f64_to_proto_tensor(&input.pulled);
                let req = StepRequest {
                    pulled_from_meta: Some(proto_tensor),
                    step_index: input.step_index,
                };
                let mut tonic_req = tonic::Request::new(req);
                if let Some(d) = self.per_request_deadline {
                    tonic_req.set_timeout(d);
                }
                Self::inject_headers(&mut tonic_req, &trace_id, &traceparent);
                client
                    .step(tonic_req)
                    .await
                    .map(|_| StepResponseSummary { target: target_url.clone(), trace_id: trace_id.clone() })
                    .map_err(|s| ClientError {
                        message: s.message().to_string(),
                        status_code: Some(s.code()),
                        target: Some(target_url),
                        trace_id: Some(trace_id),
                    })
            }
        }
    }

    pub async fn reset(&self) -> Result<SimpleResponseSummary, ClientError> {
        let bundle = self.pick_target().await;
        let target_url = bundle.url.clone();
        let channel = bundle.next_channel();
        let (trace_id, traceparent) = Self::new_traceparent();
        match self.target_role {
            crate::cli::TargetRole::Controller => {
                use efe_meta_swarm_proto::meta_swarm::{
                    meta_swarm_service_client::MetaSwarmServiceClient, ResetRequest,
                };
                let mut client = MetaSwarmServiceClient::new(channel);
                let mut tonic_req = tonic::Request::new(ResetRequest::default());
                if let Some(d) = self.per_request_deadline {
                    tonic_req.set_timeout(d);
                }
                Self::inject_headers(&mut tonic_req, &trace_id, &traceparent);
                client
                    .reset(tonic_req)
                    .await
                    .map(|_| SimpleResponseSummary { target: target_url.clone(), trace_id: trace_id.clone() })
                    .map_err(|s| ClientError {
                        message: s.message().to_string(),
                        status_code: Some(s.code()),
                        target: Some(target_url),
                        trace_id: Some(trace_id),
                    })
            }
            crate::cli::TargetRole::Participant => {
                use efe_meta_swarm_proto::meta_swarm::{
                    meta_participant_service_client::MetaParticipantServiceClient,
                    ResetParticipantRequest,
                };
                let mut client = MetaParticipantServiceClient::new(channel);
                let mut tonic_req = tonic::Request::new(ResetParticipantRequest::default());
                if let Some(d) = self.per_request_deadline {
                    tonic_req.set_timeout(d);
                }
                Self::inject_headers(&mut tonic_req, &trace_id, &traceparent);
                client
                    .reset(tonic_req)
                    .await
                    .map(|_| SimpleResponseSummary { target: target_url.clone(), trace_id: trace_id.clone() })
                    .map_err(|s| ClientError {
                        message: s.message().to_string(),
                        status_code: Some(s.code()),
                        target: Some(target_url),
                        trace_id: Some(trace_id),
                    })
            }
        }
    }

    pub async fn state(&self) -> Result<StateResponseSummary, ClientError> {
        let bundle = self.pick_target().await;
        let target_url = bundle.url.clone();
        let channel = bundle.next_channel();
        let (trace_id, traceparent) = Self::new_traceparent();
        match self.target_role {
            crate::cli::TargetRole::Controller => Err(ClientError {
                message: "state RPC unsupported on controller target ; set --target-role=participant".into(),
                status_code: Some(tonic::Code::Unimplemented),
                target: Some(target_url),
                trace_id: Some(trace_id),
            }),
            crate::cli::TargetRole::Participant => {
                use efe_meta_swarm_proto::meta_swarm::{
                    meta_participant_service_client::MetaParticipantServiceClient, StateRequest,
                };
                let mut client = MetaParticipantServiceClient::new(channel);
                let mut tonic_req = tonic::Request::new(StateRequest::default());
                if let Some(d) = self.per_request_deadline {
                    tonic_req.set_timeout(d);
                }
                Self::inject_headers(&mut tonic_req, &trace_id, &traceparent);
                client
                    .state(tonic_req)
                    .await
                    .map(|r| {
                        let dim = r
                            .into_inner()
                            .current
                            .map(|t| t.data.len())
                            .unwrap_or(0);
                        StateResponseSummary { dim, target: target_url.clone(), trace_id: trace_id.clone() }
                    })
                    .map_err(|s| ClientError {
                        message: s.message().to_string(),
                        status_code: Some(s.code()),
                        target: Some(target_url),
                        trace_id: Some(trace_id),
                    })
            }
        }
    }

    /// Number of distinct targets.
    #[must_use]
    pub fn target_count(&self) -> usize {
        self.targets.len()
    }
}
