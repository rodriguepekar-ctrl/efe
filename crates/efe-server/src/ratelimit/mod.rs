//! Token-bucket rate limiter, per-tenant.

pub mod bucket;
pub mod registry;

pub use bucket::{TokenBucket, TryAcquireError};
pub use registry::BucketRegistry;
