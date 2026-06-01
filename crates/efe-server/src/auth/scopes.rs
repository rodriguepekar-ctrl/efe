//! Per-RPC scope requirements.
//!
//! Default-deny: any unknown path yields a `"__denied__"` scope which
//! no legitimate JWT can possess.

/// Well-known scope strings. Kept as const so that typos surface at
/// compile time.
pub struct Scope;

impl Scope {
    /// Grant for running inference.
    pub const SWARM_EXECUTE: &'static str = "swarm:execute";
    /// Grant for validating specs without running.
    pub const SWARM_DESCRIBE: &'static str = "swarm:describe";
    /// Grant for reading cones.
    pub const ADMIN_CONE_READ: &'static str = "admin:cone:read";
    /// Grant for mutating cones (register / evolve).
    pub const ADMIN_CONE_WRITE: &'static str = "admin:cone:write";
    /// Grant for deleting cones.
    pub const ADMIN_CONE_DELETE: &'static str = "admin:cone:delete";
    /// Grant for reading breach history.
    pub const ADMIN_BREACH_READ: &'static str = "admin:breach:read";

    // ──────────────────────────────────────────────────────────────
    // Sprint **D12-bis** — catalog admin scopes.
    //
    // Independent from `admin:cone:*` (the M9-era cone admin
    // surface). A JWT can carry `admin:catalog:read` without
    // `admin:cone:*` and vice-versa.
    // ──────────────────────────────────────────────────────────────

    /// Grant for reading catalogs (List / Get / ListVersions).
    pub const ADMIN_CATALOG_READ: &'static str = "admin:catalog:read";
    /// Grant for mutating catalogs (Register / Evolve / Delete).
    pub const ADMIN_CATALOG_WRITE: &'static str = "admin:catalog:write";

    // ──────────────────────────────────────────────────────────────
    // Sprint **D21** (M4 § 9) — admin-side catalog management +
    // bundle reload + audit.
    //
    // These are the granular scopes a principal must hold IN ADDITION
    // to the broad `admin` / `audit` scope (D22). The 2-axis check is
    // implemented in `services/admin.rs::AdminServiceImpl::*` — first
    // `ScopeGuard::require(&claims, BroadScope::ADMIN)`, then
    // `auth.check(&mut req, Some(Scope::<granular>))`.
    // ──────────────────────────────────────────────────────────────

    /// Grant for ReloadBundle (D21/D27). Distinct from
    /// `admin:catalog:write` because bundle reload is a much higher-
    /// privilege operation (it can take the embedder offline).
    pub const ADMIN_BUNDLE_RELOAD: &'static str = "admin:bundle:reload";

    /// Grant for PinCatalog / ReloadCatalog (D21).
    pub const ADMIN_CATALOG_PIN: &'static str = "admin:catalog:pin";

    /// Grant for the audit log RPC `ListAuditEvents` (D21). Read-only.
    pub const ADMIN_AUDIT_READ: &'static str = "admin:audit:read";
}

