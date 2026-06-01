//! Simple in-handler rate-limit helper (not a tonic interceptor).
//!
//! We apply the rate limit **inside** the service handler, right after
//! we know the authenticated tenant id. This keeps all metadata in
//! scope without having to thread it through the tower layer stack.

use std::sync::Arc;

use tonic::Status;

use crate::ratelimit::BucketRegistry;

/// Acquire a single token for a tenant. Returns `Ok(())` on success, or
/// a `Status::resource_exhausted` with a `retry-after-ms` metadata entry.
pub fn check_rate_limit(
    registry: Option<&Arc<BucketRegistry>>,
    tenant: &str,
) -> Result<(), Status> {
    let Some(registry) = registry else {
        return Ok(());
    };
    let bucket = if tenant == "anonymous" {
        registry.anonymous()
    } else {
        registry.for_tenant(tenant)
    };
    match bucket.try_acquire(1) {
        Ok(()) => Ok(()),
        Err(e) => {
            let mut status = Status::resource_exhausted(format!(
                "rate limit exceeded for tenant {tenant}: retry after {}ms",
                e.retry_after_ms
            ));
            if let Ok(val) = e.retry_after_ms.to_string().parse() {
                status.metadata_mut().insert("retry-after-ms", val);
            }
            Err(status)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RateLimitConfig;

    #[test]
    fn none_registry_passes_through() {
        assert!(check_rate_limit(None, "acme").is_ok());
    }

    #[test]
    fn authenticated_tenant_uses_default_bucket() {
        let cfg = RateLimitConfig {
            enabled: true,
            default_capacity: 1,
            default_refill_rate: 0.0,
            anonymous_capacity: 99,
            anonymous_refill_rate: 99.0,
        };
        let r = Arc::new(BucketRegistry::new(&cfg));
        assert!(check_rate_limit(Some(&r), "acme").is_ok());
        let err = check_rate_limit(Some(&r), "acme").unwrap_err();
        assert_eq!(err.code(), tonic::Code::ResourceExhausted);
        assert!(err.metadata().get("retry-after-ms").is_some());
    }

    #[test]
    fn anonymous_tenant_uses_anonymous_bucket() {
        let cfg = RateLimitConfig {
            enabled: true,
            default_capacity: 99,
            default_refill_rate: 99.0,
            anonymous_capacity: 1,
            anonymous_refill_rate: 0.0,
        };
        let r = Arc::new(BucketRegistry::new(&cfg));
        assert!(check_rate_limit(Some(&r), "anonymous").is_ok());
        assert!(check_rate_limit(Some(&r), "anonymous").is_err());
    }
}
