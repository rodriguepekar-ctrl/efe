//! Sprint **D12-bis** — End-to-end integration tests for the
//! `CatalogService` gRPC handler.
//!
//! Each test spawns a local `efe-server` on an ephemeral port, then
//! exercises the wire protocol using real `tonic` clients generated
//! from the proto descriptors. Coverage matrix per the architecture
//! doc § 13.5 :
//!
//!   * Happy paths for the 6 RPCs (register / evolve / get /
//!     list / list-versions / delete).
//!   * Auth + RBAC enforcement (missing token → Unauthenticated ;
//!     wrong scope → PermissionDenied).
//!   * D11 tenant isolation invariant (`assert_tenant_match` —
//!     cross-tenant reads/writes return PermissionDenied).
//!   * Error mapping (`CatalogError` variants → tonic Status codes).
//!   * Concurrency (SLO D12-CT-6 : two parallel registers/evolves
//!     receive distinct, monotonic version keys).
//!   * Observability (D13 audit counter increments on writes).
//!
//! Pattern : an in-memory registry per test (auto-instantiated by
//! `Server::builder()` when `EFE_CATALOG_BACKEND=memory`, the
//! default). No global state ; tests can run fully in parallel.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tonic::transport::Channel;

use efe_core::alignment::{AlignmentRegistry, InMemoryRegistry as AlignmentInMemoryRegistry};
use efe_core::{DecodedOutput, Request as DomainRequest, Response, Result};
use efe_server::proto::v1_catalog as cat;
use efe_server::services::ExecuteBackend;
use efe_server::{Server, ServerConfig, Shutdown};

use cat::catalog_service_client::CatalogServiceClient;

// --------------------------------------------------------------------
// Stub swarm backend — every test still requires a backend to build the
// server. The catalog handler does not use it.
// --------------------------------------------------------------------

#[derive(Debug, Default)]
struct StubBackend;

#[async_trait]
impl ExecuteBackend for StubBackend {
    async fn execute(&self, req: DomainRequest) -> Result<Response> {
        let mut resp = Response::for_tests(req.id, DecodedOutput::Text("noop".into()));
        resp.rounds_executed = 1;
        resp.confidence = 0.5;
        Ok(resp)
    }
}

fn _swarm_backend_compile_check() -> Arc<dyn ExecuteBackend> {
    Arc::new(StubBackend)
}

// --------------------------------------------------------------------
// Test harness
// --------------------------------------------------------------------

struct TestServer {
    addr: std::net::SocketAddr,
    shutdown: Shutdown,
    handle: tokio::task::JoinHandle<anyhow::Result<()>>,
}

impl TestServer {
    /// Start an `efe-server` with the catalog service attached and
    /// auth disabled. Used by the bulk of happy-path tests where the
    /// concern is the catalog wire surface, not auth.
    async fn start_no_auth() -> TestServer {
        Self::start_no_auth_with_cfg(|_| {}).await
    }

    /// Start an `efe-server` with auth disabled and a hook to mutate
    /// the `ServerConfig` before build (e.g. to set custom catalog
    /// policy quotas — used by the audit fix H1 enforcement tests).
    async fn start_no_auth_with_cfg<F>(mutate: F) -> TestServer
    where
        F: FnOnce(&mut ServerConfig),
    {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let mut cfg = ServerConfig::default();
        cfg.listen_addr = addr;
        cfg.auth.enabled = false;
        cfg.ratelimit.enabled = false;
        cfg.idempotency.enabled = false; // catalog has no idempotency yet.
        mutate(&mut cfg);

        let alignment_registry: Arc<dyn AlignmentRegistry> =
            Arc::new(AlignmentInMemoryRegistry::with_default_config().unwrap());

        let server = Server::builder()
            .config(cfg)
            .backend(Arc::new(StubBackend) as Arc<dyn ExecuteBackend>)
            .registry(alignment_registry)
            .build()
            .await
            .unwrap();

        let shutdown = Shutdown::new();
        let shutdown_for_task = shutdown.clone();
        let handle = tokio::spawn(async move { server.serve(addr, shutdown_for_task).await });
        tokio::time::sleep(Duration::from_millis(120)).await;
        TestServer {
            addr,
            shutdown,
            handle,
        }
    }

