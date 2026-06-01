//! `AuthGuard` — per-handler authentication and scope enforcement.
//!
//! **Why a guard, not an interceptor.** `tonic::service::interceptor`
//! receives `Request<()>` **after** the URI has been stripped (see
//! `tonic::service::interceptor::InterceptedService::call`), so a single
//! generic interceptor cannot tell which RPC was invoked and therefore
//! cannot compute the required scope. The interceptor-based design in
//! earlier M10 drafts silently denied every authenticated request.
//!
//! The guard is passed to each service impl and invoked explicitly at
//! the start of every handler with the statically-known scope. This is
//! verbose but reliable, easily unit-testable, and composable with
//! `tonic-health`, `tonic-reflection`, and arbitrary service stacks.
//!
//! Sprint **D15** extends the guard with two backends — `Hs256`
//! (legacy M10 path, single shared secret) and `Oidc` (multi-issuer
//! RS256/ES256 with JWKS pull cache). The selection is made at
//! `runtime::Server::build` time based on `auth.oidc.enabled`.

use std::sync::Arc;

use tonic::{Request as TRequest, Status};

use crate::auth::{Claims, OidcValidator, TokenStore};
use crate::interceptors::{AuthExtClaims, RequestTenant};
use efe_core::swarm::TraceContext;

/// Selectable validator backend.
///
/// The two variants are mutually exclusive at runtime — either we
/// validate against the legacy HS256 token store *or* we delegate to
/// the OIDC multi-issuer validator. Mixing the two would force every
/// handler to know which strategy applies and would introduce a
/// matrix of configuration combinations no one tested.
#[derive(Clone)]
enum Backend {
    /// Sprint M10 — single-secret HS256 with rotation.
    Hs256(Arc<TokenStore>),
    /// Sprint D15 — multi-issuer OIDC.
    Oidc(Arc<OidcValidator>),
}

impl std::fmt::Debug for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Backend::Hs256(_) => f.debug_tuple("Backend::Hs256").finish(),
            Backend::Oidc(v) => f
                .debug_struct("Backend::Oidc")
                .field("issuers", &v.n_issuers())
                .finish(),
        }
    }
}

/// Shared handle for authentication decisions.
///
/// Call [`AuthGuard::check`] at the top of every RPC handler. It :
/// - validates the bearer token when `enabled` ;
/// - enforces the statically-provided `scope` ;
/// - installs the authenticated `Claims`, a `RequestTenant`, and any
///   `traceparent` header into the request extensions for downstream
///   code to consume.
#[derive(Clone, Debug)]
pub struct AuthGuard {
    backend: Option<Backend>,
    enabled: bool,
}

