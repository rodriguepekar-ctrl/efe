//! Sprint **D15** — JWKS fetch + cache.
//!
//! Mirror of `docs/bench/d15_onboarding_idp_architecture.md` § 12 + § 6.
//!
//! # Lifecycle
//!
//! 1. `JwksCache::new` — builds the cache lazily ; no HTTP yet.
//! 2. `JwksCache::refresh()` — async fetch + parse + store.
//! 3. `JwksCache::get_key(kid)` — sync read from the cache. Returns
//!    `None` if not present (the verify path then either retries
//!    after a forced refresh or rejects with `UnknownKid`).
//! 4. `periodic_refresh(cache, ttl, shutdown)` — background task
//!    spawned by the runtime ; refresh every `ttl` seconds, exits
//!    on shutdown.
//!
//! # Stale-while-revalidate
//!
//! When a fetch fails (IdP down, network blip), the cache returns
//! the **last successfully fetched** keys with `stale=true` — as
//! long as `now - last_success < ttl + grace_stale`. Past the
//! grace window, the cache returns `Err(JwksFetch)` and the verify
//! path rejects new tokens with `Unauthenticated`.
//!
//! # Error message hygiene
//!
//! Error messages NEVER include the JWKS URL fragments that could
//! leak operator infrastructure layout (subdomain enumeration). We
//! log full URLs at `tracing::error!` server-side ; the caller
//! receives an opaque `JwksFetch("network error")` style error.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use jsonwebtoken::{Algorithm, DecodingKey};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::RwLock;

// =====================================================================
// JwksDocument — what the IdP returns at /.well-known/jwks.json
// =====================================================================

/// Public-key set as defined by RFC 7517.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JwksDocument {
    pub keys: Vec<Jwk>,
}

/// One JSON Web Key. RFC 7517 § 4. We only parse the subset we use
/// (`kty`, `alg`, `kid`, `use`, `n`/`e` for RSA, `crv`/`x`/`y` for EC).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Jwk {
    pub kty: String,
    #[serde(default)]
    pub alg: Option<String>,
    pub kid: String,
    #[serde(rename = "use", default)]
    pub use_: Option<String>,
    // RSA fields.
    #[serde(default)]
    pub n: Option<String>,
    #[serde(default)]
    pub e: Option<String>,
    // EC fields.
    #[serde(default)]
    pub crv: Option<String>,
    #[serde(default)]
    pub x: Option<String>,
    #[serde(default)]
    pub y: Option<String>,
}

impl Jwk {
    /// Convert this JWK to a `jsonwebtoken::DecodingKey`. Fails if
    /// the key parameters are missing or the kty/alg combo isn't
    /// supported (RS256 RSA or ES256 EC P-256).
    pub fn to_decoding_key(&self) -> Result<(DecodingKey, Algorithm), JwksError> {
        match self.kty.as_str() {
            "RSA" => {
                let n = self.n.as_deref().ok_or_else(|| {
                    JwksError::ParseDocument(format!("RSA jwk {:?} missing `n`", self.kid))
                })?;
                let e = self.e.as_deref().ok_or_else(|| {
                    JwksError::ParseDocument(format!("RSA jwk {:?} missing `e`", self.kid))
                })?;
                let key = DecodingKey::from_rsa_components(n, e).map_err(|err| {
                    JwksError::ParseDocument(format!(
                        "RSA jwk {:?} invalid components: {err}",
                        self.kid
                    ))
                })?;
                Ok((key, Algorithm::RS256))
            }
            "EC" => {
                let crv = self.crv.as_deref().ok_or_else(|| {
                    JwksError::ParseDocument(format!("EC jwk {:?} missing `crv`", self.kid))
                })?;
                if crv != "P-256" {
                    return Err(JwksError::ParseDocument(format!(
                        "EC jwk {:?} crv {:?} unsupported (P-256 only in v0.1.15)",
                        self.kid, crv
                    )));
                }
                let x = self.x.as_deref().ok_or_else(|| {
                    JwksError::ParseDocument(format!("EC jwk {:?} missing `x`", self.kid))
                })?;
                let y = self.y.as_deref().ok_or_else(|| {
                    JwksError::ParseDocument(format!("EC jwk {:?} missing `y`", self.kid))
                })?;
                let key = DecodingKey::from_ec_components(x, y).map_err(|err| {
                    JwksError::ParseDocument(format!(
                        "EC jwk {:?} invalid components: {err}",
                        self.kid
                    ))
                })?;
                Ok((key, Algorithm::ES256))
            }
            other => Err(JwksError::ParseDocument(format!(
                "jwk {:?} kty {:?} unsupported (RSA / EC P-256 only)",
                self.kid, other
            ))),
        }
    }
}

