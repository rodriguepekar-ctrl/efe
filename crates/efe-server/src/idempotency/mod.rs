//! Idempotency cache — keyed by `request_id` × tenant, TTL-bounded.

pub mod cache;

pub use cache::IdempotencyCache;