impl AuthGuard {
    /// Disabled guard — installs `anonymous` claims on every call and
    /// returns `Ok` regardless of scope.
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            backend: None,
            enabled: false,
        }
    }

    /// Enabled guard backed by the legacy HS256 [`TokenStore`].
    #[must_use]
    pub fn enabled(token_store: Arc<TokenStore>) -> Self {
        Self {
            backend: Some(Backend::Hs256(token_store)),
            enabled: true,
        }
    }

    /// Sprint **D15** — enabled guard backed by an [`OidcValidator`].
    /// The validator dispatches by JWT `iss` claim across multiple
    /// IdPs (Auth0, Keycloak, Okta, Azure AD…).
    #[must_use]
    pub fn enabled_oidc(validator: Arc<OidcValidator>) -> Self {
        Self {
            backend: Some(Backend::Oidc(validator)),
            enabled: true,
        }
    }

    /// `true` when token validation is required.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Validate the request and ensure `required_scope` is granted.
    ///
    /// On success :
    /// - `Claims` are stored in extensions as [`AuthExtClaims`] ;
    /// - the tenant id is also stored as [`RequestTenant`] for the
    ///   rate-limiter ;
    /// - a `traceparent` header, if any, is parsed into
    ///   [`TraceContext`] and stored in extensions.
    ///
    /// Pass `required_scope = None` for public endpoints (health,
    /// reflection) to opt out of auth entirely while still installing
    /// the anonymous claim + trace context for observability.
    ///
    /// # Errors
    ///
    /// - [`Status::unauthenticated`] if the token is missing or invalid.
    /// - [`Status::permission_denied`] if the scope is not granted.
    pub async fn check<T>(
        &self,
        req: &mut TRequest<T>,
        required_scope: Option<&'static str>,
    ) -> Result<Claims, Status> {
        // Always extract the traceparent (cheap and does not depend on
        // auth state), before any early-return path.
        extract_trace_context(req);

        // Public endpoint: always anonymous.
        if required_scope.is_none() {
            let c = Claims::anonymous();
            install_claims(req, &c);
            return Ok(c);
        }

        // Auth disabled — anonymous but still pass.
        if !self.enabled {
            let c = Claims::anonymous();
            install_claims(req, &c);
            return Ok(c);
        }

        // Auth enabled but no backend: configuration bug, surface it.
        let Some(backend) = self.backend.as_ref() else {
            return Err(Status::internal(
                "auth enabled but no validator backend configured",
            ));
        };

        // Extract bearer.
        let token = extract_bearer_token(req)
            .ok_or_else(|| Status::unauthenticated("missing or malformed bearer token"))?;
        let claims = match backend {
            Backend::Hs256(store) => store
                .validate(&token)
                .map_err(|_| Status::unauthenticated("invalid token"))?,
            Backend::Oidc(validator) => validator.validate(&token).await.map_err(|e| match e {
                crate::auth::OidcError::TenantResolutionFailed(_) => {
                    Status::permission_denied("tenant resolution failed")
                }
                _ => Status::unauthenticated("invalid OIDC token"),
            })?,
        };

        // Scope check.
        let scope = required_scope.expect("checked above");
        if scope == "__denied__" {
            return Err(Status::permission_denied("endpoint not allowed"));
        }
        if !claims.has_scope(scope) {
            return Err(Status::permission_denied(format!(
                "missing required scope: {scope}"
            )));
        }

        install_claims(req, &claims);
        Ok(claims)
    }
}

fn install_claims<T>(req: &mut TRequest<T>, claims: &Claims) {
    let tenant = claims.tenant_id.clone();
    req.extensions_mut().insert(AuthExtClaims(claims.clone()));
    req.extensions_mut().insert(RequestTenant(tenant));
}

fn extract_trace_context<T>(req: &mut TRequest<T>) {
    let md = req.metadata();
    let traceparent = md
        .get("traceparent")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let tracestate = md
        .get("tracestate")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    if let Some(tp) = traceparent {
        req.extensions_mut().insert(TraceContext {
            traceparent: tp,
            tracestate,
        });
    }
}