// =====================================================================
// Cache state
// =====================================================================

#[derive(Default)]
struct CacheState {
    /// kid → (DecodingKey, Algorithm) — populated by refresh.
    keys: HashMap<String, (DecodingKey, Algorithm)>,
    /// Wall-clock instant of the last successful refresh. `None`
    /// means "never refreshed" (cache miss serves an Err).
    last_success_at: Option<Instant>,
    /// Number of consecutive fetch failures since the last success.
    consecutive_failures: u32,
}

/// Sprint **D15** — JWKS cache with periodic refresh + grace stale.
///
/// Cloning is cheap : the inner state is `Arc<RwLock>`. Multiple
/// cache handles share the same data ; the runtime gives one to
/// each `IssuerEntry` and one to the periodic refresh task.
#[derive(Clone)]
pub struct JwksCache {
    inner: Arc<JwksCacheInner>,
}

struct JwksCacheInner {
    jwks_url: String,
    ttl: Duration,
    fetch_timeout: Duration,
    retry_max: u32,
    backoff_base: Duration,
    grace_stale: Duration,
    state: RwLock<CacheState>,
    fetcher: Box<dyn JwksFetcher>,
}

impl std::fmt::Debug for JwksCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JwksCache")
            .field("jwks_url", &self.inner.jwks_url)
            .field("ttl_s", &self.inner.ttl.as_secs())
            .field("retry_max", &self.inner.retry_max)
            .finish()
    }
}

// =====================================================================
// Fetcher trait — swappable for tests (wiremock) and production (reqwest)
// =====================================================================

/// HTTP fetcher abstraction. Production uses
/// [`HttpJwksFetcher`] backed by `reqwest`. Tests use a mock that
/// returns canned [`JwksDocument`] payloads.
#[async_trait::async_trait]
pub trait JwksFetcher: Send + Sync {
    async fn fetch(&self, url: &str, timeout: Duration) -> Result<JwksDocument, JwksError>;
}

/// Production fetcher : `reqwest` over rustls.
pub struct HttpJwksFetcher {
    client: reqwest::Client,
}

impl HttpJwksFetcher {
    /// Build a new fetcher. The `reqwest::Client` is reusable across
    /// fetches and pools connections.
    #[must_use]
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(concat!("efe-server/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { client }
    }
}

impl Default for HttpJwksFetcher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl JwksFetcher for HttpJwksFetcher {
    async fn fetch(&self, url: &str, timeout: Duration) -> Result<JwksDocument, JwksError> {
        let resp = self
            .client
            .get(url)
            .timeout(timeout)
            .send()
            .await
            .map_err(|e| JwksError::Fetch(format!("HTTP send error: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(JwksError::Fetch(format!(
                "HTTP {} from JWKS endpoint",
                status.as_u16()
            )));
        }
        let body: JwksDocument = resp
            .json()
            .await
            .map_err(|e| JwksError::ParseDocument(format!("invalid JWKS JSON body: {e}")))?;
        Ok(body)
    }
}

// =====================================================================
// Cache impl
// =====================================================================

impl JwksCache {
    /// Construct a cache backed by [`HttpJwksFetcher`]. The cache is
    /// **not pre-warmed** — call `refresh()` once or wait for the
    /// first verify-path lookup.
    #[must_use]
    pub fn new(
        jwks_url: String,
        ttl: Duration,
        fetch_timeout: Duration,
        retry_max: u32,
        backoff_base: Duration,
        grace_stale: Duration,
    ) -> Self {
        Self::with_fetcher(
            jwks_url,
            ttl,
            fetch_timeout,
            retry_max,
            backoff_base,
            grace_stale,
            Box::new(HttpJwksFetcher::new()),
        )
    }

