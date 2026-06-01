//! Registry mapping a tenant id to its [`TokenBucket`].

use std::sync::Arc;

use dashmap::DashMap;

use crate::config::RateLimitConfig;
use crate::ratelimit::bucket::TokenBucket;

/// Handle returned by [`BucketRegistry::for_tenant`] / `for_anonymous`.
pub type BucketHandle = Arc<TokenBucket>;

/// Tenant-indexed rate-limit registry.
#[derive(Debug)]
pub struct BucketRegistry {
    buckets: DashMap<String, Arc<TokenBucket>>,
    anon: Arc<TokenBucket>,
    default_capacity: u32,
    default_refill_rate: f32,
}

impl BucketRegistry {
    /// Build from runtime config.
    #[must_use]
    pub fn new(cfg: &RateLimitConfig) -> Self {
        Self {
            buckets: DashMap::new(),
            anon: Arc::new(TokenBucket::new(
                cfg.anonymous_capacity,
                cfg.anonymous_refill_rate,
            )),
            default_capacity: cfg.default_capacity,
            default_refill_rate: cfg.default_refill_rate,
        }
    }

    /// Return the bucket for a given tenant, creating it lazily.
    #[must_use]
    pub fn for_tenant(&self, tenant: &str) -> BucketHandle {
        if let Some(b) = self.buckets.get(tenant) {
            return b.clone();
        }
        let new = Arc::new(TokenBucket::new(
            self.default_capacity,
            self.default_refill_rate,
        ));
        self.buckets.insert(tenant.to_string(), new.clone());
        new
    }

    /// Bucket shared by unauthenticated callers.
    #[must_use]
    pub fn anonymous(&self) -> BucketHandle {
        self.anon.clone()
    }

    /// Number of distinct tenant buckets (debug / metric).
    #[must_use]
    pub fn len(&self) -> usize {
        self.buckets.len()
    }

    /// Whether the registry is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buckets.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> RateLimitConfig {
        RateLimitConfig {
            enabled: true,
            default_capacity: 10,
            default_refill_rate: 5.0,
            anonymous_capacity: 2,
            anonymous_refill_rate: 1.0,
        }
    }

    #[test]
    fn for_tenant_is_stable_across_calls() {
        let r = BucketRegistry::new(&cfg());
        let a1 = r.for_tenant("acme");
        let a2 = r.for_tenant("acme");
        assert!(Arc::ptr_eq(&a1, &a2));
    }

    #[test]
    fn distinct_tenants_have_distinct_buckets() {
        let r = BucketRegistry::new(&cfg());
        let a = r.for_tenant("acme");
        let b = r.for_tenant("other");
        assert!(!Arc::ptr_eq(&a, &b));
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn anonymous_is_shared() {
        let r = BucketRegistry::new(&cfg());
        let a1 = r.anonymous();
        let a2 = r.anonymous();
        assert!(Arc::ptr_eq(&a1, &a2));
    }

    #[test]
    fn anonymous_uses_anonymous_config() {
        let r = BucketRegistry::new(&cfg());
        let a = r.anonymous();
        // capacity=2 : 2 acquires ok, 3rd fail.
        a.try_acquire(1).unwrap();
        a.try_acquire(1).unwrap();
        assert!(a.try_acquire(1).is_err());
    }

    #[test]
    fn tenant_uses_default_config() {
        let r = BucketRegistry::new(&cfg());
        let b = r.for_tenant("acme");
        for _ in 0..10 {
            b.try_acquire(1).unwrap();
        }
        assert!(b.try_acquire(1).is_err());
    }
}
