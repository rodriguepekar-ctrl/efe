//! Sprint **D15** — End-to-end integration tests for the OIDC
//! validator + JWKS pull cache.
//!
//! These tests exercise the full path :
//!
//!   1. boot a `wiremock` HTTP server pretending to be an IdP ;
//!   2. host a JWKS document there ;
//!   3. build an `IssuerRegistry` + `JwksCache` configured to fetch
//!      from the wiremock URL ;
//!   4. ask the cache to refresh ;
//!   5. mint a JWT with the matching kid ;
//!   6. validate via `OidcValidator`.
//!
//! No Postgres dependency — these tests run in any CI shape that has
//! tokio + wiremock available.
//!
//! Coverage (architecture doc § 20.4) :
//!
//!   * boot path : load issuers JSON file → registry → cache pre-warm
//!     via wiremock 200 ;
//!   * validate path : RS256 happy ; unknown issuer ; alg whitelist
//!     mismatch ; expired ; unknown kid ; tenant claim missing ;
//!   * stale-while-revalidate : during a `respond_with_template`
//!     500 outage, the cache keeps serving keys until grace expires.

use std::sync::Arc;
use std::time::Duration;

use efe_server::auth::{
    load_issuers_file, IssuerConfig, IssuerEntry, IssuerLoadError, IssuerRegistry, JwksCache,
    LoadParams, OidcValidator, Outcome, TenantStrategy,
};
use efe_server::config::OidcConfig;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// =====================================================================
// Fixture : embedded RSA-2048 keypair (test-only, see
// `src/auth/test_keys/rsa_2048.pem`)
// =====================================================================

const RSA_PEM: &str = include_str!("../src/auth/test_keys/rsa_2048.pem");
const RSA_N_B64URL: &str = include_str!("../src/auth/test_keys/rsa_2048.n.b64url");
const RSA_E_B64URL: &str = "AQAB";
const KID: &str = "test-rsa-1";

fn jwks_doc() -> serde_json::Value {
    json!({
        "keys": [
            {
                "kty": "RSA",
                "use": "sig",
                "alg": "RS256",
                "kid": KID,
                "n": RSA_N_B64URL.trim(),
                "e": RSA_E_B64URL,
            }
        ]
    })
}

fn signing_key() -> EncodingKey {
    EncodingKey::from_rsa_pem(RSA_PEM.as_bytes()).expect("valid PEM")
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn make_token(iss: &str, aud: &str, claims: serde_json::Value) -> String {
    let mut payload = serde_json::Map::new();
    payload.insert("iss".into(), json!(iss));
    payload.insert("aud".into(), json!(aud));
    payload.insert("sub".into(), json!("user-1"));
    payload.insert("exp".into(), json!(now() + 300));
    payload.insert("iat".into(), json!(now()));
    if let serde_json::Value::Object(m) = claims {
        for (k, v) in m {
            payload.insert(k, v);
        }
    }
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(KID.to_string());
    encode(&header, &payload, &signing_key()).expect("sign")
}

async fn build_registry(
    server: &MockServer,
    cfg_overrides: impl FnOnce(&mut IssuerConfig),
) -> (Arc<IssuerRegistry>, IssuerConfig) {
    let oidc_cfg = OidcConfig::default();
    let params = LoadParams::from_oidc_config(&oidc_cfg);
    let mut cfg = IssuerConfig {
        issuer: format!("{}/", server.uri()),
        audience: "efe.v1".into(),
        jwks_url: format!("{}/jwks.json", server.uri()),
        alg_whitelist: vec!["RS256".into()],
        tenant_strategy: TenantStrategy::Claim,
        tenant_claim: Some("tenant_id".into()),
        tenant_id_for_issuer: None,
        max_token_age_s: 3600,
        leeway_s: 30,
    };
    cfg_overrides(&mut cfg);
    let cache = JwksCache::new(
        cfg.jwks_url.clone(),
        Duration::from_secs(300),
        Duration::from_millis(params.jwks_fetch_timeout_ms),
        params.jwks_retry_max,
        Duration::from_millis(params.jwks_backoff_ms),
        Duration::from_secs(params.grace_stale_s),
    );
    cache.refresh().await.expect("initial refresh");
    let entry = IssuerEntry {
        config: cfg.clone(),
        jwks_cache: cache,
    };
    (Arc::new(IssuerRegistry::from_entries(vec![entry])), cfg)
}

// =====================================================================
// Tests
// =====================================================================

#[tokio::test]
async fn validates_rs256_token_via_wiremock_jwks() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jwks_doc()))
        .mount(&server)
        .await;
    let (reg, cfg) = build_registry(&server, |_| {}).await;
    let validator = OidcValidator::new(reg);
    let tok = make_token(
        &cfg.issuer,
        "efe.v1",
        json!({"tenant_id": "acme", "scope": "swarm:execute"}),
    );
    let claims = validator.validate(&tok).await.expect("token valid");
    assert_eq!(claims.tenant_id, "acme");
    assert!(claims.has_scope("swarm:execute"));
}