    /// Construct with a custom fetcher (used by tests / sidecar).
    #[must_use]
    pub fn with_fetcher(
        jwks_url: String,
        ttl: Duration,
        fetch_timeout: Duration,
        retry_max: u32,
        backoff_base: Duration,
        grace_stale: Duration,
        fetcher: Box<dyn JwksFetcher>,
    ) -> Self {
        Self {
            inner: Arc::new(JwksCacheInner {
                jwks_url,
                ttl,
                fetch_timeout,
                retry_max,
                backoff_base,
                grace_stale,
                state: RwLock::new(CacheState::default()),
                fetcher,
            }),
        }
    }

    /// Sync lookup of a `kid` in the cache. Returns the
    /// `(DecodingKey, Algorithm)` pair if cached, `None` otherwise.
    /// The verify path on miss can either reject with
    /// `Unauthenticated` (typical) or trigger a forced async refresh
    /// (slow path).
    ///
    /// **Stale-while-revalidate** : if the cache is older than `ttl`
    /// but younger than `ttl + grace_stale`, we still return the key
    /// AND increment `efe_jwks_stale_total` so ops can correlate
    /// stale serves with the IdP outage.
    pub async fn get_key(&self, kid: &str) -> Option<(DecodingKey, Algorithm)> {
        let st = self.inner.state.read().await;
        // **Audit fix D15-H8** — explicit `if let Some(...)` instead of
        // `unwrap()` so the `#![cfg_attr(not(test), deny(clippy::unwrap_used))]`
        // lint passes without exception.
        let last_success = st.last_success_at?;
        let age = last_success.elapsed();
        let ttl = self.inner.ttl;
        let grace = self.inner.grace_stale;
        if age > ttl + grace {
            // Past the grace window — refuse stale serves.
            return None;
        }
        // **Audit fix D15-C6+C7** — the previous implementation
        // incremented `jwks_stale_total` *before* checking whether the
        // requested kid was actually in the cache. A token with a
        // rotated-out kid against a stale cache produced a spurious
        // increment AND returned `None` ; ops saw "stale serves
        // climbing" but no traffic was actually using stale keys.
        // Now we increment ONLY after we confirm the kid is present
        // and we're about to serve it stale.
        let key = st.keys.get(kid).cloned()?;
        if age > ttl {
            efe_telemetry::metrics::d15_instruments::jwks_stale_total()
                .with_label_values(&[&self.inner.jwks_url])
                .inc();
        }
        Some(key)
    }