/// Return the scope required to call `path`, or `None` if the path is
/// public (health check, reflection). Returns `Some("__denied__")` for
/// any unknown path (default-deny).
#[must_use]
pub fn required_scope(path: &str) -> Option<&'static str> {
    match path {
        "/efe.v1.SwarmService/Execute" => Some(Scope::SWARM_EXECUTE),
        "/efe.v1.SwarmService/ExecuteStreaming" => Some(Scope::SWARM_EXECUTE),
        "/efe.v1.SwarmService/DescribeSpec" => Some(Scope::SWARM_DESCRIBE),

        "/efe.v1.admin.AdminService/RegisterCone" => Some(Scope::ADMIN_CONE_WRITE),
        "/efe.v1.admin.AdminService/EvolveCone" => Some(Scope::ADMIN_CONE_WRITE),
        "/efe.v1.admin.AdminService/GetCone" => Some(Scope::ADMIN_CONE_READ),
        "/efe.v1.admin.AdminService/ListVersions" => Some(Scope::ADMIN_CONE_READ),
        "/efe.v1.admin.AdminService/DeleteCone" => Some(Scope::ADMIN_CONE_DELETE),
        "/efe.v1.admin.AdminService/BreachHistory" => Some(Scope::ADMIN_BREACH_READ),

        // Sprint **D12-bis** — catalog service.
        "/efe.v1.catalog.CatalogService/RegisterCatalog" => Some(Scope::ADMIN_CATALOG_WRITE),
        "/efe.v1.catalog.CatalogService/EvolveCatalog" => Some(Scope::ADMIN_CATALOG_WRITE),
        "/efe.v1.catalog.CatalogService/DeleteCatalog" => Some(Scope::ADMIN_CATALOG_WRITE),
        "/efe.v1.catalog.CatalogService/GetCatalog" => Some(Scope::ADMIN_CATALOG_READ),
        "/efe.v1.catalog.CatalogService/ListCatalogs" => Some(Scope::ADMIN_CATALOG_READ),
        "/efe.v1.catalog.CatalogService/ListCatalogVersions" => Some(Scope::ADMIN_CATALOG_READ),

        // ──────────────────────────────────────────────────────────
        // Sprint **D21** (M4 § 9) — 8 admin RPCs catalog-scoped.
        // ──────────────────────────────────────────────────────────
        "/efe.v1.admin.AdminService/ListCatalogs" => Some(Scope::ADMIN_CATALOG_READ),
        "/efe.v1.admin.AdminService/GetCatalog" => Some(Scope::ADMIN_CATALOG_READ),
        "/efe.v1.admin.AdminService/DeleteCatalog" => Some(Scope::ADMIN_CATALOG_WRITE),
        "/efe.v1.admin.AdminService/ReloadBundle" => Some(Scope::ADMIN_BUNDLE_RELOAD),
        "/efe.v1.admin.AdminService/PinCatalog" => Some(Scope::ADMIN_CATALOG_PIN),
        "/efe.v1.admin.AdminService/EvolveCatalog" => Some(Scope::ADMIN_CATALOG_WRITE),
        "/efe.v1.admin.AdminService/ReloadCatalog" => Some(Scope::ADMIN_CATALOG_PIN),
        "/efe.v1.admin.AdminService/ListAuditEvents" => Some(Scope::ADMIN_AUDIT_READ),

        // ──────────────────────────────────────────────────────────
        // Sprint **D34** (M4 § 13) — Mode-2 service.
        // ──────────────────────────────────────────────────────────
        "/efe.v1.mode2.Mode2Service/Execute" => Some(Scope::SWARM_EXECUTE),
        "/efe.v1.mode2.Mode2Service/ExecuteStreaming" => Some(Scope::SWARM_EXECUTE),
        "/efe.v1.mode2.Mode2Service/RouterHealth" => Some(Scope::SWARM_DESCRIBE),

        // ──────────────────────────────────────────────────────────
        // Sprint **D37** (M4 § 14) — Mode-3 service.
        // ──────────────────────────────────────────────────────────
        "/efe.v1.mode3.Mode3Service/Execute" => Some(Scope::SWARM_EXECUTE),
        "/efe.v1.mode3.Mode3Service/SubscribeVerdict" => Some(Scope::SWARM_EXECUTE),
        "/efe.v1.mode3.Mode3Service/Cancel" => Some(Scope::SWARM_EXECUTE),

        // Public endpoints.
        p if p.starts_with("/grpc.health.v1.Health/") => None,
        p if p.starts_with("/grpc.reflection.v1alpha.ServerReflection/") => None,
        p if p.starts_with("/grpc.reflection.v1.ServerReflection/") => None,

        _ => Some("__denied__"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execute_requires_swarm_execute() {
        assert_eq!(
            required_scope("/efe.v1.SwarmService/Execute"),
            Some(Scope::SWARM_EXECUTE)
        );
    }

    #[test]
    fn admin_register_requires_cone_write() {
        assert_eq!(
            required_scope("/efe.v1.admin.AdminService/RegisterCone"),
            Some(Scope::ADMIN_CONE_WRITE)
        );
    }

    #[test]
    fn unknown_path_is_denied_by_default() {
        assert_eq!(
            required_scope("/some.foreign.Service/Method"),
            Some("__denied__")
        );
    }

    #[test]
    fn health_is_public() {
        assert_eq!(required_scope("/grpc.health.v1.Health/Check"), None);
        assert_eq!(required_scope("/grpc.health.v1.Health/Watch"), None);
    }

    #[test]
    fn reflection_is_public() {
        assert_eq!(
            required_scope("/grpc.reflection.v1alpha.ServerReflection/ServerReflectionInfo"),
            None
        );
        assert_eq!(
            required_scope("/grpc.reflection.v1.ServerReflection/ServerReflectionInfo"),
            None
        );
    }

    #[test]
    fn describe_requires_swarm_describe() {
        assert_eq!(
            required_scope("/efe.v1.SwarmService/DescribeSpec"),
            Some(Scope::SWARM_DESCRIBE)
        );
    }

    #[test]
    fn breach_history_requires_breach_read() {
        assert_eq!(
            required_scope("/efe.v1.admin.AdminService/BreachHistory"),
            Some(Scope::ADMIN_BREACH_READ)
        );
    }

    // Sprint **D12-bis** — catalog scope mapping.

    #[test]
    fn catalog_register_requires_catalog_write() {
        assert_eq!(
            required_scope("/efe.v1.catalog.CatalogService/RegisterCatalog"),
            Some(Scope::ADMIN_CATALOG_WRITE)
        );
    }

    #[test]
    fn catalog_evolve_requires_catalog_write() {
        assert_eq!(
            required_scope("/efe.v1.catalog.CatalogService/EvolveCatalog"),
            Some(Scope::ADMIN_CATALOG_WRITE)
        );
    }

    #[test]
    fn catalog_delete_requires_catalog_write() {
        assert_eq!(
            required_scope("/efe.v1.catalog.CatalogService/DeleteCatalog"),
            Some(Scope::ADMIN_CATALOG_WRITE)
        );
    }

    #[test]
    fn catalog_get_requires_catalog_read() {
        assert_eq!(
            required_scope("/efe.v1.catalog.CatalogService/GetCatalog"),
            Some(Scope::ADMIN_CATALOG_READ)
        );
    }

    #[test]
    fn catalog_list_requires_catalog_read() {
        assert_eq!(
            required_scope("/efe.v1.catalog.CatalogService/ListCatalogs"),
            Some(Scope::ADMIN_CATALOG_READ)
        );
    }

    #[test]
    fn catalog_list_versions_requires_catalog_read() {
        assert_eq!(
            required_scope("/efe.v1.catalog.CatalogService/ListCatalogVersions"),
            Some(Scope::ADMIN_CATALOG_READ)
        );
    }

    #[test]
    fn unknown_catalog_method_is_denied() {
        assert_eq!(
            required_scope("/efe.v1.catalog.CatalogService/Unknown"),
            Some("__denied__")
        );
    }

    #[test]
    fn catalog_scopes_are_distinct_from_cone_scopes() {
        assert_ne!(Scope::ADMIN_CATALOG_READ, Scope::ADMIN_CONE_READ);
        assert_ne!(Scope::ADMIN_CATALOG_WRITE, Scope::ADMIN_CONE_WRITE);
    }

    // ──────────────────────────────────────────────────────────────
    // Sprint **D21** (M4 § 9) — admin catalog RPCs scope mapping.
    // ──────────────────────────────────────────────────────────────

    #[test]
    fn admin_list_catalogs_requires_catalog_read() {
        assert_eq!(
            required_scope("/efe.v1.admin.AdminService/ListCatalogs"),
            Some(Scope::ADMIN_CATALOG_READ)
        );
    }

    #[test]
    fn admin_get_catalog_requires_catalog_read() {
        assert_eq!(
            required_scope("/efe.v1.admin.AdminService/GetCatalog"),
            Some(Scope::ADMIN_CATALOG_READ)
        );
    }

    #[test]
    fn admin_delete_catalog_requires_catalog_write() {
        assert_eq!(
            required_scope("/efe.v1.admin.AdminService/DeleteCatalog"),
            Some(Scope::ADMIN_CATALOG_WRITE)
        );
    }

    #[test]
    fn admin_reload_bundle_requires_bundle_reload() {
        assert_eq!(
            required_scope("/efe.v1.admin.AdminService/ReloadBundle"),
            Some(Scope::ADMIN_BUNDLE_RELOAD)
        );
    }

    #[test]
    fn admin_pin_catalog_requires_catalog_pin() {
        assert_eq!(
            required_scope("/efe.v1.admin.AdminService/PinCatalog"),
            Some(Scope::ADMIN_CATALOG_PIN)
        );
    }

    #[test]
    fn admin_evolve_catalog_requires_catalog_write() {
        assert_eq!(
            required_scope("/efe.v1.admin.AdminService/EvolveCatalog"),
            Some(Scope::ADMIN_CATALOG_WRITE)
        );
    }

    #[test]
    fn admin_reload_catalog_requires_catalog_pin() {
        assert_eq!(
            required_scope("/efe.v1.admin.AdminService/ReloadCatalog"),
            Some(Scope::ADMIN_CATALOG_PIN)
        );
    }

    #[test]
    fn admin_list_audit_events_requires_audit_read() {
        assert_eq!(
            required_scope("/efe.v1.admin.AdminService/ListAuditEvents"),
            Some(Scope::ADMIN_AUDIT_READ)
        );
    }

    // ──────────────────────────────────────────────────────────────
    // Sprint **D34** (M4 § 13) — Mode-2.
    // ──────────────────────────────────────────────────────────────

    #[test]
    fn mode2_execute_requires_swarm_execute() {
        assert_eq!(
            required_scope("/efe.v1.mode2.Mode2Service/Execute"),
            Some(Scope::SWARM_EXECUTE)
        );
    }

    #[test]
    fn mode2_streaming_requires_swarm_execute() {
        assert_eq!(
            required_scope("/efe.v1.mode2.Mode2Service/ExecuteStreaming"),
            Some(Scope::SWARM_EXECUTE)
        );
    }

    #[test]
    fn mode2_router_health_requires_swarm_describe() {
        assert_eq!(
            required_scope("/efe.v1.mode2.Mode2Service/RouterHealth"),
            Some(Scope::SWARM_DESCRIBE)
        );
    }

    // ──────────────────────────────────────────────────────────────
    // Sprint **D37** (M4 § 14) — Mode-3.
    // ──────────────────────────────────────────────────────────────

    #[test]
    fn mode3_execute_requires_swarm_execute() {
        assert_eq!(
            required_scope("/efe.v1.mode3.Mode3Service/Execute"),
            Some(Scope::SWARM_EXECUTE)
        );
    }

    #[test]
    fn mode3_subscribe_requires_swarm_execute() {
        assert_eq!(
            required_scope("/efe.v1.mode3.Mode3Service/SubscribeVerdict"),
            Some(Scope::SWARM_EXECUTE)
        );
    }

    #[test]
    fn mode3_cancel_requires_swarm_execute() {
        assert_eq!(
            required_scope("/efe.v1.mode3.Mode3Service/Cancel"),
            Some(Scope::SWARM_EXECUTE)
        );
    }

    #[test]
    fn d21_d22_new_scopes_are_distinct() {
        let scopes = [
            Scope::SWARM_EXECUTE,
            Scope::SWARM_DESCRIBE,
            Scope::ADMIN_CONE_READ,
            Scope::ADMIN_CONE_WRITE,
            Scope::ADMIN_CONE_DELETE,
            Scope::ADMIN_BREACH_READ,
            Scope::ADMIN_CATALOG_READ,
            Scope::ADMIN_CATALOG_WRITE,
            Scope::ADMIN_BUNDLE_RELOAD,
            Scope::ADMIN_CATALOG_PIN,
            Scope::ADMIN_AUDIT_READ,
        ];
        let unique: std::collections::HashSet<_> = scopes.iter().collect();
        assert_eq!(unique.len(), scopes.len(), "scope strings must be unique");
    }
}
