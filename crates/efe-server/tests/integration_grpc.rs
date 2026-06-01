//! End-to-end integration tests for the gRPC surface.
//!
//! Each test spawns a local `efe-server` on an ephemeral port with a
//! deterministic stub backend, then exercises the wire protocol using
//! real `tonic` clients generated from the proto descriptors.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tonic::transport::Channel;

use efe_core::alignment::{AlignmentRegistry, InMemoryRegistry};
use efe_core::{DecodedOutput, EfeError, Request as DomainRequest, Response, Result};
use efe_server::proto::v1::admin::{self as admin, admin_service_client::AdminServiceClient};
use efe_server::proto::v1::{self as v1, swarm_service_client::SwarmServiceClient};
use efe_server::services::ExecuteBackend;
use efe_server::{Server, ServerConfig, Shutdown};

// --------------------------------------------------------------------
// Stub backend
// --------------------------------------------------------------------

#[derive(Debug, Default)]
struct StubBackend {
    fail_with: Option<EfeError>,
    text_reply: String,
}

impl StubBackend {
    fn succeeding(text: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            fail_with: None,
            text_reply: text.into(),
        })
    }
    fn failing(err: EfeError) -> Arc<Self> {
        Arc::new(Self {
            fail_with: Some(err),
            text_reply: String::new(),
        })
    }
}

#[async_trait]
impl ExecuteBackend for StubBackend {
    async fn execute(&self, req: DomainRequest) -> Result<Response> {
        if let Some(err) = &self.fail_with {
            // EfeError doesn't derive Clone for every variant, so we reconstruct
            // a shaped instance per case.
            return Err(match err {
                EfeError::InvalidConfig(s) => EfeError::InvalidConfig(s.clone()),
                EfeError::AlignmentBreach { delta, max } => EfeError::AlignmentBreach {
                    delta: *delta,
                    max: *max,
                },
                EfeError::Internal(path) => EfeError::Internal(path),
                _ => EfeError::InvalidConfig("stub: generic failure".into()),
            });
        }
        let mut resp = Response::for_tests(req.id, DecodedOutput::Text(self.text_reply.clone()));
        resp.rounds_executed = 3;
        resp.total_ms = 10.0;
        resp.confidence = 0.95;
        Ok(resp)
    }
}

// --------------------------------------------------------------------
// Server harness
// --------------------------------------------------------------------

struct TestServer {
    addr: std::net::SocketAddr,
    shutdown: Shutdown,
    handle: tokio::task::JoinHandle<anyhow::Result<()>>,
    registry: Arc<dyn AlignmentRegistry>,
}

impl TestServer {
    async fn start(backend: Arc<dyn ExecuteBackend>) -> TestServer {
        // Pick an ephemeral port.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let mut cfg = ServerConfig::default();
        cfg.listen_addr = addr;
        cfg.auth.enabled = false;
        cfg.ratelimit.enabled = false;
        cfg.idempotency.enabled = true;

        let registry: Arc<dyn AlignmentRegistry> =
            Arc::new(InMemoryRegistry::with_default_config().unwrap());

        let server = Server::builder()
            .config(cfg)
            .backend(backend)
            .registry(registry.clone())
            .build()
            .await
            .unwrap();

        let shutdown = Shutdown::new();
        let shutdown_for_task = shutdown.clone();
        let handle = tokio::spawn(async move { server.serve(addr, shutdown_for_task).await });
        // Give the server a moment to bind.
        tokio::time::sleep(Duration::from_millis(100)).await;
        TestServer {
            addr,
            shutdown,
            handle,
            registry,
        }
    }

    async fn channel(&self) -> Channel {
        let endpoint = format!("http://{}", self.addr);
        Channel::from_shared(endpoint)
            .unwrap()
            .connect()
            .await
            .unwrap()
    }

    async fn stop(self) {
        self.shutdown.trigger();
        // Give the runtime a tick to drain.
        let _ = tokio::time::timeout(Duration::from_secs(3), self.handle).await;
    }
}

fn make_spec() -> v1::SwarmSpec {
    v1::SwarmSpec {
        swarm_id: String::new(),
        problem_embedding: vec![0.0; 4],
        dim_latent: 4,
        population: vec![v1::PopulationEntry {
            template_code: "na-sem".into(),
            count: 4,
        }],
        max_rounds: 4,
        convergence: None,
        cone_source: Some(v1::swarm_spec::ConeSource::AlignmentCone(
            v1::AlignmentCone {
                anchors: vec![v1::Anchor {
                    values: vec![0.1, 0.1, 0.1, 0.1],
                }],
                max_projection_distance: 0.5,
            },
        )),
        budget: None,
    }
}