    /// Force a refresh now. Retries up to `retry_max` times with
    /// exponential backoff. On success, updates the cache + bumps
    /// `last_success_at`.
    ///
    /// The retry is **per-call**, not per-process : if the periodic
    /// refresh task fires every 5 min and each call retries 3× with
    /// 1s/2s/4s backoff, the worst-case attempt window is ~7 s.
    /// Past `retry_max`, we surface the last error to the caller.
    pub async fn refresh(&self) -> Result<(), JwksError> {
        let mut last_err: Option<JwksError> = None;
        for attempt in 0..=self.inner.retry_max {
            if attempt > 0 {
                let backoff = self.inner.backoff_base * 2u32.saturating_pow(attempt - 1);
                tokio::time::sleep(backoff).await;
            }
            match self
                .inner
                .fetcher
                .fetch(&self.inner.jwks_url, self.inner.fetch_timeout)
                .await
            {
                Ok(doc) => {
                    let mut keys: HashMap<String, (DecodingKey, Algorithm)> =
                        HashMap::with_capacity(doc.keys.len());
                    for jwk in &doc.keys {
                        // Skip jwks where `use` is set and ≠ "sig".
                        if let Some(u) = &jwk.use_ {
                            if u != "sig" {
                                continue;
                            }
                        }
                        match jwk.to_decoding_key() {
                            Ok((dk, alg)) => {
                                keys.insert(jwk.kid.clone(), (dk, alg));
                            }
                            Err(e) => {
                                tracing::warn!(
                                    target: "efe.auth.jwks",
                                    err = %e,
                                    kid = %jwk.kid,
                                    "skipping unparseable JWK"
                                );
                            }
                        }
                    }
                    let n = keys.len();
                    let mut st = self.inner.state.write().await;
                    st.keys = keys;
                    st.last_success_at = Some(Instant::now());
                    st.consecutive_failures = 0;
                    drop(st);
                    efe_telemetry::metrics::d15_instruments::jwks_fetch_total()
                        .with_label_values(&[&self.inner.jwks_url, "ok"])
                        .inc();
                    efe_telemetry::metrics::d15_instruments::jwks_cache_size()
                        .with_label_values(&[&self.inner.jwks_url])
                        .set(n as i64);
                    tracing::info!(
                        target: "efe.auth.jwks",
                        url = %self.inner.jwks_url,
                        n_keys = n,
                        attempt = attempt + 1,
                        "JWKS refresh succeeded"
                    );
                    return Ok(());
                }
                Err(e) => {
                    let result_label = match &e {
                        JwksError::Fetch(_) => "fetch_error",
                        JwksError::ParseDocument(_) => "parse_error",
                    };
                    efe_telemetry::metrics::d15_instruments::jwks_fetch_total()
                        .with_label_values(&[&self.inner.jwks_url, result_label])
                        .inc();
                    tracing::warn!(
                        target: "efe.auth.jwks",
                        url = %self.inner.jwks_url,
                        err = %e,
                        attempt = attempt + 1,
                        retry_max = self.inner.retry_max,
                        "JWKS refresh attempt failed"
                    );
                    last_err = Some(e);
                }
            }
        }
        let mut st = self.inner.state.write().await;
        st.consecutive_failures = st.consecutive_failures.saturating_add(1);
        drop(st);
        Err(last_err.unwrap_or_else(|| JwksError::Fetch("retry budget exhausted".to_string())))
    }

    /// Cache age in seconds since the last successful refresh.
    /// Returns `None` if never refreshed. Used by the periodic ticker
    /// to publish the `efe_jwks_cache_age_seconds` gauge.
    pub async fn age_seconds(&self) -> Option<u64> {
        let st = self.inner.state.read().await;
        st.last_success_at.map(|t| t.elapsed().as_secs())
    }

    /// Number of distinct kids in the cache. Used by the periodic
    /// ticker to publish the `efe_jwks_cache_size` gauge.
    pub async fn n_keys(&self) -> usize {
        self.inner.state.read().await.keys.len()
    }

    /// Accessor for the JWKS URL — exposed for the periodic ticker
    /// metric labels.
    #[must_use]
    pub fn jwks_url(&self) -> &str {
        &self.inner.jwks_url
    }
}

// =====================================================================
// Periodic refresh task
// =====================================================================

/// Background task : refresh the cache every `ttl` seconds. Exits
/// on `shutdown.notified()`.
///
/// Spawned by `runtime::Server::serve` for each `IssuerEntry`'s cache.
pub async fn periodic_refresh(
    cache: JwksCache,
    interval: Duration,
    shutdown: crate::shutdown::Shutdown,
) {
    // Fire one refresh immediately at startup so the cache is warm
    // for the first verify call. Errors logged but not fatal — the
    // server still boots ; tokens will fail verify until the next
    // tick succeeds.
    if let Err(e) = cache.refresh().await {
        tracing::warn!(
            target: "efe.auth.jwks",
            url = %cache.inner.jwks_url,
            err = %e,
            "initial JWKS refresh failed ; tokens will be rejected until next tick"
        );
    }

    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    ticker.tick().await; // skip immediate (we just refreshed above)
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if let Err(e) = cache.refresh().await {
                    tracing::warn!(
                        target: "efe.auth.jwks",
                        url = %cache.inner.jwks_url,
                        err = %e,
                        "JWKS refresh failed ; cache may go stale"
                    );
                }
                if let Some(age) = cache.age_seconds().await {
                    efe_telemetry::metrics::d15_instruments::jwks_cache_age_seconds()
                        .with_label_values(&[&cache.inner.jwks_url])
                        .set(age as i64);
                }
            }
            _ = shutdown.notified() => {
                tracing::info!(
                    target: "efe.auth.jwks",
                    url = %cache.inner.jwks_url,
                    "shutdown — periodic JWKS refresh exiting"
                );
                return;
            }
        }
    }
}