fn extract_bearer_token<T>(req: &TRequest<T>) -> Option<String> {
    let value = req.metadata().get("authorization")?.to_str().ok()?;
    value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{Scope, TokenStore};
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    fn sign(secret: &[u8], scopes: &[&str], tenant: &str) -> String {
        let claims = Claims {
            iss: None,
            sub: "svc".into(),
            aud: "efe.v1".into(),
            exp: now_secs() + 60,
            iat: None,
            tenant_id: tenant.into(),
            scopes: scopes.iter().map(|s| (*s).to_string()).collect(),
        };
        encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(secret),
        )
        .unwrap()
    }

    fn req(token: Option<&str>, traceparent: Option<&str>) -> TRequest<()> {
        let mut r = TRequest::new(());
        if let Some(t) = token {
            r.metadata_mut()
                .insert("authorization", format!("Bearer {t}").parse().unwrap());
        }
        if let Some(tp) = traceparent {
            r.metadata_mut().insert("traceparent", tp.parse().unwrap());
        }
        r
    }

    // Extension lookup helpers.
    fn tenant_of<T>(r: &TRequest<T>) -> Option<String> {
        r.extensions().get::<RequestTenant>().map(|t| t.0.clone())
    }
    fn trace_of<T>(r: &TRequest<T>) -> Option<TraceContext> {
        r.extensions().get::<TraceContext>().cloned()
    }

    #[tokio::test]
    async fn disabled_accepts_public_endpoint() {
        let g = AuthGuard::disabled();
        let mut r = req(None, None);
        let c = g.check(&mut r, None).await.unwrap();
        assert_eq!(c.tenant_id, "anonymous");
        assert_eq!(tenant_of(&r).as_deref(), Some("anonymous"));
    }

    #[tokio::test]
    async fn disabled_accepts_authenticated_endpoint_as_anonymous() {
        let g = AuthGuard::disabled();
        let mut r = req(None, None);
        let c = g.check(&mut r, Some(Scope::SWARM_EXECUTE)).await.unwrap();
        assert_eq!(c.tenant_id, "anonymous");
    }

    #[tokio::test]
    async fn enabled_without_backend_returns_internal() {
        // Build the guard via the disabled() helper then flip the
        // `enabled` flag back to true while leaving `backend = None`.
        // This simulates the misconfiguration "auth=on but no
        // validator wired" — Server::build catches it via config
        // validation in production.
        let mut g = AuthGuard::disabled();
        g.enabled = true;
        let mut r = req(None, None);
        let err = g
            .check(&mut r, Some(Scope::SWARM_EXECUTE))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::Internal);
    }

    #[tokio::test]
    async fn enabled_public_endpoint_stays_anonymous() {
        let secret = b"sec".to_vec();
        let store = TokenStore::from_hs256(&[secret], "efe.v1", None).unwrap();
        let g = AuthGuard::enabled(Arc::new(store));
        let mut r = req(None, None);
        let c = g.check(&mut r, None).await.unwrap();
        assert_eq!(c.tenant_id, "anonymous");
    }

    #[tokio::test]
    async fn enabled_missing_token_unauthenticated() {
        let secret = b"sec".to_vec();
        let store = TokenStore::from_hs256(&[secret], "efe.v1", None).unwrap();
        let g = AuthGuard::enabled(Arc::new(store));
        let mut r = req(None, None);
        let err = g
            .check(&mut r, Some(Scope::SWARM_EXECUTE))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    #[tokio::test]
    async fn enabled_invalid_token_unauthenticated() {
        let secret = b"sec".to_vec();
        let store = TokenStore::from_hs256(&[secret], "efe.v1", None).unwrap();
        let g = AuthGuard::enabled(Arc::new(store));
        let mut r = req(Some("garbage"), None);
        let err = g
            .check(&mut r, Some(Scope::SWARM_EXECUTE))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    #[tokio::test]
    async fn enabled_valid_token_and_scope_ok() {
        let secret = b"sec".to_vec();
        let store = TokenStore::from_hs256(&[secret.clone()], "efe.v1", None).unwrap();
        let g = AuthGuard::enabled(Arc::new(store));
        let tok = sign(&secret, &[Scope::SWARM_EXECUTE], "acme");
        let mut r = req(Some(&tok), None);
        let c = g.check(&mut r, Some(Scope::SWARM_EXECUTE)).await.unwrap();
        assert_eq!(c.tenant_id, "acme");
        assert_eq!(tenant_of(&r).as_deref(), Some("acme"));
    }

    #[tokio::test]
    async fn enabled_valid_token_missing_scope_permission_denied() {
        let secret = b"sec".to_vec();
        let store = TokenStore::from_hs256(&[secret.clone()], "efe.v1", None).unwrap();
        let g = AuthGuard::enabled(Arc::new(store));
        let tok = sign(&secret, &[Scope::SWARM_DESCRIBE], "acme");
        let mut r = req(Some(&tok), None);
        let err = g
            .check(&mut r, Some(Scope::SWARM_EXECUTE))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
    }

    #[tokio::test]
    async fn trace_context_extracted_regardless_of_auth_state() {
        let g = AuthGuard::disabled();
        let mut r = req(None, Some("00-abc123-def456-01"));
        g.check(&mut r, Some(Scope::SWARM_EXECUTE)).await.unwrap();
        let tc = trace_of(&r).unwrap();
        assert_eq!(tc.traceparent, "00-abc123-def456-01");
    }

    #[tokio::test]
    async fn denied_scope_path_rejected() {
        let secret = b"sec".to_vec();
        let store = TokenStore::from_hs256(&[secret.clone()], "efe.v1", None).unwrap();
        let g = AuthGuard::enabled(Arc::new(store));
        let tok = sign(&secret, &["__denied__"], "acme");
        let mut r = req(Some(&tok), None);
        let err = g.check(&mut r, Some("__denied__")).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
    }
}
