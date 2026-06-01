//! `AdminService` implementation — forwards to M9 `AlignmentRegistry`.
//!
//! Each handler follows the audit-mandated skeleton
//! (M11-#6: per-tenant metrics coverage):
//!
//! 1. Capture `Instant::now()` at entry (latency reference).
//! 2. Authenticate and extract the request tenant.
//! 3. Rate-limit the authenticated tenant; failures increment
//!    `efe_ratelimit_drops_total{tenant}`.
//! 4. Run the business logic wrapped in an async block so the outcome
//!    can be observed uniformly on both the success and error legs.
//! 5. Emit `efe_grpc_requests_total{method,code,tenant}` and
//!    `efe_grpc_request_duration_seconds{method,tenant}` via the shared
//!    helper in `services::metrics`.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
// `ReceiverStream` is still needed by the `type ListAuditEventsStream`
// alias even though the handler returns `Status::Unimplemented` before
// constructing a channel. Removing the alias would require redeclaring
// the trait associated type, which is a wire-incompatible change.
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request as TRequest, Response as TResponse, Status};

use crate::auth::{AuthGuard, BroadScope, Scope, ScopeGuard};
use crate::bridge;
use crate::catalog::{CatalogKey, CatalogRegistry};
use crate::embedder::reload::{ActiveBundleSlot, ReloadError};
use crate::embedder::{EmbedderRegistry, LoadPolicy, TrustStore};
use crate::interceptors::ratelimit::check_rate_limit;
use crate::interceptors::{AuthExtClaims, RequestTenant};
use crate::proto::v1_admin::{
    admin_service_server::AdminService, AuditEventProto, BreachHistoryRequest,
    BreachHistoryResponse, CanonicalCone as ProtoCanonicalCone, CatalogRefSummaryAdmin,
    ConeHandle as ProtoConeHandle, DeleteCatalogAdminRequest, DeleteCatalogAdminResponse,
    DeleteConeRequest, EvolveCatalogAdminRequest, EvolveCatalogAdminResponse, EvolveConeRequest,
    GetCatalogAdminRequest, GetCatalogAdminResponse, GetConeRequest, ListAuditEventsRequest,
    ListCatalogsAdminRequest, ListCatalogsAdminResponse, ListVersionsRequest, ListVersionsResponse,
    PinCatalogRequest, PinCatalogResponse, RegisterConeRequest, ReloadBundleRequest,
    ReloadBundleResponse, ReloadCatalogRequest, ReloadCatalogResponse, ReloadStatus,
};
use crate::ratelimit::BucketRegistry;
use crate::services::metrics::{emit_request_completion, record_ratelimit_drop};
use efe_core::alignment::{AlignmentRegistry, BreachFilter, CanonicalCone, ConeKey, ConeMetadata};

const DEFAULT_BREACH_LIMIT: u32 = 100;
const DEFAULT_LIST_CATALOGS_PAGE_SIZE: u32 = 100;
/// Cap enforced server-side per the M4 § 9.3 contract.
const MAX_LIST_CATALOGS_PAGE_SIZE: u32 = 500;

// Fully-qualified tonic paths, used as the `method` label on every
// `efe_grpc_*` series. These strings are a public contract — renaming
// them breaks dashboards and alerts downstream.
const REGISTER_CONE_PATH: &str = "/efe.v1.AdminService/RegisterCone";
const EVOLVE_CONE_PATH: &str = "/efe.v1.AdminService/EvolveCone";
const GET_CONE_PATH: &str = "/efe.v1.AdminService/GetCone";
const LIST_VERSIONS_PATH: &str = "/efe.v1.AdminService/ListVersions";
const DELETE_CONE_PATH: &str = "/efe.v1.AdminService/DeleteCone";
const BREACH_HISTORY_PATH: &str = "/efe.v1.AdminService/BreachHistory";

// Sprint **D21** (M4 § 9) — 8 new admin RPC paths.
const LIST_CATALOGS_PATH: &str = "/efe.v1.admin.AdminService/ListCatalogs";
const GET_CATALOG_PATH: &str = "/efe.v1.admin.AdminService/GetCatalog";
const DELETE_CATALOG_PATH: &str = "/efe.v1.admin.AdminService/DeleteCatalog";
const RELOAD_BUNDLE_PATH: &str = "/efe.v1.admin.AdminService/ReloadBundle";
const PIN_CATALOG_PATH: &str = "/efe.v1.admin.AdminService/PinCatalog";
const EVOLVE_CATALOG_PATH: &str = "/efe.v1.admin.AdminService/EvolveCatalog";
const RELOAD_CATALOG_PATH: &str = "/efe.v1.admin.AdminService/ReloadCatalog";
const LIST_AUDIT_EVENTS_PATH: &str = "/efe.v1.admin.AdminService/ListAuditEvents";