// =====================================================================
// Errors
// =====================================================================

/// Error type returned by [`JwksCache::refresh`] and [`JwksFetcher::fetch`].
///
/// Carries human-readable detail in the `String` payload — never the
/// raw response body (which can leak private data when the IdP
/// returns an HTML error page).
#[derive(Debug, Error)]
pub enum JwksError {
    /// Network / TLS / HTTP-level error during the JWKS HTTP fetch.
    /// Examples : DNS failure, TCP timeout, HTTP 4xx / 5xx,
    /// non-200 with `Content-Type: text/html`.
    #[error("JWKS fetch failed: {0}")]
    Fetch(String),

    /// The HTTP response was 200 but the body was not a valid JWKS
    /// document (RFC 7517) : missing `keys` field, malformed JSON,
    /// invalid UTF-8.
    #[error("JWKS document parse error: {0}")]
    ParseDocument(String),
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Mock fetcher that returns canned responses (or errors) per
    /// call. Used by unit tests that don't need wiremock.
    struct MockFetcher {
        responses: Mutex<Vec<Result<JwksDocument, JwksError>>>,
    }

    impl MockFetcher {
        fn new(responses: Vec<Result<JwksDocument, JwksError>>) -> Self {
            Self {
                responses: Mutex::new(responses),
            }
        }
    }

    #[async_trait::async_trait]
    impl JwksFetcher for MockFetcher {
        async fn fetch(&self, _url: &str, _timeout: Duration) -> Result<JwksDocument, JwksError> {
            let mut q = self.responses.lock().unwrap();
            if q.is_empty() {
                return Err(JwksError::Fetch("mock: queue empty".to_string()));
            }
            q.remove(0)
        }
    }

    fn empty_doc() -> JwksDocument {
        JwksDocument { keys: vec![] }
    }

    /// A minimal RSA JWK that parses through `from_rsa_components`.
    /// `n` and `e` come from the `jsonwebtoken` test vectors so
    /// validate isn't needed at unit-test time.
    fn sample_rsa_jwk(kid: &str) -> Jwk {
        Jwk {
            kty: "RSA".to_string(),
            alg: Some("RS256".to_string()),
            kid: kid.to_string(),
            use_: Some("sig".to_string()),
            // Test vector — 2048-bit RSA pub key (publicly published
            // jsonwebtoken examples).
            n: Some(
                "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw".to_string()
            ),
            e: Some("AQAB".to_string()),
            crv: None,
            x: None,
            y: None,
        }
    }

    fn make_cache(fetcher: MockFetcher) -> JwksCache {
        JwksCache::with_fetcher(
            "https://mock.example/.well-known/jwks.json".to_string(),
            Duration::from_secs(300),
            Duration::from_millis(100),
            2,
            Duration::from_millis(10),
            Duration::from_secs(60),
            Box::new(fetcher),
        )
    }

    #[tokio::test]
    async fn refresh_populates_cache() {
        let doc = JwksDocument {
            keys: vec![sample_rsa_jwk("k1"), sample_rsa_jwk("k2")],
        };
        let cache = make_cache(MockFetcher::new(vec![Ok(doc)]));
        cache.refresh().await.unwrap();
        assert_eq!(cache.n_keys().await, 2);
        assert!(cache.get_key("k1").await.is_some());
        assert!(cache.get_key("k2").await.is_some());
        assert!(cache.get_key("unknown").await.is_none());
    }

    #[tokio::test]
    async fn refresh_retries_on_failure() {
        let cache = make_cache(MockFetcher::new(vec![
            Err(JwksError::Fetch("timeout 1".into())),
            Err(JwksError::Fetch("timeout 2".into())),
            Ok(JwksDocument {
                keys: vec![sample_rsa_jwk("k1")],
            }),
        ]));
        cache.refresh().await.unwrap();
        assert_eq!(cache.n_keys().await, 1);
    }

    #[tokio::test]
    async fn refresh_exhausts_retries_and_errors() {
        let cache = make_cache(MockFetcher::new(vec![
            Err(JwksError::Fetch("a".into())),
            Err(JwksError::Fetch("b".into())),
            Err(JwksError::Fetch("c".into())),
        ]));
        let err = cache.refresh().await.unwrap_err();
        assert!(matches!(err, JwksError::Fetch(_)));
    }

