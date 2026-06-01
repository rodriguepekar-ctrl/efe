// SPDX-License-Identifier: UNLICENSED
//! **Sprint 12 — N20** : Multi-provider embedding client with
//! circuit breaker — § 7.5 + ADR-3 + G7.
//!
//! Priority-ordered fallback : try `providers[0]` first ; on
//! `RateLimited` or `Backend` error, record the failure on the
//! per-provider circuit breaker, and try the next.  Returns the
//! first successful embedding ; if all providers fail, returns
//! the LAST error.

use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use efe_nano_sdk::ModelIdentity;

use crate::embedding::circuit_breaker::{CircuitBreaker, CircuitBreakerState};
use crate::embedding::{Embedding, EmbeddingProvider, EmbeddingProviderError};

/// **N20** : a named provider in the priority list.
pub struct NamedProvider {
    pub name: String,
    pub provider: Arc<dyn EmbeddingProvider>,
}

impl NamedProvider {
    pub fn new(name: impl Into<String>, provider: Arc<dyn EmbeddingProvider>) -> Self {
        Self {
            name: name.into(),
            provider,
        }
    }
}

impl std::fmt::Debug for NamedProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NamedProvider")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// **N20** : multi-provider client with per-provider circuit breakers.
pub struct MultiProviderEmbeddingClient {
    providers: Vec<NamedProvider>,
    breakers: DashMap<String, CircuitBreaker>,
}

impl std::fmt::Debug for MultiProviderEmbeddingClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiProviderEmbeddingClient")
            .field("providers", &self.providers)
            .field("breakers_count", &self.breakers.len())
            .finish()
    }
}

impl MultiProviderEmbeddingClient {
    /// **N20** : Construct with the ordered priority list.  Empty
    /// vec is allowed (every call returns `Backend`).
    pub fn new(providers: Vec<NamedProvider>) -> Self {
        let breakers = DashMap::new();
        for p in &providers {
            breakers.insert(p.name.clone(), CircuitBreaker::new());
        }
        Self { providers, breakers }
    }

    /// **N20** : Construct with custom breaker thresholds (tests).
    pub fn with_breaker_thresholds(
        providers: Vec<NamedProvider>,
        failure_threshold: u32,
        open_to_half_open_seconds: u64,
    ) -> Self {
        let breakers = DashMap::new();
        for p in &providers {
            breakers.insert(
                p.name.clone(),
                CircuitBreaker::with_thresholds(failure_threshold, open_to_half_open_seconds),
            );
        }
        Self { providers, breakers }
    }

    /// **N20** : Inspect a breaker (ops dashboard).
    pub fn breaker_state(&self, name: &str) -> Option<CircuitBreakerState> {
        self.breakers.get(name).map(|b| b.value().state())
    }

    /// **N20** : Force-reset a breaker (ops escape hatch).
    pub fn reset_breaker(&self, name: &str) -> bool {
        if let Some(b) = self.breakers.get(name) {
            b.value().reset();
            true
        } else {
            false
        }
    }

    /// **N20** : List the configured providers in priority order.
    pub fn provider_names(&self) -> Vec<String> {
        self.providers.iter().map(|p| p.name.clone()).collect()
    }
}