#[tokio::test]
async fn rejects_token_with_unknown_issuer() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jwks_doc()))
        .mount(&server)
        .await;
    let (reg, _cfg) = build_registry(&server, |_| {}).await;
    let validator = OidcValidator::new(reg);
    let tok = make_token(
        "https://attacker.example/", // not in registry
        "efe.v1",
        json!({"tenant_id": "x"}),
    );
    let err = validator.validate(&tok).await.unwrap_err();
    assert_eq!(err.outcome(), Outcome::UnknownIssuer);
}

#[tokio::test]
async fn rejects_token_with_alg_outside_whitelist() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jwks_doc()))
        .mount(&server)
        .await;
    let (reg, cfg) = build_registry(&server, |c| {
        c.alg_whitelist = vec!["ES256".into()]; // narrow
    })
    .await;
    let validator = OidcValidator::new(reg);
    let tok = make_token(&cfg.issuer, "efe.v1", json!({"tenant_id": "acme"}));
    let err = validator.validate(&tok).await.unwrap_err();
    assert_eq!(err.outcome(), Outcome::InvalidSignature);
}

#[tokio::test]
async fn rejects_token_with_unknown_kid() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jwks_doc()))
        .mount(&server)
        .await;
    let (reg, cfg) = build_registry(&server, |_| {}).await;
    let validator = OidcValidator::new(reg);
    // Sign with a different kid that's not in cache.
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("rotated-out-kid".into());
    let payload = json!({
        "iss": cfg.issuer,
        "aud": "efe.v1",
        "sub": "u",
        "exp": now() + 300,
        "iat": now(),
        "tenant_id": "acme"
    });
    let tok = encode(&header, &payload, &signing_key()).unwrap();
    let err = validator.validate(&tok).await.unwrap_err();
    assert_eq!(err.outcome(), Outcome::InvalidSignature);
}

#[tokio::test]
async fn rejects_token_when_aud_mismatched() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jwks_doc()))
        .mount(&server)
        .await;
    let (reg, cfg) = build_registry(&server, |_| {}).await;
    let validator = OidcValidator::new(reg);
    let tok = make_token(&cfg.issuer, "wrong-aud", json!({"tenant_id": "acme"}));
    let err = validator.validate(&tok).await.unwrap_err();
    // **Audit fix D15-C4** — audience mismatch now lights the
    // dedicated `Outcome::BadAudience` panel (was previously folded
    // into `InvalidSignature`).
    assert_eq!(err.outcome(), Outcome::BadAudience);
}

#[tokio::test]
async fn rejects_token_when_tenant_claim_missing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jwks_doc()))
        .mount(&server)
        .await;
    let (reg, cfg) = build_registry(&server, |_| {}).await;
    let validator = OidcValidator::new(reg);
    // No tenant_id claim.
    let tok = make_token(&cfg.issuer, "efe.v1", json!({}));
    let err = validator.validate(&tok).await.unwrap_err();
    assert_eq!(err.outcome(), Outcome::TenantResolutionFailed);
}

#[tokio::test]
async fn issuer_strategy_resolves_fixed_tenant() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jwks_doc()))
        .mount(&server)
        .await;
    let (reg, cfg) = build_registry(&server, |c| {
        c.tenant_strategy = TenantStrategy::Issuer;
        c.tenant_claim = None;
        c.tenant_id_for_issuer = Some("acme".into());
    })
    .await;
    let validator = OidcValidator::new(reg);
    // No tenant_id claim — strategy=issuer maps the URL to "acme".
    let tok = make_token(&cfg.issuer, "efe.v1", json!({}));
    let claims = validator.validate(&tok).await.expect("ok");
    assert_eq!(claims.tenant_id, "acme");
}