    #[tokio::test]
    async fn get_key_before_refresh_returns_none() {
        let cache = make_cache(MockFetcher::new(vec![]));
        assert!(cache.get_key("k1").await.is_none());
    }

    #[tokio::test]
    async fn refresh_skips_use_enc_keys() {
        let mut k = sample_rsa_jwk("k_enc");
        k.use_ = Some("enc".to_string());
        let doc = JwksDocument {
            keys: vec![k, sample_rsa_jwk("k_sig")],
        };
        let cache = make_cache(MockFetcher::new(vec![Ok(doc)]));
        cache.refresh().await.unwrap();
        assert_eq!(cache.n_keys().await, 1);
        assert!(cache.get_key("k_sig").await.is_some());
        assert!(cache.get_key("k_enc").await.is_none());
    }

    #[test]
    fn jwk_to_decoding_key_rsa_round_trip() {
        let jwk = sample_rsa_jwk("k1");
        // `DecodingKey` doesn't implement `Debug`, so we can't call
        // `.unwrap()` on the `Result` ; pattern-match instead.
        match jwk.to_decoding_key() {
            Ok((_dk, alg)) => assert_eq!(alg, Algorithm::RS256),
            Err(e) => panic!("expected Ok, got error: {e}"),
        }
    }

    #[test]
    fn jwk_to_decoding_key_unknown_kty_fails() {
        let mut jwk = sample_rsa_jwk("k1");
        jwk.kty = "OKP".to_string();
        // Same Debug-bound limitation as above ; match on the `Err`.
        match jwk.to_decoding_key() {
            Ok(_) => panic!("expected error for unknown kty=OKP"),
            Err(e) => assert!(matches!(e, JwksError::ParseDocument(_))),
        }
    }

    #[tokio::test]
    async fn refresh_empty_doc_yields_empty_cache_but_succeeds() {
        let cache = make_cache(MockFetcher::new(vec![Ok(empty_doc())]));
        cache.refresh().await.unwrap();
        assert_eq!(cache.n_keys().await, 0);
    }

    /// **Audit fix D15-C6+C7 regression test** — `get_key` for an
    /// unknown kid against a stale-but-within-grace cache MUST NOT
    /// increment `efe_jwks_stale_total`. Pre-audit code incremented
    /// the counter unconditionally before the kid lookup, producing
    /// false-positive "stale serves" telemetry.
    #[tokio::test]
    async fn get_key_does_not_inflate_stale_counter_on_kid_miss() {
        // Construct a cache with a 0 ttl so it's "stale" the moment
        // it's populated. Any get_key for an existing kid would mark
        // it stale ; for a missing kid it must NOT.
        let doc = JwksDocument {
            keys: vec![sample_rsa_jwk("k1")],
        };
        let cache = JwksCache::with_fetcher(
            "https://stale-test.example/jwks.json".to_string(),
            Duration::from_millis(0),   // ttl=0 → instantly stale
            Duration::from_millis(100), // fetch_timeout
            0,
            Duration::from_millis(10),
            Duration::from_secs(60), // grace window keeps it serving
            Box::new(MockFetcher::new(vec![Ok(doc)])),
        );
        cache.refresh().await.unwrap();
        // Sleep to ensure the cache is past ttl=0.
        tokio::time::sleep(Duration::from_millis(5)).await;

        let url = "https://stale-test.example/jwks.json";
        let metric =
            efe_telemetry::metrics::d15_instruments::jwks_stale_total().with_label_values(&[url]);
        let before = metric.get();

        // Kid miss — must NOT bump the counter.
        assert!(cache.get_key("missing-kid").await.is_none());
        let after_miss = metric.get();
        assert_eq!(
            after_miss, before,
            "stale counter inflated on kid miss (audit regression)",
        );

        // Kid hit — MUST bump the counter (we served stale).
        assert!(cache.get_key("k1").await.is_some());
        let after_hit = metric.get();
        assert_eq!(
            after_hit,
            before + 1,
            "stale counter not bumped on stale serve",
        );
    }
}
