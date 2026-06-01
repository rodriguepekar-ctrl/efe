//! Authentication subsystem — JWT HS256 + scope enforcement.
//!
//! Sprint **D15** extends the legacy HS256 path with an
//! [`OidcValidator`] that multiplexes multiple issuers (Auth0,
//! Keycloak, Okta, Azure AD…) via JWKS pull + per-issuer
//! `alg_whitelist`. The gRPC interceptors continue to call
//! [`AuthGuard`] ; the guard internally dispatches to either
//! `JwtValidator` (M10 HS256) or `OidcValidator` (D15 RS256/ES256)
//! based on `auth.oidc.enabled`.
//!
//! The public entry point is [`AuthGuard`]; handlers call
//! [`AuthGuard::check`] with their statically-known required scope.

pub mod guard;
/// Sprint **D15** — runtime registry of OIDC issuers loaded from
/// `oidc-issuers.json`. Cf. `docs/bench/d15_onboarding_idp_architecture.md`
/// § 5.2.
pub mod issuers;
/// Sprint **D15** — JWKS cache with stale-while-revalidate, periodic
/// refresh, and pluggable HTTP fetcher (real `reqwest` or test mock).
/// Cf. `docs/bench/d15_onboarding_idp_architecture.md` § 5.3 + § 12.3.
pub mod jwks;
pub mod jwt;
/// Sprint **D22** (M4 § 10.2) — typed mTLS configuration + validator.
/// Surfaces a typed boot-time error when mTLS is mandatory but the
/// client-CA is absent (audit fix S-A1). The actual `tonic` integration
/// lives in [`crate::runtime::Server::serve`].
pub mod mtls;
/// Sprint **D15** — multi-issuer OIDC JWT validator (RS256/ES256).
///
/// Dispatches by JWT `iss` claim, enforces per-issuer alg whitelist,
/// resolves tenant_id from claim or fixed mapping. Cf.
/// `docs/bench/d15_onboarding_idp_architecture.md` § 5–§ 12.
pub mod oidc;
/// Sprint **D22** (M4 § 10.3) — per-RPC scope guard + 3 broad scopes
/// (`data` / `admin` / `audit`). See [`scope_guard::ScopeGuard`].
pub mod scope_guard;
pub mod scopes;
/// Sprint **D15** — Tenant registry CRUD layer wrapping
/// `migrations/0004_tenants.sql`. Cf.
/// `docs/bench/d15_onboarding_idp_architecture.md` § 4 + § 15.
pub mod tenants;
pub mod token_store;

pub use guard::AuthGuard;
pub use issuers::{
    load_issuers_file, IssuerConfig, IssuerEntry, IssuerLoadError, IssuerRegistry, LoadParams,
    TenantStrategy,
};
pub use jwks::{periodic_refresh, JwksCache, JwksDocument, JwksError, JwksFetcher};
pub use jwt::{Claims, JwtValidator};
pub use mtls::{MtlsConfig, MtlsError};
pub use oidc::{OidcError, OidcValidator, Outcome, TenantStatusGate};
pub use scope_guard::{BroadScope, ScopeGuard};
pub use scopes::{required_scope, Scope};
pub use tenants::{
    validate_tenant_id, IdpStrategy, NewTenant, Tenant, TenantStatus, TenantStore, TenantStoreError,
};
pub use token_store::TokenStore;
