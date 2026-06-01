//! **Sprint 8 — N11 days 14-15** : RPC dispatcher.
//!
//! Translates [`VsockRpcRequest`] → [`VsockRpcResponse`] by dispatching
//! on the method tag.  Stateless per-call (the model state lives in
//! the [`ModelBundle`] held by Arc).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use efe_nano::runtime::pool::firecracker::rpc::{
    HealthResult, InferParams, InferResult, RpcError, RpcErrorCode, RpcMethod, RpcPayload,
    RpcResult, VsockRpcRequest, VsockRpcResponse, SCHEMA_VERSION,
};

use crate::model::ModelBundle;

pub struct Handler {
    pub model_id: String,
    pub model_version: String,
    pub bundle: ModelBundle,
    pub boot_time: Instant,
}

impl Handler {
    pub fn new(
        model_id: String,
        model_version: String,
        bundle: ModelBundle,
        boot_time: Instant,
    ) -> Self {
        Self {
            model_id,
            model_version,
            bundle,
            boot_time,
        }
    }

    pub async fn dispatch(self: &Arc<Self>, request: VsockRpcRequest) -> VsockRpcResponse {
        let request_id = request.request_id;
        match request.method {
            RpcMethod::Health => VsockRpcResponse {
                schema_version: SCHEMA_VERSION,
                request_id,
                payload: RpcPayload::Ok {
                    result: RpcResult::Health(HealthResult {
                        status: "ready".into(),
                        uptime_secs: self.boot_time.elapsed().as_secs(),
                        model_id: self.model_id.clone(),
                        model_version: self.model_version.clone(),
                    }),
                },
            },
            RpcMethod::Shutdown { grace_secs: _ } => {
                // We ACK the shutdown ; the transport layer is responsible
                // for actually exiting the process after the response is
                // written (so the host can drain).
                VsockRpcResponse {
                    schema_version: SCHEMA_VERSION,
                    request_id,
                    payload: RpcPayload::Ok {
                        result: RpcResult::Shutdown { acknowledged: true },
                    },
                }
            }
            RpcMethod::Infer { params } => self.handle_infer(request_id, params),
        }
    }

    fn handle_infer(&self, request_id: uuid::Uuid, params: InferParams) -> VsockRpcResponse {
        // 1. Deadline check.
        let now = chrono::Utc::now().timestamp_micros();
        if now > params.deadline_micros {
            return VsockRpcResponse {
                schema_version: SCHEMA_VERSION,
                request_id,
                payload: RpcPayload::Err {
                    error: RpcError {
                        code: RpcErrorCode::DeadlineExceeded,
                        message: format!(
                            "deadline {} < now {}",
                            params.deadline_micros, now
                        ),
                    },
                },
            };
        }

        // 2. (Skipped : input_schema enforcement is the host's job.)

        // 3. Invoke the engine.
        match self.bundle.engine.infer(&params.input) {
            Ok(out) => VsockRpcResponse {
                schema_version: SCHEMA_VERSION,
                request_id,
                payload: RpcPayload::Ok {
                    result: RpcResult::Infer(InferResult {
                        output: out.value,
                        model_invocation_micros: out.model_invocation_micros,
                        model_memory_high_water_mb: out.model_memory_high_water_mb,
                        guest_metadata: {
                            let mut m = HashMap::new();
                            m.insert(
                                "engine".into(),
                                serde_json::Value::String(
                                    self.bundle.engine.engine_name().into(),
                                ),
                            );
                            m
                        },
                    }),
                },
            },
            Err(e) => {
                // Sprint 8 conservative mapping : engine errors are
                // classified as `Internal` (reap-worthy) so a single
                // broken inference doesn't keep poisoning the VM.
                VsockRpcResponse {
                    schema_version: SCHEMA_VERSION,
                    request_id,
                    payload: RpcPayload::Err {
                        error: RpcError {
                            code: RpcErrorCode::Internal,
                            message: e.to_string(),
                        },
                    },
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ModelLoader;
    use uuid::Uuid;

    async fn make_handler() -> Arc<Handler> {
        let bundle = ModelLoader::load("test_model", "1.0.0").unwrap();
        Arc::new(Handler::new(
            "test_model".into(),
            "1.0.0".into(),
            bundle,
            Instant::now(),
        ))
    }

    #[tokio::test]
    async fn health_returns_ready() {
        let h = make_handler().await;
        let req = VsockRpcRequest::health(Uuid::new_v4());
        let resp = h.dispatch(req).await;
        match resp.payload {
            RpcPayload::Ok {
                result: RpcResult::Health(r),
            } => {
                assert_eq!(r.status, "ready");
                assert_eq!(r.model_id, "test_model");
            }
            other => panic!("expected Health Ok, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn infer_echoes_via_stub_engine() {
        let h = make_handler().await;
        let req = VsockRpcRequest::infer(
            Uuid::new_v4(),
            serde_json::json!({"text": "hello"}),
            "test_model",
            "1.0.0",
            Uuid::new_v4(),
            i64::MAX,
        );
        let resp = h.dispatch(req).await;
        match resp.payload {
            RpcPayload::Ok {
                result: RpcResult::Infer(r),
            } => {
                assert_eq!(r.output["stub"], true);
                assert_eq!(r.output["echoed_input"]["text"], "hello");
                assert_eq!(r.guest_metadata["engine"], "stub");
            }
            other => panic!("expected Infer Ok, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn infer_with_past_deadline_returns_deadline_exceeded() {
        let h = make_handler().await;
        let req = VsockRpcRequest::infer(
            Uuid::new_v4(),
            serde_json::json!({}),
            "test_model",
            "1.0.0",
            Uuid::new_v4(),
            0, // Unix epoch — definitely in the past
        );
        let resp = h.dispatch(req).await;
        match resp.payload {
            RpcPayload::Err { error } => {
                assert_eq!(error.code, RpcErrorCode::DeadlineExceeded);
            }
            other => panic!("expected Err, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn shutdown_returns_acknowledged() {
        let h = make_handler().await;
        let req = VsockRpcRequest::shutdown(Uuid::new_v4(), 5);
        let resp = h.dispatch(req).await;
        match resp.payload {
            RpcPayload::Ok {
                result: RpcResult::Shutdown { acknowledged },
            } => assert!(acknowledged),
            other => panic!("expected Shutdown Ok, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn dispatch_preserves_request_id() {
        let h = make_handler().await;
        let id = Uuid::from_u128(0x42);
        let req = VsockRpcRequest::health(id);
        let resp = h.dispatch(req).await;
        assert_eq!(resp.request_id, id);
    }
}
