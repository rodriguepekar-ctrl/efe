//! Error types for the cache crate.

use thiserror::Error;

/// All `efe-cache` operations return this error.
#[derive(Debug, Error)]
pub enum CacheError {
    /// Redis returned an error (network, command, cluster slot missing, etc.).
    #[error("redis error : {0}")]
    Redis(#[from] redis::RedisError),

    /// Redis cluster or single-instance is unreachable.
    /// Callers must degrade gracefully (no cache).
    #[error("cache backend unavailable : {0}")]
    Unavailable(String),

    /// Serialization failed when writing to cache.
    #[error("encode failed : {0}")]
    Encode(String),

    /// Deserialization failed when reading from cache.
    /// Indicates either a schema drift between writer and reader (bug)
    /// or partial-write corruption (rare).
    #[error("decode failed : {0}")]
    Decode(String),

    /// Cached value did not pass HMAC verification.
    /// Critical : the cache has been tampered with. Caller should
    /// emit an alert + invalidate the key.
    #[error("cache value HMAC verification failed (poisoning suspected)")]
    Poisoned,

    /// HMAC key wasn't configured at boot.
    #[error("HMAC key missing : {0}")]
    HmacKeyMissing(String),

    /// Generic config issue (URL malformed, etc.).
    #[error("cache misconfiguration : {0}")]
    Misconfig(String),
}

impl From<bincode::Error> for CacheError {
    fn from(e: bincode::Error) -> Self {
        Self::Decode(e.to_string())
    }
}

impl From<serde_json::Error> for CacheError {
    fn from(e: serde_json::Error) -> Self {
        Self::Decode(e.to_string())
    }
}

/// Convenience alias.
pub type CacheResult<T> = Result<T, CacheError>;