// --------------------------------------------------------------------
// Tests
// --------------------------------------------------------------------

#[tokio::test]
async fn execute_happy_path_returns_expected_text() {
    let ts = TestServer::start(StubBackend::succeeding("hello world")).await;
    let channel = ts.channel().await;
    let mut client = SwarmServiceClient::new(channel);
    let resp = client
        .execute(v1::ExecuteRequest {
            request_id: String::new(),
            raw_prompt: String::new(),
            spec: Some(make_spec()),
            decode_mode: None,
            deadline: None,
        })
        .await
        .unwrap()
        .into_inner();
    match resp.output.and_then(|o| o.payload) {
        Some(v1::decoded_output::Payload::Text(s)) => assert_eq!(s, "hello world"),
        _ => panic!("expected text"),
    }
    assert_eq!(resp.rounds_executed, 3);
    ts.stop().await;
}

#[tokio::test]
async fn execute_invalid_spec_returns_invalid_argument() {
    let ts = TestServer::start(StubBackend::succeeding("x")).await;
    let mut client = SwarmServiceClient::new(ts.channel().await);
    let mut bad = make_spec();
    bad.dim_latent = 0; // triggers LatentDim::new failure in bridge.
    let err = client
        .execute(v1::ExecuteRequest {
            request_id: String::new(),
            raw_prompt: String::new(),
            spec: Some(bad),
            decode_mode: None,
            deadline: None,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    ts.stop().await;
}

#[tokio::test]
async fn execute_backend_internal_error_never_leaks_static_msg() {
    let backend = StubBackend::failing(EfeError::Internal("kernel/secret/path"));
    let ts = TestServer::start(backend).await;
    let mut client = SwarmServiceClient::new(ts.channel().await);
    let err = client
        .execute(v1::ExecuteRequest {
            request_id: String::new(),
            raw_prompt: String::new(),
            spec: Some(make_spec()),
            decode_mode: None,
            deadline: None,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::Internal);
    assert!(!err.message().contains("kernel/secret/path"));
    assert_eq!(err.message(), "internal error");
    ts.stop().await;
}

#[tokio::test]
async fn describe_spec_valid_reports_rounds_expected() {
    let ts = TestServer::start(StubBackend::succeeding("x")).await;
    let mut client = SwarmServiceClient::new(ts.channel().await);
    let resp = client
        .describe_spec(v1::DescribeSpecRequest {
            spec: Some(make_spec()),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(resp.valid);
    assert_eq!(resp.rounds_expected, 4);
    assert!(resp.hbm_bytes_estimate > 0);
    ts.stop().await;
}

#[tokio::test]
async fn describe_spec_invalid_returns_errors_but_ok_status() {
    let ts = TestServer::start(StubBackend::succeeding("x")).await;
    let mut client = SwarmServiceClient::new(ts.channel().await);
    let mut bad = make_spec();
    bad.dim_latent = 0;
    let resp = client
        .describe_spec(v1::DescribeSpecRequest { spec: Some(bad) })
        .await
        .unwrap()
        .into_inner();
    assert!(!resp.valid);
    assert!(!resp.errors.is_empty());
    ts.stop().await;
}

#[tokio::test]
async fn execute_streaming_emits_final_chunk_for_stub_backend() {
    let ts = TestServer::start(StubBackend::succeeding("streamed")).await;
    let mut client = SwarmServiceClient::new(ts.channel().await);
    let mut stream = client
        .execute_streaming(v1::ExecuteRequest {
            request_id: String::new(),
            raw_prompt: String::new(),
            spec: Some(make_spec()),
            decode_mode: None,
            deadline: None,
        })
        .await
        .unwrap()
        .into_inner();
    let mut saw_final = false;
    while let Some(chunk) = stream.message().await.unwrap() {
        match chunk.payload {
            Some(v1::stream_chunk::Payload::Final(_)) => saw_final = true,
            Some(_) => {}
            None => {}
        }
    }
    assert!(saw_final, "expected at least one Final chunk");
    ts.stop().await;
}

#[tokio::test]
async fn idempotency_short_circuits_identical_request() {
    let ts = TestServer::start(StubBackend::succeeding("cached")).await;
    let mut client = SwarmServiceClient::new(ts.channel().await);
    let req = v1::ExecuteRequest {
        request_id: "idem-1".into(),
        raw_prompt: String::new(),
        spec: Some(make_spec()),
        decode_mode: None,
        deadline: None,
    };
    let r1 = client.execute(req.clone()).await.unwrap().into_inner();
    let r2 = client.execute(req).await.unwrap().into_inner();
    assert_eq!(r1.request_id, r2.request_id);
    assert_eq!(r1.swarm_id, r2.swarm_id); // cached → same swarm id
    ts.stop().await;
}

#[tokio::test]
async fn admin_register_get_roundtrip() {
    let ts = TestServer::start(StubBackend::succeeding("x")).await;
    let mut admin = AdminServiceClient::new(ts.channel().await);

    let handle = admin
        .register_cone(admin::RegisterConeRequest {
            tenant: "acme".into(),
            task: "qa".into(),
            version: 1,
            cone: Some(v1::AlignmentCone {
                anchors: vec![v1::Anchor {
                    values: vec![1.0, 0.0],
                }],
                max_projection_distance: 0.5,
            }),
            created_by: "bot".into(),
            description: "first cone".into(),
            labels: Default::default(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(handle.key, "acme/qa@v1");

    let got = admin
        .get_cone(admin::GetConeRequest {
            key: "acme/qa@v1".into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(got.cone.unwrap().anchors.len(), 1);
    ts.stop().await;
}

#[tokio::test]
async fn admin_list_versions_returns_sorted_keys() {
    let ts = TestServer::start(StubBackend::succeeding("x")).await;
    let mut admin = AdminServiceClient::new(ts.channel().await);
    for v in 1..=3 {
        admin
            .register_cone(admin::RegisterConeRequest {
                tenant: "t".into(),
                task: "k".into(),
                version: v,
                cone: Some(v1::AlignmentCone {
                    anchors: vec![v1::Anchor {
                        values: vec![1.0, 0.0],
                    }],
                    max_projection_distance: 0.5,
                }),
                created_by: "bot".into(),
                description: format!("v{v}"),
                labels: Default::default(),
            })
            .await
            .unwrap();
    }
    let resp = admin
        .list_versions(admin::ListVersionsRequest {
            tenant: "t".into(),
            task: "k".into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.keys,
        vec![
            "t/k@v1".to_string(),
            "t/k@v2".to_string(),
            "t/k@v3".to_string()
        ]
    );
    ts.stop().await;
}

#[tokio::test]
async fn admin_delete_last_without_force_rejected() {
    let ts = TestServer::start(StubBackend::succeeding("x")).await;
    let mut admin = AdminServiceClient::new(ts.channel().await);
    admin
        .register_cone(admin::RegisterConeRequest {
            tenant: "t".into(),
            task: "a".into(),
            version: 1,
            cone: Some(v1::AlignmentCone {
                anchors: vec![v1::Anchor {
                    values: vec![1.0, 0.0],
                }],
                max_projection_distance: 0.5,
            }),
            created_by: "bot".into(),
            description: "initial".into(),
            labels: Default::default(),
        })
        .await
        .unwrap();
    let err = admin
        .delete_cone(admin::DeleteConeRequest {
            key: "t/a@v1".into(),
            force: false,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    ts.stop().await;
}

#[tokio::test]
async fn registry_is_accessible_via_test_handle() {
    // Smoke check that the test harness exposes the registry for
    // assertions made outside the gRPC surface (useful for breach
    // injection prior to BreachHistory calls).
    let ts = TestServer::start(StubBackend::succeeding("x")).await;
    assert_eq!(ts.registry.backend_name(), "in_memory");
    ts.stop().await;
}

// -------------------------------------------------------------------
// Auth integration tests — these guard against the regression the
// original interceptor-based design would have introduced: unable to
// resolve the RPC path, every authenticated request was denied.
// -------------------------------------------------------------------

use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn sign_jwt(secret: &[u8], scopes: &[&str], tenant: &str) -> String {
    #[derive(serde::Serialize)]
    struct TestClaims<'a> {
        iss: &'a str,
        sub: &'a str,
        aud: &'a str,
        exp: u64,
        tenant_id: &'a str,
        scopes: Vec<&'a str>,
    }
    let claims = TestClaims {
        iss: "efe.test",
        sub: "svc",
        aud: "efe.v1",
        exp: now_secs() + 300,
        tenant_id: tenant,
        scopes: scopes.to_vec(),
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret),
    )
    .unwrap()
}

async fn start_with_auth(backend: Arc<dyn ExecuteBackend>, secret: Vec<u8>) -> TestServer {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let mut cfg = ServerConfig::default();
    cfg.listen_addr = addr;
    cfg.auth.enabled = true;
    cfg.auth.hs256_secrets = vec![secret];
    cfg.auth.jwt_audience = "efe.v1".into();
    cfg.ratelimit.enabled = false;
    cfg.idempotency.enabled = true;

    let registry: Arc<dyn AlignmentRegistry> =
        Arc::new(InMemoryRegistry::with_default_config().unwrap());

    let server = Server::builder()
        .config(cfg)
        .backend(backend)
        .registry(registry.clone())
        .build()
        .await
        .unwrap();

    let shutdown = Shutdown::new();
    let shutdown_for_task = shutdown.clone();
    let handle = tokio::spawn(async move { server.serve(addr, shutdown_for_task).await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    TestServer {
        addr,
        shutdown,
        handle,
        registry,
    }
}

fn request_with_token(token: &str) -> tonic::Request<v1::ExecuteRequest> {
    let mut req = tonic::Request::new(v1::ExecuteRequest {
        request_id: String::new(),
        raw_prompt: String::new(),
        spec: Some(make_spec()),
        decode_mode: None,
        deadline: None,
    });
    req.metadata_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    req
}

#[tokio::test]
async fn auth_enabled_valid_token_accepted() {
    let secret = b"test-secret".to_vec();
    let ts = start_with_auth(StubBackend::succeeding("ok"), secret.clone()).await;
    let mut client = SwarmServiceClient::new(ts.channel().await);
    // D22 broad-scope : data-plane principals MUST also carry the
    // broad `data` scope ; the granular `swarm:execute` alone is
    // necessary but not sufficient.
    let tok = sign_jwt(&secret, &["swarm:execute", "data"], "acme");
    let resp = client.execute(request_with_token(&tok)).await.unwrap();
    let inner = resp.into_inner();
    match inner.output.and_then(|o| o.payload) {
        Some(v1::decoded_output::Payload::Text(s)) => assert_eq!(s, "ok"),
        _ => panic!("expected text"),
    }
    ts.stop().await;
}

#[tokio::test]
async fn auth_enabled_missing_token_unauthenticated() {
    let secret = b"test-secret".to_vec();
    let ts = start_with_auth(StubBackend::succeeding("ok"), secret).await;
    let mut client = SwarmServiceClient::new(ts.channel().await);
    let err = client
        .execute(v1::ExecuteRequest {
            request_id: String::new(),
            raw_prompt: String::new(),
            spec: Some(make_spec()),
            decode_mode: None,
            deadline: None,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unauthenticated);
    ts.stop().await;
}

#[tokio::test]
async fn auth_enabled_invalid_token_unauthenticated() {
    let secret = b"test-secret".to_vec();
    let ts = start_with_auth(StubBackend::succeeding("ok"), secret).await;
    let mut client = SwarmServiceClient::new(ts.channel().await);
    let err = client
        .execute(request_with_token("garbage"))
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unauthenticated);
    ts.stop().await;
}

#[tokio::test]
async fn auth_enabled_missing_scope_permission_denied() {
    let secret = b"test-secret".to_vec();
    let ts = start_with_auth(StubBackend::succeeding("ok"), secret.clone()).await;
    let mut client = SwarmServiceClient::new(ts.channel().await);
    // Token only has swarm:describe, but we call Execute which needs swarm:execute.
    let tok = sign_jwt(&secret, &["swarm:describe"], "acme");
    let err = client.execute(request_with_token(&tok)).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::PermissionDenied);
    ts.stop().await;
}

#[tokio::test]
async fn auth_enabled_describe_needs_describe_scope() {
    let secret = b"test-secret".to_vec();
    let ts = start_with_auth(StubBackend::succeeding("ok"), secret.clone()).await;
    let mut client = SwarmServiceClient::new(ts.channel().await);
    let tok = sign_jwt(&secret, &["swarm:describe"], "acme");
    let mut req = tonic::Request::new(v1::DescribeSpecRequest {
        spec: Some(make_spec()),
    });
    req.metadata_mut()
        .insert("authorization", format!("Bearer {tok}").parse().unwrap());
    let resp = client.describe_spec(req).await.unwrap().into_inner();
    assert!(resp.valid);
    ts.stop().await;
}

#[tokio::test]
async fn auth_enabled_health_is_public() {
    // Health has no required scope and must remain accessible.
    let secret = b"test-secret".to_vec();
    let ts = start_with_auth(StubBackend::succeeding("ok"), secret).await;
    let channel = ts.channel().await;
    let mut client = tonic_health::pb::health_client::HealthClient::new(channel);
    let resp = client
        .check(tonic_health::pb::HealthCheckRequest {
            service: String::new(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.status,
        tonic_health::pb::health_check_response::ServingStatus::Serving as i32
    );
    ts.stop().await;
}

#[tokio::test]
async fn auth_enabled_admin_register_with_proper_scope() {
    let secret = b"test-secret".to_vec();
    let ts = start_with_auth(StubBackend::succeeding("ok"), secret.clone()).await;
    let mut client = AdminServiceClient::new(ts.channel().await);
    let tok = sign_jwt(&secret, &["admin:cone:write"], "acme");
    let mut req = tonic::Request::new(admin::RegisterConeRequest {
        tenant: "acme".into(),
        task: "qa".into(),
        version: 1,
        cone: Some(v1::AlignmentCone {
            anchors: vec![v1::Anchor {
                values: vec![1.0, 0.0],
            }],
            max_projection_distance: 0.5,
        }),
        created_by: "alice".into(),
        description: "first".into(),
        labels: Default::default(),
    });
    req.metadata_mut()
        .insert("authorization", format!("Bearer {tok}").parse().unwrap());
    let handle = client.register_cone(req).await.unwrap().into_inner();
    assert_eq!(handle.key, "acme/qa@v1");
    ts.stop().await;
}

#[tokio::test]
async fn auth_enabled_admin_register_wrong_scope_denied() {
    let secret = b"test-secret".to_vec();
    let ts = start_with_auth(StubBackend::succeeding("ok"), secret.clone()).await;
    let mut client = AdminServiceClient::new(ts.channel().await);
    // Read scope, trying to call write op.
    let tok = sign_jwt(&secret, &["admin:cone:read"], "acme");
    let mut req = tonic::Request::new(admin::RegisterConeRequest {
        tenant: "acme".into(),
        task: "qa".into(),
        version: 1,
        cone: Some(v1::AlignmentCone {
            anchors: vec![v1::Anchor {
                values: vec![1.0, 0.0],
            }],
            max_projection_distance: 0.5,
        }),
        created_by: "alice".into(),
        description: "nope".into(),
        labels: Default::default(),
    });
    req.metadata_mut()
        .insert("authorization", format!("Bearer {tok}").parse().unwrap());
    let err = client.register_cone(req).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::PermissionDenied);
    ts.stop().await;
}

#[tokio::test]
async fn traceparent_metadata_is_propagated_to_backend() {
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    struct CapturingBackend {
        captured: Mutex<Option<Option<efe_core::swarm::TraceContext>>>,
    }
    #[async_trait]
    impl ExecuteBackend for CapturingBackend {
        async fn execute(&self, req: DomainRequest) -> Result<Response> {
            *self.captured.lock().unwrap() = Some(req.trace_context.clone());
            Ok(Response::for_tests(
                req.id,
                DecodedOutput::Text("captured".into()),
            ))
        }
    }

    let backend = Arc::new(CapturingBackend::default());
    let ts = TestServer::start(backend.clone()).await;
    let mut client = SwarmServiceClient::new(ts.channel().await);
    let mut req = tonic::Request::new(v1::ExecuteRequest {
        request_id: String::new(),
        raw_prompt: String::new(),
        spec: Some(make_spec()),
        decode_mode: None,
        deadline: None,
    });
    req.metadata_mut().insert(
        "traceparent",
        "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"
            .parse()
            .unwrap(),
    );
    let _ = client.execute(req).await.unwrap();
    let captured = backend.captured.lock().unwrap().clone();
    let tc = captured
        .expect("backend invoked")
        .expect("trace_context populated");
    assert!(tc.traceparent.contains("0af7651916cd43dd8448eb211c80319c"));
    ts.stop().await;
}