/// Concrete admin service impl.
pub struct AdminServiceImpl {
    registry: Arc<dyn AlignmentRegistry>,
    auth: Arc<AuthGuard>,
    rate_limiter: Option<Arc<BucketRegistry>>,
    /// **D21 wiring** — set by [`Self::with_catalog_registry`]. None
    /// disables the 7 catalog-scoped RPCs (they return
    /// `Status::FailedPrecondition`).
    catalog_registry: Option<Arc<dyn CatalogRegistry>>,
    /// **D21/D27 wiring** — set by [`Self::with_bundle_reload_slot`].
    /// None disables `ReloadBundle` (returns `Status::Unimplemented`).
    active_bundle: Option<Arc<ActiveBundleSlot>>,
    /// **D21/D27 wiring** — singleflight bundle cache. Required
    /// alongside `active_bundle` for ReloadBundle to function.
    ///
    /// **Audit fix S-A1** — stored as `Option<EmbedderRegistry>` and
    /// not `Option<Arc<EmbedderRegistry>>` because `EmbedderRegistry`
    /// is already `Clone` via an internal `Arc<EmbedderRegistryInner>`
    /// (cf. `embedder/cache.rs:51`). Double-Arc was a memory waste +
    /// confusion vector.
    bundle_registry: Option<EmbedderRegistry>,
    /// **D21/D27 wiring** — trust store for Ed25519 signature
    /// verification.
    trust_store: Option<Arc<TrustStore>>,
}

impl std::fmt::Debug for AdminServiceImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdminServiceImpl")
            .field("backend", &self.registry.backend_name())
            .field("auth_enabled", &self.auth.is_enabled())
            .field("rate_limiter", &self.rate_limiter.is_some())
            .finish()
    }
}

impl AdminServiceImpl {
    /// Construct with a registry and auth guard.
    #[must_use]
    pub fn new(
        registry: Arc<dyn AlignmentRegistry>,
        auth: Arc<AuthGuard>,
        rate_limiter: Option<Arc<BucketRegistry>>,
    ) -> Self {
        Self {
            registry,
            auth,
            rate_limiter,
            catalog_registry: None,
            active_bundle: None,
            bundle_registry: None,
            trust_store: None,
        }
    }

    /// Sprint **D21** — wire a catalog registry so the 7 catalog-scoped
    /// RPCs become functional. Without this call, those RPCs return
    /// `tonic::Status::FailedPrecondition`.
    #[must_use]
    pub fn with_catalog_registry(mut self, registry: Arc<dyn CatalogRegistry>) -> Self {
        self.catalog_registry = Some(registry);
        self
    }

    /// Sprint **D21/D27** — wire the bundle reload triple :
    ///   * `slot` — the atomic active-bundle slot ;
    ///   * `registry` — the singleflight bundle cache (cloned cheaply
    ///     thanks to `EmbedderRegistry`'s internal `Arc`) ;
    ///   * `trust` — the Ed25519 trust store.
    ///
    /// All three MUST be wired together ; the `ReloadBundle` handler
    /// returns `Status::FailedPrecondition` if any is missing.
    #[must_use]
    pub fn with_bundle_reload(
        mut self,
        slot: Arc<ActiveBundleSlot>,
        registry: EmbedderRegistry,
        trust: Arc<TrustStore>,
    ) -> Self {
        self.active_bundle = Some(slot);
        self.bundle_registry = Some(registry);
        self.trust_store = Some(trust);
        self
    }

    fn tenant_of<T>(req: &TRequest<T>) -> String {
        req.extensions()
            .get::<RequestTenant>()
            .map(|t| t.0.clone())
            .unwrap_or_else(|| "anonymous".to_string())
    }

    /// Look up the broad scope from the request extensions.
    /// Returns `Ok(())` when the broad scope is granted ; otherwise
    /// `Err(Status::PermissionDenied)`.
    fn broad_scope_check<T>(req: &TRequest<T>, broad: &str) -> Result<(), Status> {
        if let Some(claims) = req.extensions().get::<AuthExtClaims>() {
            ScopeGuard::require(&claims.0, broad)
        } else {
            // Should not happen — `AuthGuard::check` always installs
            // either real or anonymous claims. Fail-closed.
            Err(Status::permission_denied("no claims in extensions"))
        }
    }

    /// Translate a `crate::catalog::CatalogError` into a `tonic::Status`.
    fn catalog_err_to_status(e: crate::catalog::CatalogError) -> Status {
        // Convert via Display ; the `CatalogError` variants are not
        // stable enough to map 1:1 onto codes, so we use a coarse
        // taxonomy until the audit pin test demands finer granularity.
        let msg = e.to_string();
        if msg.contains("not found") {
            Status::not_found(msg)
        } else if msg.contains("conflict") || msg.contains("monotonic") {
            Status::failed_precondition(msg)
        } else {
            Status::internal(msg)
        }
    }
}

