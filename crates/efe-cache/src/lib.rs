//! `efe-cache` — M27 Redis-backed cache plane.
//!
//! Three independent caches share one Redis cluster client :
//!
//! 1. **Embedding cache** ([`embedding::EmbeddingCache`]) — SHA256-keyed
//!    binary blobs (Vec<f32>) with 7-day TTL. Keyed by canonical text +
//!    embedder version so model swaps invalidate cleanly.
//! 2. **LLM response cache** ([`llm::LlmCache`]) — JSON blobs of provider
//!    responses with 1-hour default TTL. HMAC-signed to prevent
//!    poisoning (same pattern as `efe-llm-router::cache`).
//! 3. **Quota counters** ([`quota::QuotaCounters`]) — per-tenant per-
//!    endpoint sliding-window counters in epoch-minute and epoch-day
//!    buckets.
//!
//! All three sit behind a single [`client::CacheClient`] which transparently
//! routes to either a single-instance Redis (`redis://host:6379`) or a
//! Redis Cluster (`redis-cluster://h1:7000,h2:7000,h3:7000`) depending
//! on the URL scheme.
//!
//! ## Threading model
//! `CacheClient` is `Clone + Send + Sync`. Internal Redis connections
//! use `redis::aio::ConnectionManager` (single) or
//! `redis::cluster_async::ClusterConnection` (cluster) — both
//! multiplex many concurrent requests on one TCP connection.
//!
//! ## Failure modes
//! - **Redis down** : every method returns `CacheError::Unavailable`
//!   *without* blocking the caller path. Callers degrade to "no cache".
//! - **HMAC mismatch** : returns `CacheError::Poisoned`, never serves a
//!   tampered value.
//! - **Deserialization failure** : returns `CacheError::Decode` ; never
//!   serves a partial value.
//!
//! ## Security
//! - Tenant data : every tenant-specific key is prefixed `tenant:{slug}:`.
//! - Cross-tenant cache (embeddings, LLM responses) uses SHA256 of
//!   inputs so colliding keys imply colliding inputs — collision is
//!   acceptable as a result-level cache hit.
//! - HMAC signatures on LLM responses use a per-cluster secret loaded
//!   from `EFE_CACHE_HMAC_KEY` env var (32+ random bytes).

#![warn(missing_docs)]

pub mod client;
pub mod embedding;
pub mod error;
pub mod key;
pub mod llm;
pub mod metrics;
pub mod quota;

pub use client::CacheClient;
pub use embedding::EmbeddingCache;
pub use error::CacheError;
pub use llm::{LlmCache, LlmCachedResponse};
pub use quota::QuotaCounters;