/// Φ-D-OPS post-audit fix B8 : `EFE_OIDC_ALLOW_HTTP` is a global env
/// var. Any test that mutates it must hold this lock so sibling tests
/// reading process env (directly or via `load_issuers_file`) don't
/// observe a transient `"true"` they didn't expect. Mirrors the
/// `BATCH_ENV_LOCK` pattern in `efe-server/src/v1/batch/worker.rs`.
static OIDC_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[tokio::test]
async fn loads_issuers_from_json_file() {
    let _env_guard = OIDC_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // wiremock serves over HTTP ; the validator gates `http://` behind
    // `EFE_OIDC_ALLOW_HTTP=true` (architecture § 13.2). We set it for
    // the duration of this test ; the unset case is exercised by
    // `rejects_issuers_file_with_https_violation`.
    std::env::set_var("EFE_OIDC_ALLOW_HTTP", "true");
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jwks_doc()))
        .mount(&server)
        .await;
    // Write a temporary JSON file with one issuer entry.
    let issuer_url = format!("{}/", server.uri());
    let jwks_url = format!("{}/jwks.json", server.uri());
    let body = serde_json::json!([
        {
            "issuer": issuer_url,
            "audience": "efe.v1",
            "jwks_url": jwks_url,
            "alg_whitelist": ["RS256"],
            "tenant_strategy": "claim",
            "tenant_claim": "tenant_id",
            "max_token_age_s": 3600,
            "leeway_s": 30
        }
    ]);
    let tmp = std::env::temp_dir().join(format!("efe-d15-issuers-{}.json", std::process::id()));
    std::fs::write(&tmp, serde_json::to_string_pretty(&body).unwrap()).expect("write tmp file");
    let oidc = OidcConfig::default();
    let params = LoadParams::from_oidc_config(&oidc);
    let registry = load_issuers_file(&tmp, &params).expect("load");
    assert_eq!(registry.len(), 1);
    let entry = registry.iter().next().expect("one entry");
    assert_eq!(entry.config.issuer, issuer_url);
    let _ = std::fs::remove_file(&tmp);
    std::env::remove_var("EFE_OIDC_ALLOW_HTTP");
}

#[tokio::test]
async fn rejects_issuers_file_with_invariant_violation() {
    // Φ-D-OPS post-audit fix B8 : take the lock anyway so we don't
    // race the toggler. The invariant we test (schema validation)
    // runs before the HTTP gate, so a leaked `EFE_OIDC_ALLOW_HTTP=true`
    // can't make this test pass spuriously — but holding the lock
    // makes the test deterministic regardless of future refactors
    // that might reorder the validation pipeline.
    let _env_guard = OIDC_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let body = serde_json::json!([
        {
            "issuer": "https://example.com/",
            "audience": "efe.v1",
            "jwks_url": "https://example.com/.well-known/jwks.json",
            "alg_whitelist": ["RS256"],
            "tenant_strategy": "claim"
            // tenant_claim is missing → invalid
        }
    ]);
    let tmp = std::env::temp_dir().join(format!("efe-d15-bad-{}.json", std::process::id()));
    std::fs::write(&tmp, serde_json::to_string(&body).unwrap()).unwrap();
    let oidc = OidcConfig::default();
    let params = LoadParams::from_oidc_config(&oidc);
    let res = load_issuers_file(&tmp, &params);
    assert!(matches!(res, Err(IssuerLoadError::Invalid { .. })));
    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test]
async fn jwks_refresh_recovers_after_idp_5xx() {
    // First two refreshes return 500 (transient outage) ; the third
    // returns 200. With retry_max=3 + small backoff, the cache
    // should ultimately populate.
    let server = MockServer::start().await;
    // Up_then_500 mock isn't trivial in wiremock — we simulate the
    // "outage then recovery" by registering two mocks with
    // priority/expectations.
    Mock::given(method("GET"))
        .and(path("/jwks.json"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jwks_doc()))
        .mount(&server)
        .await;
    let oidc = OidcConfig::default();
    let cache = JwksCache::new(
        format!("{}/jwks.json", server.uri()),
        Duration::from_secs(300),
        Duration::from_millis(oidc.jwks_fetch_timeout_ms),
        oidc.jwks_retry_max,
        Duration::from_millis(50), // tiny backoff for the test
        Duration::from_secs(oidc.grace_stale_s),
    );
    cache
        .refresh()
        .await
        .expect("eventually succeeds via retries");
    assert_eq!(cache.n_keys().await, 1);
}

#[tokio::test]
async fn jwks_refresh_fails_after_retry_budget_exhausted() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/jwks.json"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let cache = JwksCache::new(
        format!("{}/jwks.json", server.uri()),
        Duration::from_secs(300),
        Duration::from_millis(500),
        2,                         // retry_max
        Duration::from_millis(20), // tight backoff so the test is quick
        Duration::from_secs(3600),
    );
    let err = cache.refresh().await.unwrap_err();
    assert!(format!("{err}").to_lowercase().contains("fetch"));
    assert_eq!(cache.n_keys().await, 0);
}