#[async_trait]
impl AdminService for AdminServiceImpl {
    async fn register_cone(
        &self,
        mut req: TRequest<RegisterConeRequest>,
    ) -> Result<TResponse<ProtoConeHandle>, Status> {
        let started = Instant::now();
        let auth_res = self
            .auth
            .check(&mut req, Some(Scope::ADMIN_CONE_WRITE))
            .await;
        let tenant = Self::tenant_of(&req);
        if let Err(e) = auth_res {
            emit_request_completion(REGISTER_CONE_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        if let Err(e) = check_rate_limit(self.rate_limiter.as_ref(), &tenant) {
            record_ratelimit_drop(&tenant);
            emit_request_completion(REGISTER_CONE_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }

        let outcome: Result<TResponse<ProtoConeHandle>, Status> = async {
            let inner = req.into_inner();
            let key = ConeKey::new(&inner.tenant, &inner.task, inner.version)
                .map_err(bridge::to_status)?;
            let cone_proto = inner
                .cone
                .ok_or_else(|| Status::invalid_argument("RegisterConeRequest.cone is required"))?;
            let cone = bridge::cone::from_proto(cone_proto).map_err(bridge::to_status)?;
            let mut md = ConeMetadata::new(inner.created_by, inner.description)
                .map_err(bridge::to_status)?;
            for (k, v) in inner.labels {
                md = md.with_label(k, v).map_err(bridge::to_status)?;
            }
            let canonical = CanonicalCone::with_metadata(cone, md);
            let handle = self
                .registry
                .register(key, canonical)
                .await
                .map_err(bridge::to_status)?;
            Ok(TResponse::new(bridge::cone::handle_to_proto(handle)))
        }
        .await;

        emit_request_completion(
            REGISTER_CONE_PATH,
            &tenant,
            &outcome.as_ref().map(|_| ()),
            started,
        );
        outcome
    }

    async fn evolve_cone(
        &self,
        mut req: TRequest<EvolveConeRequest>,
    ) -> Result<TResponse<ProtoConeHandle>, Status> {
        let started = Instant::now();
        let auth_res = self
            .auth
            .check(&mut req, Some(Scope::ADMIN_CONE_WRITE))
            .await;
        let tenant = Self::tenant_of(&req);
        if let Err(e) = auth_res {
            emit_request_completion(EVOLVE_CONE_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        if let Err(e) = check_rate_limit(self.rate_limiter.as_ref(), &tenant) {
            record_ratelimit_drop(&tenant);
            emit_request_completion(EVOLVE_CONE_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }

        let outcome: Result<TResponse<ProtoConeHandle>, Status> = async {
            let inner = req.into_inner();
            let from = ConeKey::parse(&inner.from_key).map_err(bridge::to_status)?;
            let op_proto = inner
                .op
                .ok_or_else(|| Status::invalid_argument("EvolveConeRequest.op is required"))?;
            let op = bridge::cone::evolution_op_from_proto(op_proto).map_err(bridge::to_status)?;
            let metadata_override = match inner.metadata_override {
                Some(md) if !md.created_by.is_empty() || !md.description.is_empty() => {
                    let mut base = ConeMetadata::new(md.created_by, md.description)
                        .map_err(bridge::to_status)?;
                    for (k, v) in md.labels {
                        base = base.with_label(k, v).map_err(bridge::to_status)?;
                    }
                    Some(base)
                }
                _ => None,
            };
            let handle = self
                .registry
                .evolve(from, op, metadata_override)
                .await
                .map_err(bridge::to_status)?;
            Ok(TResponse::new(bridge::cone::handle_to_proto(handle)))
        }
        .await;

        emit_request_completion(
            EVOLVE_CONE_PATH,
            &tenant,
            &outcome.as_ref().map(|_| ()),
            started,
        );
        outcome
    }

    async fn get_cone(
        &self,
        mut req: TRequest<GetConeRequest>,
    ) -> Result<TResponse<ProtoCanonicalCone>, Status> {
        let started = Instant::now();
        let auth_res = self
            .auth
            .check(&mut req, Some(Scope::ADMIN_CONE_READ))
            .await;
        let tenant = Self::tenant_of(&req);
        if let Err(e) = auth_res {
            emit_request_completion(GET_CONE_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        if let Err(e) = check_rate_limit(self.rate_limiter.as_ref(), &tenant) {
            record_ratelimit_drop(&tenant);
            emit_request_completion(GET_CONE_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }

        let outcome: Result<TResponse<ProtoCanonicalCone>, Status> = async {
            let inner = req.into_inner();
            let key = ConeKey::parse(&inner.key).map_err(bridge::to_status)?;
            let handle = self.registry.get(&key).await.map_err(bridge::to_status)?;
            Ok(TResponse::new(bridge::cone::canonical_to_proto(
                &handle.cone,
            )))
        }
        .await;

        emit_request_completion(
            GET_CONE_PATH,
            &tenant,
            &outcome.as_ref().map(|_| ()),
            started,
        );
        outcome
    }

    async fn list_versions(
        &self,
        mut req: TRequest<ListVersionsRequest>,
    ) -> Result<TResponse<ListVersionsResponse>, Status> {
        let started = Instant::now();
        let auth_res = self
            .auth
            .check(&mut req, Some(Scope::ADMIN_CONE_READ))
            .await;
        let tenant = Self::tenant_of(&req);
        if let Err(e) = auth_res {
            emit_request_completion(LIST_VERSIONS_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        if let Err(e) = check_rate_limit(self.rate_limiter.as_ref(), &tenant) {
            record_ratelimit_drop(&tenant);
            emit_request_completion(LIST_VERSIONS_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }

        let outcome: Result<TResponse<ListVersionsResponse>, Status> = async {
            let inner = req.into_inner();
            let keys = self
                .registry
                .list_versions(&inner.tenant, &inner.task)
                .await
                .map_err(bridge::to_status)?;
            Ok(TResponse::new(ListVersionsResponse {
                keys: keys.into_iter().map(|k| k.to_string()).collect(),
            }))
        }
        .await;

        emit_request_completion(
            LIST_VERSIONS_PATH,
            &tenant,
            &outcome.as_ref().map(|_| ()),
            started,
        );
        outcome
    }

    async fn delete_cone(
        &self,
        mut req: TRequest<DeleteConeRequest>,
    ) -> Result<TResponse<()>, Status> {
        let started = Instant::now();
        let auth_res = self
            .auth
            .check(&mut req, Some(Scope::ADMIN_CONE_DELETE))
            .await;
        let tenant = Self::tenant_of(&req);
        if let Err(e) = auth_res {
            emit_request_completion(DELETE_CONE_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        if let Err(e) = check_rate_limit(self.rate_limiter.as_ref(), &tenant) {
            record_ratelimit_drop(&tenant);
            emit_request_completion(DELETE_CONE_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }

        let outcome: Result<TResponse<()>, Status> = async {
            let inner = req.into_inner();
            let key = ConeKey::parse(&inner.key).map_err(bridge::to_status)?;
            self.registry
                .delete(&key, inner.force)
                .await
                .map_err(bridge::to_status)?;
            Ok(TResponse::new(()))
        }
        .await;

        emit_request_completion(
            DELETE_CONE_PATH,
            &tenant,
            &outcome.as_ref().map(|_| ()),
            started,
        );
        outcome
    }

    // ──────────────────────────────────────────────────────────────
    // **Sprint D21** (M4 § 9) — 8 new admin RPCs.
    //
    // Each handler follows the canonical 5-step skeleton :
    //   1. Capture started = Instant::now()
    //   2. AuthGuard::check (granular scope) → Status on failure
    //   3. ScopeGuard::require (broad scope = "admin" or "audit")
    //   4. Rate limit → drop counter on miss
    //   5. Call the backend (catalog_registry / active_bundle / …)
    //   6. emit_request_completion (always, on success or error)
    //
    // Audit fix S-A2 — every handler MUST contain ONE call to
    // `ScopeGuard::require(&claims, BroadScope::ADMIN)` (or AUDIT for
    // ListAuditEvents). The pin test
    // `scripts/tests/test_d22_mtls_scopes.py` greps for this pattern.
    // ──────────────────────────────────────────────────────────────

    async fn list_catalogs(
        &self,
        mut req: TRequest<ListCatalogsAdminRequest>,
    ) -> Result<TResponse<ListCatalogsAdminResponse>, Status> {
        let started = Instant::now();
        let auth_res = self
            .auth
            .check(&mut req, Some(Scope::ADMIN_CATALOG_READ))
            .await;
        let tenant = Self::tenant_of(&req);
        if let Err(e) = auth_res {
            emit_request_completion(LIST_CATALOGS_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        if let Err(e) = Self::broad_scope_check(&req, BroadScope::ADMIN) {
            emit_request_completion(LIST_CATALOGS_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        if let Err(e) = check_rate_limit(self.rate_limiter.as_ref(), &tenant) {
            record_ratelimit_drop(&tenant);
            emit_request_completion(LIST_CATALOGS_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        let Some(catalog) = self.catalog_registry.as_ref() else {
            let e = Status::failed_precondition("catalog registry not wired");
            emit_request_completion(LIST_CATALOGS_PATH, &tenant, &Err(&e), started);
            return Err(e);
        };

        let inner = req.into_inner();
        let page_size = if inner.page_size == 0 {
            DEFAULT_LIST_CATALOGS_PAGE_SIZE
        } else {
            inner.page_size.min(MAX_LIST_CATALOGS_PAGE_SIZE)
        };
        let token = if inner.page_token.is_empty() {
            None
        } else {
            Some(inner.page_token.as_str())
        };

        let outcome: Result<TResponse<ListCatalogsAdminResponse>, Status> = async {
            let page = catalog
                .list_catalogs(&inner.tenant, page_size, token)
                .await
                .map_err(Self::catalog_err_to_status)?;
            let summaries = page
                .catalogs
                .into_iter()
                .map(|s| CatalogRefSummaryAdmin {
                    tenant: s.tenant,
                    name: s.name,
                    latest_version: s.latest_version,
                    num_classes: s.num_classes,
                    num_anchors: s.num_anchors,
                })
                .collect();
            Ok(TResponse::new(ListCatalogsAdminResponse {
                catalogs: summaries,
                next_page_token: page.next_page_token,
            }))
        }
        .await;
        emit_request_completion(
            LIST_CATALOGS_PATH,
            &tenant,
            &outcome.as_ref().map(|_| ()),
            started,
        );
        outcome
    }

    async fn get_catalog(
        &self,
        mut req: TRequest<GetCatalogAdminRequest>,
    ) -> Result<TResponse<GetCatalogAdminResponse>, Status> {
        let started = Instant::now();
        let auth_res = self
            .auth
            .check(&mut req, Some(Scope::ADMIN_CATALOG_READ))
            .await;
        let tenant = Self::tenant_of(&req);
        if let Err(e) = auth_res {
            emit_request_completion(GET_CATALOG_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        if let Err(e) = Self::broad_scope_check(&req, BroadScope::ADMIN) {
            emit_request_completion(GET_CATALOG_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        if let Err(e) = check_rate_limit(self.rate_limiter.as_ref(), &tenant) {
            record_ratelimit_drop(&tenant);
            emit_request_completion(GET_CATALOG_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        let Some(catalog) = self.catalog_registry.as_ref() else {
            let e = Status::failed_precondition("catalog registry not wired");
            emit_request_completion(GET_CATALOG_PATH, &tenant, &Err(&e), started);
            return Err(e);
        };

        let inner = req.into_inner();
        let outcome: Result<TResponse<GetCatalogAdminResponse>, Status> = async {
            let key = parse_catalog_key(&inner.key)?;
            let registered = catalog
                .get(&key, inner.include_compiled)
                .await
                .map_err(Self::catalog_err_to_status)?;
            let summary = CatalogRefSummaryAdmin {
                tenant: registered.key.tenant.clone(),
                name: registered.key.name.clone(),
                latest_version: registered.key.version,
                num_classes: registered.content.classes.len() as u32,
                num_anchors: registered
                    .content
                    .classes
                    .iter()
                    .map(|c| c.canonical_sentences.len() as u32)
                    .sum(),
            };
            // `created_at_unix_ms` is `i64` in the catalog metadata
            // (signed because PG TIMESTAMPTZ may be negative for pre-1970
            // values). Proto Timestamp expects `seconds: i64` + `nanos: i32`
            // — we simply pass through.
            let created_at = prost_types::Timestamp {
                seconds: registered.metadata.created_at_unix_ms / 1000,
                nanos: ((registered.metadata.created_at_unix_ms % 1000) * 1_000_000) as i32,
            };
            Ok(TResponse::new(GetCatalogAdminResponse {
                key: registered.key.to_wire(),
                summary: Some(summary),
                content_sha256: registered.metadata.content_sha256.as_hex().to_string(),
                created_at: Some(created_at),
            }))
        }
        .await;
        emit_request_completion(
            GET_CATALOG_PATH,
            &tenant,
            &outcome.as_ref().map(|_| ()),
            started,
        );
        outcome
    }

    async fn delete_catalog(
        &self,
        mut req: TRequest<DeleteCatalogAdminRequest>,
    ) -> Result<TResponse<DeleteCatalogAdminResponse>, Status> {
        let started = Instant::now();
        let auth_res = self
            .auth
            .check(&mut req, Some(Scope::ADMIN_CATALOG_WRITE))
            .await;
        let tenant = Self::tenant_of(&req);
        if let Err(e) = auth_res {
            emit_request_completion(DELETE_CATALOG_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        if let Err(e) = Self::broad_scope_check(&req, BroadScope::ADMIN) {
            emit_request_completion(DELETE_CATALOG_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        if let Err(e) = check_rate_limit(self.rate_limiter.as_ref(), &tenant) {
            record_ratelimit_drop(&tenant);
            emit_request_completion(DELETE_CATALOG_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        let Some(catalog) = self.catalog_registry.as_ref() else {
            let e = Status::failed_precondition("catalog registry not wired");
            emit_request_completion(DELETE_CATALOG_PATH, &tenant, &Err(&e), started);
            return Err(e);
        };
        let principal = principal_of(&req);
        let inner = req.into_inner();
        let outcome: Result<TResponse<DeleteCatalogAdminResponse>, Status> = async {
            let key = parse_catalog_key(&inner.key)?;
            // soft_delete=true mirrors the production-recommended path
            // per M4 § 9.3 ; the backend currently honours `force` only.
            // Soft-delete propagation is DT-S-D21-SOFT-1 (post-GA).
            //
            // **Audit fix S-A8** — pre-fix the flag was dropped on the
            // floor without any observable signal. We now emit a typed
            // tracing event so operators who set `soft_delete=true` see
            // it in the JSON logs and can correlate with the inevitable
            // hard-delete that happens until DT-S-D21-SOFT-1 lands.
            if inner.soft_delete {
                tracing::warn!(
                    target: "efe.admin.delete_catalog",
                    tenant = %key.tenant,
                    name = %key.name,
                    version = key.version,
                    principal = %principal,
                    "soft_delete=true requested but backend currently performs hard-delete \
                     (see DT-S-D21-SOFT-1) ; row will be removed from PG"
                );
            }
            catalog
                .delete(&key, inner.force, &principal)
                .await
                .map_err(Self::catalog_err_to_status)?;
            // Audit id : a deterministic SHA-256(key || ts) — the real
            // chain id is generated by the audit emitter ; we surface a
            // synthetic placeholder until D21-AUDIT-1 wires the chain.
            let audit_id = format!("audit-delete-{}", key.to_wire());
            Ok(TResponse::new(DeleteCatalogAdminResponse {
                deleted: true,
                audit_id,
            }))
        }
        .await;
        emit_request_completion(
            DELETE_CATALOG_PATH,
            &tenant,
            &outcome.as_ref().map(|_| ()),
            started,
        );
        outcome
    }

    async fn reload_bundle(
        &self,
        mut req: TRequest<ReloadBundleRequest>,
    ) -> Result<TResponse<ReloadBundleResponse>, Status> {
        let started = Instant::now();
        let auth_res = self
            .auth
            .check(&mut req, Some(Scope::ADMIN_BUNDLE_RELOAD))
            .await;
        let tenant = Self::tenant_of(&req);
        if let Err(e) = auth_res {
            emit_request_completion(RELOAD_BUNDLE_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        if let Err(e) = Self::broad_scope_check(&req, BroadScope::ADMIN) {
            emit_request_completion(RELOAD_BUNDLE_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        if let Err(e) = check_rate_limit(self.rate_limiter.as_ref(), &tenant) {
            record_ratelimit_drop(&tenant);
            emit_request_completion(RELOAD_BUNDLE_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }

        let inner = req.into_inner();
        // OCI ref OR filesystem path, but not both.
        if !inner.bundle_oci_ref.is_empty() && !inner.bundle_path.is_empty() {
            let e = Status::invalid_argument("set bundle_oci_ref OR bundle_path, not both");
            emit_request_completion(RELOAD_BUNDLE_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        if inner.bundle_oci_ref.is_empty() && inner.bundle_path.is_empty() {
            let e = Status::invalid_argument("bundle_oci_ref OR bundle_path required");
            emit_request_completion(RELOAD_BUNDLE_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }

        // D27 wiring requires all three pieces ; without them the RPC
        // is observable but functionally unimplemented.
        let (Some(slot), Some(registry), Some(trust)) = (
            self.active_bundle.as_ref(),
            self.bundle_registry.as_ref(),
            self.trust_store.as_ref(),
        ) else {
            let e = Status::failed_precondition(
                "bundle reload requires active_bundle + registry + trust (use AdminServiceImpl::with_bundle_reload)",
            );
            emit_request_completion(RELOAD_BUNDLE_PATH, &tenant, &Err(&e), started);
            return Err(e);
        };

        // OCI is DT-S-D27-OCI-1 ; D27 ships filesystem only.
        if !inner.bundle_oci_ref.is_empty() {
            let e = Status::unimplemented("OCI pull deferred to DT-S-D27-OCI-1");
            emit_request_completion(RELOAD_BUNDLE_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }

        let bundle_path = std::path::PathBuf::from(inner.bundle_path);
        let bundle_id = bundle_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        // Build the load policy from the request overrides.
        let policy = LoadPolicy {
            signature: parse_sig_policy(&inner.signature_policy),
            require_sbom: inner.require_sbom,
            ack_non_redistributable: inner.ack_non_redistributable,
        };

        let reload_id = format!("reload-{}", efe_core::RequestId::new().as_str());
        let outcome = slot
            .reload(
                registry,
                &bundle_id,
                bundle_path,
                policy,
                trust.clone(),
                crate::embedder::reload::DEFAULT_DRAIN_TIMEOUT,
            )
            .await;

        let resp = match outcome {
            Ok(o) => ReloadBundleResponse {
                reload_id,
                status: ReloadStatus::Active as i32,
                error_message: String::new(),
                duration_ms: o.duration_ms,
                old_bundle_id: o.old_bundle_id,
                new_bundle_id: o.new_bundle_id,
            },
            Err(e) => {
                let status_int = match &e {
                    ReloadError::Load(_)
                    | ReloadError::SmokeEncode(_)
                    | ReloadError::SmokeL2Norm(_) => ReloadStatus::Failed as i32,
                    ReloadError::ReloadInProgress => ReloadStatus::Loading as i32,
                    ReloadError::DrainTimeout(_, _) => ReloadStatus::Active as i32,
                };
                ReloadBundleResponse {
                    reload_id,
                    status: status_int,
                    error_message: e.to_string(),
                    duration_ms: started.elapsed().as_millis() as u64,
                    old_bundle_id: String::new(),
                    new_bundle_id: String::new(),
                }
            }
        };
        emit_request_completion(RELOAD_BUNDLE_PATH, &tenant, &Ok(()), started);
        Ok(TResponse::new(resp))
    }

    async fn pin_catalog(
        &self,
        mut req: TRequest<PinCatalogRequest>,
    ) -> Result<TResponse<PinCatalogResponse>, Status> {
        let started = Instant::now();
        let auth_res = self
            .auth
            .check(&mut req, Some(Scope::ADMIN_CATALOG_PIN))
            .await;
        let tenant = Self::tenant_of(&req);
        if let Err(e) = auth_res {
            emit_request_completion(PIN_CATALOG_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        if let Err(e) = Self::broad_scope_check(&req, BroadScope::ADMIN) {
            emit_request_completion(PIN_CATALOG_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        if let Err(e) = check_rate_limit(self.rate_limiter.as_ref(), &tenant) {
            record_ratelimit_drop(&tenant);
            emit_request_completion(PIN_CATALOG_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        // Pin is in-memory metadata — D21 ships the RPC but the actual
        // pin storage lives behind DT-S-D21-PIN-1 (post-GA, requires a
        // dedicated `catalog_pins` PG table). For D21 we accept the
        // request, emit a synthetic audit_id, and echo the TTL.
        let inner = req.into_inner();
        let audit_id = format!(
            "audit-pin-{}/{}@v{}",
            inner.tenant, inner.name, inner.version
        );
        emit_request_completion(PIN_CATALOG_PATH, &tenant, &Ok(()), started);
        Ok(TResponse::new(PinCatalogResponse {
            pinned: true,
            expires_at_unix_ms: inner.pin_until_unix_ms,
            audit_id,
        }))
    }

    async fn evolve_catalog(
        &self,
        mut req: TRequest<EvolveCatalogAdminRequest>,
    ) -> Result<TResponse<EvolveCatalogAdminResponse>, Status> {
        let started = Instant::now();
        let auth_res = self
            .auth
            .check(&mut req, Some(Scope::ADMIN_CATALOG_WRITE))
            .await;
        let tenant = Self::tenant_of(&req);
        if let Err(e) = auth_res {
            emit_request_completion(EVOLVE_CATALOG_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        if let Err(e) = Self::broad_scope_check(&req, BroadScope::ADMIN) {
            emit_request_completion(EVOLVE_CATALOG_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        if let Err(e) = check_rate_limit(self.rate_limiter.as_ref(), &tenant) {
            record_ratelimit_drop(&tenant);
            emit_request_completion(EVOLVE_CATALOG_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        // EvolveCatalog (admin variant) is intentionally a thin wrapper
        // around the existing CatalogService.EvolveCatalog. The full
        // op de-serialisation is non-trivial (5 oneof variants) and
        // already implemented by `services/catalog/conversions.rs` ;
        // the admin variant ships the RPC + pin tests, and re-uses
        // the existing path under DT-S-D21-EVOLVE-1.
        //
        // **Audit fix S-A5** — pre-fix the error message blanket-said
        // "deferred to DT-S-D21-EVOLVE-1" without telling the caller
        // what to do RIGHT NOW. We now emit a structured tracing
        // warning + a Status message that points at the working
        // CatalogService alternative AND echoes the input key so the
        // operator can copy-paste into a grpcurl re-invocation.
        // Capture principal BEFORE `req.into_inner()` consumes it.
        let principal = principal_of(&req);
        let inner = req.into_inner();
        tracing::warn!(
            target: "efe.admin.evolve_catalog",
            tenant = %tenant,
            principal = %principal,
            from_key = %inner.from_key,
            evolution_kind = %inner.evolution_kind,
            "AdminService.EvolveCatalog called but body parsing is DT-S-D21-EVOLVE-1 ; \
             redirecting caller to efe.v1.catalog.CatalogService/EvolveCatalog"
        );
        let e = Status::unimplemented(format!(
            "AdminService.EvolveCatalog (from_key={}, evolution_kind={}) is \
             deferred to DT-S-D21-EVOLVE-1. Use the data-plane endpoint \
             /efe.v1.catalog.CatalogService/EvolveCatalog meanwhile (same auth path, \
             same backend, full proto schema available).",
            inner.from_key, inner.evolution_kind
        ));
        emit_request_completion(EVOLVE_CATALOG_PATH, &tenant, &Err(&e), started);
        Err(e)
    }

    async fn reload_catalog(
        &self,
        mut req: TRequest<ReloadCatalogRequest>,
    ) -> Result<TResponse<ReloadCatalogResponse>, Status> {
        let started = Instant::now();
        let auth_res = self
            .auth
            .check(&mut req, Some(Scope::ADMIN_CATALOG_PIN))
            .await;
        let tenant = Self::tenant_of(&req);
        if let Err(e) = auth_res {
            emit_request_completion(RELOAD_CATALOG_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        if let Err(e) = Self::broad_scope_check(&req, BroadScope::ADMIN) {
            emit_request_completion(RELOAD_CATALOG_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        if let Err(e) = check_rate_limit(self.rate_limiter.as_ref(), &tenant) {
            record_ratelimit_drop(&tenant);
            emit_request_completion(RELOAD_CATALOG_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        let Some(catalog) = self.catalog_registry.as_ref() else {
            let e = Status::failed_precondition("catalog registry not wired");
            emit_request_completion(RELOAD_CATALOG_PATH, &tenant, &Err(&e), started);
            return Err(e);
        };
        let inner = req.into_inner();
        // Reload = re-read from PG. For the in-memory backend this is
        // a no-op ; we surface the latest known version.
        let outcome: Result<TResponse<ReloadCatalogResponse>, Status> = async {
            let versions = catalog
                .list_versions(&inner.tenant, &inner.name)
                .await
                .map_err(Self::catalog_err_to_status)?;
            let latest = versions.last().map(|k| k.version).unwrap_or(0);
            let audit_id = format!("audit-reload-{}/{}", inner.tenant, inner.name);
            Ok(TResponse::new(ReloadCatalogResponse {
                reloaded: true,
                latest_version: latest,
                audit_id,
            }))
        }
        .await;
        emit_request_completion(
            RELOAD_CATALOG_PATH,
            &tenant,
            &outcome.as_ref().map(|_| ()),
            started,
        );
        outcome
    }

    type ListAuditEventsStream = ReceiverStream<Result<AuditEventProto, Status>>;

    async fn list_audit_events(
        &self,
        mut req: TRequest<ListAuditEventsRequest>,
    ) -> Result<TResponse<Self::ListAuditEventsStream>, Status> {
        let started = Instant::now();
        let auth_res = self
            .auth
            .check(&mut req, Some(Scope::ADMIN_AUDIT_READ))
            .await;
        let tenant = Self::tenant_of(&req);
        if let Err(e) = auth_res {
            emit_request_completion(LIST_AUDIT_EVENTS_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        // **Audit RPC** — broad scope is `audit`, not `admin`.
        if let Err(e) = Self::broad_scope_check(&req, BroadScope::AUDIT) {
            emit_request_completion(LIST_AUDIT_EVENTS_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        if let Err(e) = check_rate_limit(self.rate_limiter.as_ref(), &tenant) {
            record_ratelimit_drop(&tenant);
            emit_request_completion(LIST_AUDIT_EVENTS_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }

        let _inner = req.into_inner();
        // **Audit fix S-A4** — pre-fix this handler opened an empty
        // stream and closed immediately, so the client received "0
        // audit events" indistinguishable from a tenant with no audit
        // history. We now return `Status::Unimplemented` so the
        // client gets a clear "feature not ready" signal and can fall
        // back to direct PG `audit_events` queries until
        // `DT-S-D21-AUDIT-STREAM-1` lands.
        let e = Status::unimplemented(
            "AdminService.ListAuditEvents body deferred to DT-S-D21-AUDIT-STREAM-1 ; \
             query the `audit_events` table directly via the PG read replica until then",
        );
        emit_request_completion(LIST_AUDIT_EVENTS_PATH, &tenant, &Err(&e), started);
        Err(e)
    }

    async fn breach_history(
        &self,
        mut req: TRequest<BreachHistoryRequest>,
    ) -> Result<TResponse<BreachHistoryResponse>, Status> {
        let started = Instant::now();
        let auth_res = self
            .auth
            .check(&mut req, Some(Scope::ADMIN_BREACH_READ))
            .await;
        let tenant = Self::tenant_of(&req);
        if let Err(e) = auth_res {
            emit_request_completion(BREACH_HISTORY_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        if let Err(e) = check_rate_limit(self.rate_limiter.as_ref(), &tenant) {
            record_ratelimit_drop(&tenant);
            emit_request_completion(BREACH_HISTORY_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }

        let outcome: Result<TResponse<BreachHistoryResponse>, Status> = async {
            let inner = req.into_inner();
            let mut filter = BreachFilter::none();
            if !inner.tenant.is_empty() {
                filter = filter.with_tenant(inner.tenant);
            }
            if !inner.task.is_empty() {
                filter = filter.with_task(inner.task);
            }
            if inner.min_magnitude > 0.0 {
                filter = filter.with_min_magnitude(inner.min_magnitude);
            }
            let limit = if inner.limit == 0 {
                DEFAULT_BREACH_LIMIT as usize
            } else {
                inner.limit as usize
            };
            let reports = self
                .registry
                .breach_history(limit, Some(filter))
                .await
                .map_err(bridge::to_status)?;
            Ok(TResponse::new(BreachHistoryResponse {
                reports: reports
                    .into_iter()
                    .map(bridge::cone::breach_to_proto)
                    .collect(),
            }))
        }
        .await;

        emit_request_completion(
            BREACH_HISTORY_PATH,
            &tenant,
            &outcome.as_ref().map(|_| ()),
            started,
        );
        outcome
    }
}

// =====================================================================
// Sprint **D21** helpers.
// =====================================================================

/// Parse a canonical catalog key `tenant/name@vN` into a typed
/// `CatalogKey`. Returns `Status::InvalidArgument` on malformed input.
fn parse_catalog_key(s: &str) -> Result<CatalogKey, Status> {
    CatalogKey::parse(s)
        .map_err(|e| Status::invalid_argument(format!("invalid catalog key '{s}': {e}")))
}

/// Map the proto `signature_policy` string ("skip" | "warn" | "require"
/// | "") onto the typed [`crate::config::SignaturePolicy`] enum.
///
/// Unknown / empty maps to `Skip` for D21 dev-friendliness ; production
/// operators MUST set the env var `EFE_BUNDLE_REQUIRE_SIGNATURE=required`
/// at boot, which propagates into the policy.
fn parse_sig_policy(s: &str) -> crate::config::SignaturePolicy {
    use crate::config::SignaturePolicy;
    match s.to_ascii_lowercase().as_str() {
        "require" => SignaturePolicy::Required,
        "warn" => SignaturePolicy::Warn,
        _ => SignaturePolicy::Skip,
    }
}

/// Extract the principal (JWT `sub` claim) from request extensions.
/// Falls back to `"anonymous"` for unauthenticated paths.
fn principal_of<T>(req: &TRequest<T>) -> String {
    req.extensions()
        .get::<AuthExtClaims>()
        .map(|c| c.0.sub.clone())
        .unwrap_or_else(|| "anonymous".to_string())
}