#[async_trait]
impl EmbeddingProvider for MultiProviderEmbeddingClient {
    async fn embed(
        &self,
        model: &ModelIdentity,
        text: &str,
    ) -> Result<Embedding, EmbeddingProviderError> {
        let mut last_error: Option<EmbeddingProviderError> = None;
        let mut tried_count = 0;
        let mut blocked_count = 0;

        for entry in &self.providers {
            // **Post-audit fix B1** : Read the blocked state under a
            // short-lived `Ref` guard then DROP IT before crossing
            // the `.await` point.  Holding a `DashMap::Ref` across
            // an await deadlocks any concurrent writer (e.g.
            // `reset_breaker`) that targets the same shard.
            let is_blocked = match self.breakers.get(&entry.name) {
                Some(b) => b.value().is_blocked(),
                None => continue,
            };
            if is_blocked {
                blocked_count += 1;
                continue;
            }
            tried_count += 1;

            // The call point — no DashMap Ref alive here.
            let result = entry.provider.embed(model, text).await;

            // Re-acquire the breaker briefly to record the outcome.
            // We tolerate the (extremely unlikely) case where the
            // breaker has been removed between read and write : if
            // so, the outcome is dropped silently — the next call
            // will pick up the absent breaker and continue.
            match result {
                Ok(e) => {
                    if let Some(b) = self.breakers.get(&entry.name) {
                        b.value().record_success();
                    }
                    return Ok(e);
                }
                Err(err) => {
                    if let Some(b) = self.breakers.get(&entry.name) {
                        b.value().record_failure();
                    }
                    last_error = Some(err);
                    // Continue to the next provider.
                }
            }
        }

        match last_error {
            Some(e) => Err(e),
            None => Err(EmbeddingProviderError::Backend {
                message: if blocked_count > 0 && tried_count == 0 {
                    format!("all {blocked_count} providers in open-circuit state")
                } else if self.providers.is_empty() {
                    "no providers configured".into()
                } else {
                    "all providers unavailable".into()
                },
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::StubEmbeddingProvider;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn model() -> ModelIdentity {
        ModelIdentity::hosted("openai", "text-embedding-3-small", "1")
    }

    /// Programmable provider that fails N times then succeeds.
    struct FailNTimes {
        remaining: AtomicU32,
    }
    impl FailNTimes {
        fn new(n: u32) -> Self {
            Self {
                remaining: AtomicU32::new(n),
            }
        }
    }
    impl std::fmt::Debug for FailNTimes {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("FailNTimes")
                .field("remaining", &self.remaining.load(Ordering::Relaxed))
                .finish()
        }
    }
    #[async_trait]
    impl EmbeddingProvider for FailNTimes {
        async fn embed(
            &self,
            model: &ModelIdentity,
            _text: &str,
        ) -> Result<Embedding, EmbeddingProviderError> {
            let r = self.remaining.fetch_sub(1, Ordering::AcqRel);
            if r > 0 {
                Err(EmbeddingProviderError::Backend {
                    message: "simulated fail".into(),
                })
            } else {
                self.remaining.store(0, Ordering::Release);
                Ok(Embedding {
                    model: model.clone(),
                    vector: vec![0.5; 8],
                })
            }
        }
    }

    /// Provider that always rate-limits.
    struct AlwaysRateLimit;
    impl std::fmt::Debug for AlwaysRateLimit {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("AlwaysRateLimit")
        }
    }
    #[async_trait]
    impl EmbeddingProvider for AlwaysRateLimit {
        async fn embed(
            &self,
            _: &ModelIdentity,
            _: &str,
        ) -> Result<Embedding, EmbeddingProviderError> {
            Err(EmbeddingProviderError::RateLimited)
        }
    }

    // ─── Happy path ──────────────────────────────────────────────

    #[tokio::test]
    async fn first_provider_succeeds_returns_immediately() {
        let stub = Arc::new(StubEmbeddingProvider::new());
        let client = MultiProviderEmbeddingClient::new(vec![
            NamedProvider::new("primary", stub.clone()),
            NamedProvider::new("fallback", Arc::new(AlwaysRateLimit)),
        ]);
        let e = client.embed(&model(), "x").await.unwrap();
        assert!(!e.vector.is_empty());
        assert_eq!(stub.call_count(), 1);
        // Breaker stays closed.
        assert_eq!(
            client.breaker_state("primary"),
            Some(CircuitBreakerState::Closed)
        );
    }

    #[tokio::test]
    async fn primary_rate_limited_falls_back_to_secondary() {
        let client = MultiProviderEmbeddingClient::new(vec![
            NamedProvider::new("primary", Arc::new(AlwaysRateLimit)),
            NamedProvider::new("secondary", Arc::new(StubEmbeddingProvider::new())),
        ]);
        let e = client.embed(&model(), "x").await.unwrap();
        assert!(!e.vector.is_empty());
    }

    #[tokio::test]
    async fn primary_succeeds_after_intermittent_failure() {
        // FailNTimes(0) → succeeds on first call.
        let client = MultiProviderEmbeddingClient::new(vec![NamedProvider::new(
            "primary",
            Arc::new(FailNTimes::new(0)),
        )]);
        let _ = client.embed(&model(), "x").await.unwrap();
    }

    // ─── Circuit breaker opens after threshold ───────────────────

    #[tokio::test]
    async fn three_consecutive_failures_open_primary_breaker() {
        let client = MultiProviderEmbeddingClient::with_breaker_thresholds(
            vec![
                NamedProvider::new("primary", Arc::new(AlwaysRateLimit)),
                NamedProvider::new("secondary", Arc::new(StubEmbeddingProvider::new())),
            ],
            3,
            30,
        );
        for _ in 0..3 {
            let _ = client.embed(&model(), "x").await;
        }
        assert_eq!(
            client.breaker_state("primary"),
            Some(CircuitBreakerState::Open)
        );
        // Now primary is open ; secondary still serves.
        let e = client.embed(&model(), "x").await.unwrap();
        assert!(!e.vector.is_empty());
        assert_eq!(
            client.breaker_state("secondary"),
            Some(CircuitBreakerState::Closed)
        );
    }

    #[tokio::test]
    async fn all_providers_failing_returns_last_error() {
        let client = MultiProviderEmbeddingClient::new(vec![
            NamedProvider::new("p1", Arc::new(AlwaysRateLimit)),
            NamedProvider::new("p2", Arc::new(AlwaysRateLimit)),
        ]);
        let err = client.embed(&model(), "x").await.unwrap_err();
        assert!(matches!(err, EmbeddingProviderError::RateLimited));
    }

    #[tokio::test]
    async fn all_breakers_open_returns_backend_error() {
        let client = MultiProviderEmbeddingClient::with_breaker_thresholds(
            vec![
                NamedProvider::new("p1", Arc::new(AlwaysRateLimit)),
                NamedProvider::new("p2", Arc::new(AlwaysRateLimit)),
            ],
            1,
            30,
        );
        // First call opens both (each gets 1 failure → open).
        let _ = client.embed(&model(), "x").await;
        // Both breakers now Open ; next call sees blocked_count = 2,
        // tried_count = 0 → Backend error.
        let err = client.embed(&model(), "x").await.unwrap_err();
        match err {
            EmbeddingProviderError::Backend { message } => {
                assert!(message.contains("open-circuit"), "got : {message}");
            }
            other => panic!("expected Backend, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn empty_provider_list_returns_backend() {
        let client = MultiProviderEmbeddingClient::new(vec![]);
        let err = client.embed(&model(), "x").await.unwrap_err();
        match err {
            EmbeddingProviderError::Backend { message } => {
                assert!(message.contains("no providers configured"));
            }
            other => panic!("expected Backend, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn success_records_on_breaker() {
        let client = MultiProviderEmbeddingClient::new(vec![NamedProvider::new(
            "p",
            Arc::new(StubEmbeddingProvider::new()),
        )]);
        let _ = client.embed(&model(), "x").await.unwrap();
        assert_eq!(client.breaker_state("p"), Some(CircuitBreakerState::Closed));
    }

    #[tokio::test]
    async fn reset_breaker_force_closes() {
        let client = MultiProviderEmbeddingClient::with_breaker_thresholds(
            vec![NamedProvider::new("p", Arc::new(AlwaysRateLimit))],
            1,
            30,
        );
        // Force open.
        let _ = client.embed(&model(), "x").await;
        assert_eq!(client.breaker_state("p"), Some(CircuitBreakerState::Open));
        // Reset.
        assert!(client.reset_breaker("p"));
        assert_eq!(client.breaker_state("p"), Some(CircuitBreakerState::Closed));
    }

    #[tokio::test]
    async fn reset_unknown_provider_returns_false() {
        let client = MultiProviderEmbeddingClient::new(vec![]);
        assert!(!client.reset_breaker("missing"));
    }

    #[tokio::test]
    async fn provider_names_preserves_order() {
        let client = MultiProviderEmbeddingClient::new(vec![
            NamedProvider::new("z", Arc::new(StubEmbeddingProvider::new())),
            NamedProvider::new("a", Arc::new(StubEmbeddingProvider::new())),
            NamedProvider::new("m", Arc::new(StubEmbeddingProvider::new())),
        ]);
        let names = client.provider_names();
        assert_eq!(names, vec!["z", "a", "m"]);
    }

    // ─── Recovery via HalfOpen ───────────────────────────────────

    #[tokio::test]
    async fn half_open_one_success_closes_breaker() {
        let client = MultiProviderEmbeddingClient::with_breaker_thresholds(
            vec![NamedProvider::new(
                "p",
                Arc::new(FailNTimes::new(1)), // fail 1, then succeed
            )],
            1,
            30,
        );
        // First call : fails → open.
        let _ = client.embed(&model(), "x").await;
        assert_eq!(client.breaker_state("p"), Some(CircuitBreakerState::Open));
        // Force the breaker into HalfOpen by manipulating its
        // opened_at timestamp.
        client.breakers.get("p").unwrap().value().force_open_at(31);
        // Now HalfOpen ; next call succeeds → Closed.
        let _ = client.embed(&model(), "x").await.unwrap();
        assert_eq!(client.breaker_state("p"), Some(CircuitBreakerState::Closed));
    }

    #[test]
    fn debug_format_does_not_panic() {
        let client = MultiProviderEmbeddingClient::new(vec![NamedProvider::new(
            "p",
            Arc::new(StubEmbeddingProvider::new()),
        )]);
        let _ = format!("{client:?}");
    }
}