    /// Start with auth enabled and an HS256 secret. Tests that
    /// validate scope enforcement go through this.
    async fn start_with_auth(secret: Vec<u8>) -> TestServer {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let mut cfg = ServerConfig::default();
        cfg.listen_addr = addr;
        cfg.auth.enabled = true;
        cfg.auth.hs256_secrets = vec![secret];
        cfg.auth.jwt_audience = "efe.v1".into();
        cfg.ratelimit.enabled = false;
        cfg.idempotency.enabled = false;

        let alignment_registry: Arc<dyn AlignmentRegistry> =
            Arc::new(AlignmentInMemoryRegistry::with_default_config().unwrap());

        let server = Server::builder()
            .config(cfg)
            .backend(Arc::new(StubBackend) as Arc<dyn ExecuteBackend>)
            .registry(alignment_registry)
            .build()
            .await
            .unwrap();

        let shutdown = Shutdown::new();
        let shutdown_for_task = shutdown.clone();
        let handle = tokio::spawn(async move { server.serve(addr, shutdown_for_task).await });
        tokio::time::sleep(Duration::from_millis(120)).await;
        TestServer {
            addr,
            shutdown,
            handle,
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

    async fn client(&self) -> CatalogServiceClient<Channel> {
        CatalogServiceClient::new(self.channel().await)
    }

    async fn stop(self) {
        self.shutdown.trigger();
        let _ = tokio::time::timeout(Duration::from_secs(3), self.handle).await;
    }
}

// --------------------------------------------------------------------
// Catalog content fixtures
// --------------------------------------------------------------------

fn sample_content() -> cat::CatalogContent {
    cat::CatalogContent {
        embedder: Some(cat::EmbedderSpec {
            family: "sentence-transformers".into(),
            model: "paraphrase-multilingual-MiniLM-L12-v2".into(),
            dim: 384,
            normalize: "l2".into(),
        }),
        classes: vec![
            cat::CatalogClass {
                name: "billing".into(),
                description: "billing-related questions".into(),
                canonical_sentences: vec![
                    "How do I update my credit card?".into(),
                    "Where can I find my invoice?".into(),
                ],
                color: "#2563eb".into(),
                deprecated: false,
            },
            cat::CatalogClass {
                name: "tech_support".into(),
                description: "technical support".into(),
                canonical_sentences: vec!["My device is not working.".into()],
                color: "#dc2626".into(),
                deprecated: false,
            },
        ],
        cone_policy: Some(cat::ConePolicy {
            max_projection_distance: 0.6,
        }),
        thresholds: Some(cat::ThresholdPolicy {
            confidence_high: 0.85,
            confidence_medium: 0.6,
        }),
    }
}

fn sample_register_req(tenant: &str, name: &str) -> cat::RegisterCatalogRequest {
    cat::RegisterCatalogRequest {
        tenant: tenant.into(),
        name: name.into(),
        version: "1.0.0".into(),
        content: Some(sample_content()),
        created_by: "test@efe".into(),
        description: "smoke catalog".into(),
        labels: Default::default(),
    }
}

// --------------------------------------------------------------------
// JWT helpers (mirror of integration_grpc.rs)
// --------------------------------------------------------------------

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
        sub: "alice@acme",
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

fn with_token<T>(req: T, token: &str) -> tonic::Request<T> {
    let mut r = tonic::Request::new(req);
    r.metadata_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    r
}

// ====================================================================
// Section A — Happy paths (6 tests)
// ====================================================================

#[tokio::test]
async fn register_catalog_creates_v1_with_canonical_key() {
    let ts = TestServer::start_no_auth().await;
    let mut client = ts.client().await;
    let resp = client
        .register_catalog(sample_register_req("anonymous", "support"))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(resp.key, "anonymous/support@v1");
    let canon = resp.catalog.expect("catalog populated");
    let meta = canon.metadata.expect("metadata populated");
    assert_eq!(meta.evolution_op, "register");
    assert_eq!(meta.parent_key, "");
    assert!(!meta.content_sha256.is_empty(), "sha256 witness present");
    ts.stop().await;
}

#[tokio::test]
async fn evolve_catalog_increments_version_to_v2() {
    let ts = TestServer::start_no_auth().await;
    let mut client = ts.client().await;
    let v1_h = client
        .register_catalog(sample_register_req("anonymous", "evolve_target"))
        .await
        .unwrap()
        .into_inner();

    // Append a new class — anchors_blob is server-compiled at evolve time.
    let evolve = cat::EvolveCatalogRequest {
        from_key: v1_h.key.clone(),
        op: Some(cat::CatalogEvolutionOp {
            op: Some(cat::catalog_evolution_op::Op::AppendClass(
                cat::AppendClassOp {
                    cls: Some(cat::CatalogClass {
                        name: "new_intent".into(),
                        description: "freshly added".into(),
                        canonical_sentences: vec!["Help me get started.".into()],
                        color: "#16a34a".into(),
                        deprecated: false,
                    }),
                },
            )),
        }),
        metadata_override: None,
    };
    let v2_h = client.evolve_catalog(evolve).await.unwrap().into_inner();
    assert_eq!(v2_h.key, "anonymous/evolve_target@v2");
    let canon = v2_h.catalog.expect("catalog populated");
    let meta = canon.metadata.expect("metadata populated");
    assert_eq!(meta.parent_key, "anonymous/evolve_target@v1");
    assert_eq!(meta.evolution_op, "append_class");
    ts.stop().await;
}

#[tokio::test]
async fn get_catalog_returns_registered_content() {
    let ts = TestServer::start_no_auth().await;
    let mut client = ts.client().await;
    let h = client
        .register_catalog(sample_register_req("anonymous", "billing"))
        .await
        .unwrap()
        .into_inner();
    let got = client
        .get_catalog(cat::GetCatalogRequest {
            key: h.key,
            include_compiled: false,
        })
        .await
        .unwrap()
        .into_inner();
    let content = got.content.expect("content populated");
    assert_eq!(content.classes.len(), 2);
    assert_eq!(content.classes[0].name, "billing");
    assert_eq!(content.classes[1].name, "tech_support");
    // include_compiled=false → compiled vec must be empty.
    assert!(got.compiled.is_empty());
    ts.stop().await;
}

#[tokio::test]
async fn list_catalogs_returns_only_caller_tenant_entries() {
    let ts = TestServer::start_no_auth().await;
    let mut client = ts.client().await;
    // Register two distinct names under tenant "anonymous".
    client
        .register_catalog(sample_register_req("anonymous", "alpha"))
        .await
        .unwrap();
    client
        .register_catalog(sample_register_req("anonymous", "beta"))
        .await
        .unwrap();
    let resp = client
        .list_catalogs(cat::ListCatalogsRequest {
            tenant: "anonymous".into(),
            page_size: 100,
            page_token: String::new(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(resp.catalogs.len(), 2);
    let names: Vec<_> = resp.catalogs.iter().map(|c| c.name.clone()).collect();
    assert!(names.contains(&"alpha".to_string()));
    assert!(names.contains(&"beta".to_string()));
    // Every entry's tenant must match the caller (D11 invariant).
    for c in &resp.catalogs {
        assert_eq!(c.tenant, "anonymous");
    }
    ts.stop().await;
}

#[tokio::test]
async fn list_catalog_versions_returns_keys_ascending() {
    let ts = TestServer::start_no_auth().await;
    let mut client = ts.client().await;
    let v1_h = client
        .register_catalog(sample_register_req("anonymous", "history"))
        .await
        .unwrap()
        .into_inner();
    let _v2_h = client
        .evolve_catalog(cat::EvolveCatalogRequest {
            from_key: v1_h.key,
            op: Some(cat::CatalogEvolutionOp {
                op: Some(cat::catalog_evolution_op::Op::UpdateThresholds(
                    cat::UpdateThresholdsOp {
                        thresholds: Some(cat::ThresholdPolicy {
                            confidence_high: 0.9,
                            confidence_medium: 0.7,
                        }),
                    },
                )),
            }),
            metadata_override: None,
        })
        .await
        .unwrap()
        .into_inner();

    let listed = client
        .list_catalog_versions(cat::ListCatalogVersionsRequest {
            tenant: "anonymous".into(),
            name: "history".into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        listed.keys,
        vec![
            "anonymous/history@v1".to_string(),
            "anonymous/history@v2".to_string(),
        ]
    );
    ts.stop().await;
}

#[tokio::test]
async fn delete_catalog_removes_version_with_force() {
    let ts = TestServer::start_no_auth().await;
    let mut client = ts.client().await;
    let h = client
        .register_catalog(sample_register_req("anonymous", "deleteme"))
        .await
        .unwrap()
        .into_inner();
    client
        .delete_catalog(cat::DeleteCatalogRequest {
            key: h.key.clone(),
            force: true,
        })
        .await
        .unwrap();

    let err = client
        .get_catalog(cat::GetCatalogRequest {
            key: h.key,
            include_compiled: false,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::NotFound);
    ts.stop().await;
}

// ====================================================================
// Section B — Auth + RBAC (5 tests)
// ====================================================================

#[tokio::test]
async fn register_without_token_unauthenticated() {
    let secret = b"d12bis-secret-1".to_vec();
    let ts = TestServer::start_with_auth(secret).await;
    let mut client = ts.client().await;
    let err = client
        .register_catalog(sample_register_req("acme", "support"))
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unauthenticated);
    ts.stop().await;
}

#[tokio::test]
async fn register_with_read_only_scope_permission_denied() {
    let secret = b"d12bis-secret-2".to_vec();
    let ts = TestServer::start_with_auth(secret.clone()).await;
    let mut client = ts.client().await;
    let tok = sign_jwt(&secret, &["admin:catalog:read"], "acme");
    let err = client
        .register_catalog(with_token(sample_register_req("acme", "support"), &tok))
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::PermissionDenied);
    ts.stop().await;
}

#[tokio::test]
async fn get_catalog_without_token_unauthenticated() {
    let secret = b"d12bis-secret-3".to_vec();
    let ts = TestServer::start_with_auth(secret).await;
    let mut client = ts.client().await;
    let err = client
        .get_catalog(cat::GetCatalogRequest {
            key: "acme/support@v1".into(),
            include_compiled: false,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unauthenticated);
    ts.stop().await;
}

#[tokio::test]
async fn delete_with_read_only_scope_permission_denied() {
    let secret = b"d12bis-secret-4".to_vec();
    let ts = TestServer::start_with_auth(secret.clone()).await;
    let mut client = ts.client().await;
    let tok = sign_jwt(&secret, &["admin:catalog:read"], "acme");
    let err = client
        .delete_catalog(with_token(
            cat::DeleteCatalogRequest {
                key: "acme/support@v1".into(),
                force: true,
            },
            &tok,
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::PermissionDenied);
    ts.stop().await;
}

#[tokio::test]
async fn valid_token_with_write_scope_can_register() {
    let secret = b"d12bis-secret-5".to_vec();
    let ts = TestServer::start_with_auth(secret.clone()).await;
    let mut client = ts.client().await;
    let tok = sign_jwt(&secret, &["admin:catalog:write"], "acme");
    let resp = client
        .register_catalog(with_token(sample_register_req("acme", "support"), &tok))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(resp.key, "acme/support@v1");
    let canon = resp.catalog.expect("populated");
    let meta = canon.metadata.expect("metadata populated");
    // The principal field is the JWT `sub` claim, not the request body's `created_by`.
    assert_eq!(meta.created_by, "alice@acme");
    ts.stop().await;
}

// ====================================================================
// Section C — D11 tenant isolation (3 tests)
// ====================================================================

#[tokio::test]
async fn register_with_mismatched_tenant_in_body_permission_denied() {
    let secret = b"d12bis-secret-iso-1".to_vec();
    let ts = TestServer::start_with_auth(secret.clone()).await;
    let mut client = ts.client().await;
    let tok = sign_jwt(&secret, &["admin:catalog:write"], "acme");
    // JWT says tenant=acme, body says tenant=globex → must be rejected.
    let err = client
        .register_catalog(with_token(sample_register_req("globex", "support"), &tok))
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::PermissionDenied);
    ts.stop().await;
}

#[tokio::test]
async fn get_catalog_from_other_tenant_permission_denied() {
    let secret = b"d12bis-secret-iso-2".to_vec();
    let ts = TestServer::start_with_auth(secret.clone()).await;
    let mut client = ts.client().await;
    // First, create a catalog under tenant "acme".
    let acme_tok = sign_jwt(&secret, &["admin:catalog:write"], "acme");
    let h = client
        .register_catalog(with_token(
            sample_register_req("acme", "support"),
            &acme_tok,
        ))
        .await
        .unwrap()
        .into_inner();

    // Then try to read it from tenant "globex".
    let globex_tok = sign_jwt(&secret, &["admin:catalog:read"], "globex");
    let err = client
        .get_catalog(with_token(
            cat::GetCatalogRequest {
                key: h.key,
                include_compiled: false,
            },
            &globex_tok,
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::PermissionDenied);
    ts.stop().await;
}

#[tokio::test]
async fn list_catalogs_for_other_tenant_permission_denied() {
    let secret = b"d12bis-secret-iso-3".to_vec();
    let ts = TestServer::start_with_auth(secret.clone()).await;
    let mut client = ts.client().await;
    let tok = sign_jwt(&secret, &["admin:catalog:read"], "acme");
    let err = client
        .list_catalogs(with_token(
            cat::ListCatalogsRequest {
                tenant: "globex".into(),
                page_size: 100,
                page_token: String::new(),
            },
            &tok,
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::PermissionDenied);
    ts.stop().await;
}

// ====================================================================
// Section D — Error mapping (3 tests)
// ====================================================================

#[tokio::test]
async fn register_with_empty_name_invalid_argument() {
    let ts = TestServer::start_no_auth().await;
    let mut client = ts.client().await;
    let mut req = sample_register_req("anonymous", "billing");
    req.name = String::new();
    let err = client.register_catalog(req).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    ts.stop().await;
}

#[tokio::test]
async fn delete_referenced_catalog_without_force_failed_precondition() {
    // The InMemoryRegistry's reference-tracking surface is exercised by
    // creating v2 (which makes v1 a parent). Without force, deleting v1
    // should be rejected with FailedPrecondition because v2 references it.
    let ts = TestServer::start_no_auth().await;
    let mut client = ts.client().await;
    let v1_h = client
        .register_catalog(sample_register_req("anonymous", "ref_target"))
        .await
        .unwrap()
        .into_inner();
    let _v2 = client
        .evolve_catalog(cat::EvolveCatalogRequest {
            from_key: v1_h.key.clone(),
            op: Some(cat::CatalogEvolutionOp {
                op: Some(cat::catalog_evolution_op::Op::UpdateThresholds(
                    cat::UpdateThresholdsOp {
                        thresholds: Some(cat::ThresholdPolicy {
                            confidence_high: 0.92,
                            confidence_medium: 0.7,
                        }),
                    },
                )),
            }),
            metadata_override: None,
        })
        .await
        .unwrap();

    let err = client
        .delete_catalog(cat::DeleteCatalogRequest {
            key: v1_h.key,
            force: false,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    ts.stop().await;
}

#[tokio::test]
async fn get_catalog_with_invalid_key_invalid_argument() {
    let ts = TestServer::start_no_auth().await;
    let mut client = ts.client().await;
    let err = client
        .get_catalog(cat::GetCatalogRequest {
            key: "this is not a canonical key".into(),
            include_compiled: false,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    ts.stop().await;
}

// ====================================================================
// Section E — Concurrency / SLO D12-CT-6 (2 tests)
// ====================================================================

#[tokio::test]
async fn concurrent_register_two_distinct_names_succeeds() {
    // Two different (tenant, name) tuples register in parallel — both
    // should land at v1 with no inter-test interference.
    let ts = TestServer::start_no_auth().await;
    let chan = ts.channel().await;
    let mut c1 = CatalogServiceClient::new(chan.clone());
    let mut c2 = CatalogServiceClient::new(chan);

    let (r1, r2) = tokio::join!(
        c1.register_catalog(sample_register_req("anonymous", "concurrent_a")),
        c2.register_catalog(sample_register_req("anonymous", "concurrent_b")),
    );
    let h1 = r1.unwrap().into_inner();
    let h2 = r2.unwrap().into_inner();
    assert_eq!(h1.key, "anonymous/concurrent_a@v1");
    assert_eq!(h2.key, "anonymous/concurrent_b@v1");
    ts.stop().await;
}

#[tokio::test]
async fn concurrent_evolve_assigns_distinct_versions() {
    // Two concurrent `EvolveCatalog` calls on the same `from_key` must
    // return *distinct* canonical keys (v2 and v3), proving the
    // registry serializes monotonic version assignment under
    // contention. This is the wire-level surface of SLO D12-CT-6.
    let ts = TestServer::start_no_auth().await;
    let chan = ts.channel().await;
    let mut writer = CatalogServiceClient::new(chan.clone());
    let v1_h = writer
        .register_catalog(sample_register_req("anonymous", "race"))
        .await
        .unwrap()
        .into_inner();

    let mut c1 = CatalogServiceClient::new(chan.clone());
    let mut c2 = CatalogServiceClient::new(chan);

    let op_a = cat::CatalogEvolutionOp {
        op: Some(cat::catalog_evolution_op::Op::AppendClass(
            cat::AppendClassOp {
                cls: Some(cat::CatalogClass {
                    name: "race_a".into(),
                    description: "racer A".into(),
                    canonical_sentences: vec!["alpha".into()],
                    color: "#000000".into(),
                    deprecated: false,
                }),
            },
        )),
    };
    let op_b = cat::CatalogEvolutionOp {
        op: Some(cat::catalog_evolution_op::Op::AppendClass(
            cat::AppendClassOp {
                cls: Some(cat::CatalogClass {
                    name: "race_b".into(),
                    description: "racer B".into(),
                    canonical_sentences: vec!["beta".into()],
                    color: "#111111".into(),
                    deprecated: false,
                }),
            },
        )),
    };
    let (r1, r2) = tokio::join!(
        c1.evolve_catalog(cat::EvolveCatalogRequest {
            from_key: v1_h.key.clone(),
            op: Some(op_a),
            metadata_override: None,
        }),
        c2.evolve_catalog(cat::EvolveCatalogRequest {
            from_key: v1_h.key.clone(),
            op: Some(op_b),
            metadata_override: None,
        }),
    );

    // Both operations must succeed (the registry serializes them).
    let h1 = r1.expect("first evolve succeeds").into_inner();
    let h2 = r2.expect("second evolve succeeds").into_inner();
    let mut keys = [h1.key.as_str(), h2.key.as_str()];
    keys.sort();
    assert_eq!(
        keys,
        ["anonymous/race@v2", "anonymous/race@v3"],
        "evolutions must be assigned distinct, monotonic versions",
    );
    ts.stop().await;
}

// ====================================================================
// Section F — Observability emissions (3 tests)
// ====================================================================

#[tokio::test]
async fn register_emits_audit_counter() {
    let ts = TestServer::start_no_auth().await;
    let mut client = ts.client().await;
    let labels = ["anonymous", "register", "register", "true"];
    let before = efe_telemetry::metrics::d13_instruments::audit_events_total()
        .with_label_values(&labels)
        .get();
    client
        .register_catalog(sample_register_req("anonymous", "obs_register"))
        .await
        .unwrap();
    let after = efe_telemetry::metrics::d13_instruments::audit_events_total()
        .with_label_values(&labels)
        .get();
    assert!(
        after >= before + 1,
        "expected register audit counter to increment ; before={before}, after={after}"
    );
    ts.stop().await;
}

#[tokio::test]
async fn evolve_emits_audit_counter_with_event_type_evolve() {
    let ts = TestServer::start_no_auth().await;
    let mut client = ts.client().await;
    // Labels schema (D13 C-1 fix) :
    //   tenant, event_type, category, principal_present
    // For evolve calls, `category` is `op.audit_tag()` — for an
    // `UpdateThresholds` op that's `"update_thresholds"`, NOT
    // `"evolve"`. The event_type stays `"evolve"`.
    let labels = ["anonymous", "evolve", "update_thresholds", "true"];
    let before = efe_telemetry::metrics::d13_instruments::audit_events_total()
        .with_label_values(&labels)
        .get();
    let v1_h = client
        .register_catalog(sample_register_req("anonymous", "obs_evolve"))
        .await
        .unwrap()
        .into_inner();
    client
        .evolve_catalog(cat::EvolveCatalogRequest {
            from_key: v1_h.key,
            op: Some(cat::CatalogEvolutionOp {
                op: Some(cat::catalog_evolution_op::Op::UpdateThresholds(
                    cat::UpdateThresholdsOp {
                        thresholds: Some(cat::ThresholdPolicy {
                            confidence_high: 0.91,
                            confidence_medium: 0.7,
                        }),
                    },
                )),
            }),
            metadata_override: None,
        })
        .await
        .unwrap();
    let after = efe_telemetry::metrics::d13_instruments::audit_events_total()
        .with_label_values(&labels)
        .get();
    assert!(
        after >= before + 1,
        "expected evolve audit counter to increment ; before={before}, after={after}"
    );
    ts.stop().await;
}

#[tokio::test]
async fn delete_emits_audit_counter_with_event_type_delete() {
    let ts = TestServer::start_no_auth().await;
    let mut client = ts.client().await;
    let labels = ["anonymous", "delete", "delete", "true"];
    let before = efe_telemetry::metrics::d13_instruments::audit_events_total()
        .with_label_values(&labels)
        .get();
    let h = client
        .register_catalog(sample_register_req("anonymous", "obs_delete"))
        .await
        .unwrap()
        .into_inner();
    client
        .delete_catalog(cat::DeleteCatalogRequest {
            key: h.key,
            force: true,
        })
        .await
        .unwrap();
    let after = efe_telemetry::metrics::d13_instruments::audit_events_total()
        .with_label_values(&labels)
        .get();
    assert!(
        after >= before + 1,
        "expected delete audit counter to increment ; before={before}, after={after}"
    );
    ts.stop().await;
}

// ====================================================================
// Section G — Quota enforcement (Audit D12-bis fix H1, gates fix C1)
// ====================================================================
//
// These tests prove that `EFE_CATALOG_MAX_*` env vars (rendered by
// the Helm chart and consumed by `ServerConfig::from_env`) are
// actually wired through to the registry's policy. Pre-fix C1 the
// runtime constructed `InMemoryRegistry::new()` which hardcoded
// `CatalogPolicy::default()`, so any operator-supplied limits were
// silently ignored. Each test below configures a tight quota,
// drives traffic to the boundary, and asserts ResourceExhausted on
// crossing it.

#[tokio::test]
async fn quota_max_names_per_tenant_is_enforced_via_config() {
    // Tighten the per-tenant chain quota to 2 names. Registering a
    // third name MUST be rejected with ResourceExhausted.
    let ts = TestServer::start_no_auth_with_cfg(|cfg| {
        cfg.catalog.policy.max_names_per_tenant = 2;
    })
    .await;
    let mut client = ts.client().await;

    client
        .register_catalog(sample_register_req("anonymous", "alpha"))
        .await
        .expect("first chain accepted");
    client
        .register_catalog(sample_register_req("anonymous", "beta"))
        .await
        .expect("second chain accepted");
    let err = client
        .register_catalog(sample_register_req("anonymous", "gamma"))
        .await
        .expect_err("third chain should exceed max_names_per_tenant=2");
    assert_eq!(
        err.code(),
        tonic::Code::ResourceExhausted,
        "expected ResourceExhausted, got {:?}: {}",
        err.code(),
        err.message(),
    );
    ts.stop().await;
}

#[tokio::test]
async fn quota_max_versions_per_name_is_enforced_via_config() {
    // Tighten max versions to 2. Register v1, evolve to v2, evolve again → must fail.
    let ts = TestServer::start_no_auth_with_cfg(|cfg| {
        cfg.catalog.policy.max_versions_per_name = 2;
    })
    .await;
    let mut client = ts.client().await;

    let v1 = client
        .register_catalog(sample_register_req("anonymous", "history"))
        .await
        .expect("v1 accepted")
        .into_inner();
    let _v2 = client
        .evolve_catalog(cat::EvolveCatalogRequest {
            from_key: v1.key.clone(),
            op: Some(cat::CatalogEvolutionOp {
                op: Some(cat::catalog_evolution_op::Op::UpdateThresholds(
                    cat::UpdateThresholdsOp {
                        thresholds: Some(cat::ThresholdPolicy {
                            confidence_high: 0.9,
                            confidence_medium: 0.7,
                        }),
                    },
                )),
            }),
            metadata_override: None,
        })
        .await
        .expect("v2 accepted (within quota)")
        .into_inner();
    let err = client
        .evolve_catalog(cat::EvolveCatalogRequest {
            from_key: _v2.key,
            op: Some(cat::CatalogEvolutionOp {
                op: Some(cat::catalog_evolution_op::Op::UpdateThresholds(
                    cat::UpdateThresholdsOp {
                        thresholds: Some(cat::ThresholdPolicy {
                            confidence_high: 0.92,
                            confidence_medium: 0.71,
                        }),
                    },
                )),
            }),
            metadata_override: None,
        })
        .await
        .expect_err("v3 should exceed max_versions_per_name=2");
    assert_eq!(err.code(), tonic::Code::ResourceExhausted);
    ts.stop().await;
}

#[tokio::test]
async fn quota_max_classes_per_catalog_is_enforced_via_config() {
    // Tighten classes-per-catalog to 2 (the policy MIN). Register a
    // catalog with 3 classes → must fail with InvalidArgument
    // (schema violation surfaces as InvalidArgument from the
    // registry's `validate_content`).
    let ts = TestServer::start_no_auth_with_cfg(|cfg| {
        cfg.catalog.policy.max_classes_per_catalog = 2;
    })
    .await;
    let mut client = ts.client().await;
    let mut req = sample_register_req("anonymous", "wide");
    let three_classes = vec![
        cat::CatalogClass {
            name: "a".into(),
            description: "".into(),
            canonical_sentences: vec!["alpha".into()],
            color: "".into(),
            deprecated: false,
        },
        cat::CatalogClass {
            name: "b".into(),
            description: "".into(),
            canonical_sentences: vec!["beta".into()],
            color: "".into(),
            deprecated: false,
        },
        cat::CatalogClass {
            name: "c".into(),
            description: "".into(),
            canonical_sentences: vec!["gamma".into()],
            color: "".into(),
            deprecated: false,
        },
    ];
    let mut content = req.content.unwrap();
    content.classes = three_classes;
    req.content = Some(content);
    let err = client
        .register_catalog(req)
        .await
        .expect_err("3 classes should exceed max_classes_per_catalog=2");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    ts.stop().await;
}

#[tokio::test]
async fn quota_max_sentences_per_class_is_enforced_via_config() {
    // Tighten to 1 sentence per class. Register with 2 sentences → must fail.
    let ts = TestServer::start_no_auth_with_cfg(|cfg| {
        cfg.catalog.policy.max_sentences_per_class = 1;
    })
    .await;
    let mut client = ts.client().await;
    let req = sample_register_req("anonymous", "wide_class"); // sample has 2 sentences in `billing`.
    let err = client
        .register_catalog(req)
        .await
        .expect_err("2 sentences should exceed max_sentences_per_class=1");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    ts.stop().await;
}

// ====================================================================
// Section H — Mandatory cone_policy + thresholds (Audit fix M1)
// ====================================================================
//
// Pre-fix M1 the handler silently defaulted missing `thresholds` to
// `(0.0, 0.0)` which the registry rejected with the opaque message
// "confidence_medium must be in (0,1) (got 0)" — confusing because
// the client never sent 0. The fix surfaces the missing-field error
// at the conversion boundary with a clear "thresholds is required"
// InvalidArgument.

#[tokio::test]
async fn register_rejects_missing_thresholds_with_clear_error() {
    let ts = TestServer::start_no_auth().await;
    let mut client = ts.client().await;
    let mut req = sample_register_req("anonymous", "no_thresholds");
    let mut content = req.content.unwrap();
    content.thresholds = None;
    req.content = Some(content);
    let err = client.register_catalog(req).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    assert!(
        err.message().contains("thresholds"),
        "expected error mentioning `thresholds`, got: {}",
        err.message(),
    );
    ts.stop().await;
}

#[tokio::test]
async fn register_rejects_missing_cone_policy_with_clear_error() {
    let ts = TestServer::start_no_auth().await;
    let mut client = ts.client().await;
    let mut req = sample_register_req("anonymous", "no_cone");
    let mut content = req.content.unwrap();
    content.cone_policy = None;
    req.content = Some(content);
    let err = client.register_catalog(req).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    assert!(
        err.message().contains("cone_policy"),
        "expected error mentioning `cone_policy`, got: {}",
        err.message(),
    );
    ts.stop().await;
}

// ====================================================================
// Bonus — Health probe sanity check
// ====================================================================
//
// Catalog service is added to the health reporter unconditionally
// (runtime.rs `set_serving::<CatalogServiceServer<…>>`). We don't
// import the tonic-health client here ; the smoke test is implicit
// in `start_no_auth` succeeding — if the catalog server failed to
// attach the listener never binds.

// EOF
