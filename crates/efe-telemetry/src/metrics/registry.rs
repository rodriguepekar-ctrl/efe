//! Shared `prometheus::Registry` held behind a `OnceLock` so that every
//! crate in the workspace sees the same metric set regardless of link
//! order.

use std::sync::OnceLock;

use prometheus::Registry;

/// Return the process-global metrics registry.
#[must_use]
pub fn global_registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(Registry::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_registry_is_idempotent() {
        let a = global_registry() as *const _;
        let b = global_registry() as *const _;
        assert_eq!(a, b);
    }
}
