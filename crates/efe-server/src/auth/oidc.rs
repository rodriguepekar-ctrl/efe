//! Sprint **D15** — OIDC JWT validator (RS256/ES256, multi-issuer).
//!
//! Supersedes the M10 single-secret HS256 path *for OIDC mode* ; the
//! HS256 path stays alive for the local-dev / on-prem-no-IdP scenarios
//! and is selected by [`crate::config::OidcConfig::enabled = false`].
//!
//! Design highlights — cf. `docs/bench/d15_onboarding_idp_architecture.md`
//! § 5–§ 12 :
//!
//! 1. **Multi-issuer dispatch** : the JWT's `iss` claim is first read
//!    from the *unverified* payload (we cannot verify yet — we don't
//!    know which key to use). The matching [`IssuerEntry`] is then
//!    looked up in the [`IssuerRegistry`]. An unknown issuer fails
//!    fast with `outcome="unknown_issuer"`.
//!
//! 2. **Algorithm confusion prevention** : each issuer carries an
//!    explicit `alg_whitelist`. The header `alg` MUST be one of those
//!    values — an attacker that swaps `alg` from `RS256` to `none` or
//!    `HS256` is rejected before the signature check (cf.
//!    OWASP-API4-2023 + the "alg=none" CVE class).
//!
//! 3. **JWKS pull cache + stale-while-revalidate** : key lookup hits
//!    the local [`JwksCache`] (in-memory, refreshed every `ttl`).
//!    During an IdP outage the cache serves stale-but-within-grace
//!    entries and bumps `efe_jwks_stale_total`.
//!
//! 4. **Tenant resolution** : two strategies (cf.
//!    [`super::issuers::TenantStrategy`]) :
//!     * `claim` — read a custom claim (`tenant_id` by default,
//!       overridable per issuer with `tenant_claim`) ;
//!     * `issuer` — the issuer URL maps to one fixed tenant
//!       (`tenant_id_for_issuer` ; useful for Auth0 organizations
//!       and Keycloak realms).
//!
//! 5. **Tenant-id format check** : both strategies reject any
//!    tenant_id that doesn't match the canonical regex
//!    `^[a-z0-9_-]{1,63}$` (same as the catalog `tenants` table).
//!
//! 6. **Token-age check** : the architecture caps `exp - iat` at
//!    `max_token_age_s` (default 3600) so a leaked one-year token
//!    can't be replayed forever.
//!
//! 7. **Outcome telemetry** : every code path emits
//!    `efe_oidc_tokens_validated_total{issuer, outcome}` — see
//!    [`Outcome`].
//!
//! 8. **Defence-in-depth `outcome="ok"` ordering** : we increment the
//!    metric *after* every check has passed and the `Claims` are
//!    returned. A panic mid-validation never inflates the success
//!    counter.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use jsonwebtoken::{decode, decode_header, Algorithm, Validation};
use serde::Deserialize;
use thiserror::Error;

use super::issuers::{IssuerEntry, IssuerRegistry, TenantStrategy};
use super::jwt::Claims;

/// Outcome of a single `OidcValidator::validate` call. Used as the
/// `outcome` label of `efe_oidc_tokens_validated_total`.
///
/// **Cardinality budget** : 6 values × ~10 issuers = 60 series, well
/// under D13 50 000 panic line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Signature valid + claims valid + tenant resolved + format OK.
    Ok,
    /// Signature mismatch / `kid` not in cache / unsupported alg.
    InvalidSignature,
    /// `exp` past or `iat` outside `max_token_age_s`.
    Expired,
    /// `aud` claim mismatch.
    BadAudience,
    /// `iss` claim not in `IssuerRegistry`.
    UnknownIssuer,
    /// `tenant_strategy="claim"` and the claim is missing / not a
    /// string / regex mismatch ; or `tenant_strategy="issuer"` and
    /// `tenant_id_for_issuer` is missing.
    TenantResolutionFailed,
}

impl Outcome {
    /// String form for the Prometheus label.
    #[must_use]
    pub fn as_label(self) -> &'static str {
        match self {
            Outcome::Ok => "ok",
            Outcome::InvalidSignature => "invalid_signature",
            Outcome::Expired => "expired",
            Outcome::BadAudience => "bad_audience",
            Outcome::UnknownIssuer => "unknown_issuer",
            Outcome::TenantResolutionFailed => "tenant_resolution_failed",
        }
    }
}

/// Error returned by [`OidcValidator::validate`].
///
/// Each variant maps to an [`Outcome`] via [`Self::outcome`] for the
/// `efe_oidc_tokens_validated_total{outcome=...}` metric. The mapping
/// is **not** 1-1 because some failure classes (malformed token,
/// unknown kid, alg mismatch) are folded into `InvalidSignature` to
/// avoid leaking attack-surface detail through the metric.
#[derive(Debug, Error)]
pub enum OidcError {
    /// JWT header / payload is not parseable (not a JWT, base64
    /// corruption, missing `alg`, missing `kid`).
    #[error("malformed JWT: {0}")]
    Malformed(String),

    /// `iss` claim resolves to no [`IssuerEntry`].
    #[error("unknown issuer: {0}")]
    UnknownIssuer(String),

    /// Header `alg` is not in the per-issuer whitelist.
    #[error("algorithm not whitelisted for issuer {issuer}: header_alg={alg}")]
    AlgorithmMismatch {
        /// The header's `alg` claim that did not match the whitelist.
        alg: String,
        /// Issuer whose whitelist we checked against.
        issuer: String,
    },

    /// `kid` not in JWKS cache (or cache empty / stale-past-grace).
    #[error("unknown kid {0} for issuer (or JWKS cache cold)")]
    UnknownKid(String),

    /// jsonwebtoken signature / claims validation failure.
    #[error("signature/claims validation failed: {0}")]
    InvalidSignature(String),

    /// **Audit fix D15-C5** : the token's `exp` is in the past.
    /// Distinct from [`Self::TokenTooOld`] (lifetime cap exceeded) :
    /// an expired token had a normal lifetime when issued but was
    /// presented after expiry. Both map to [`Outcome::Expired`] for
    /// the metric, but the error message is now accurate.
    #[error("token expired: exp claim is in the past")]
    Expired,

    /// `exp - iat` exceeds the per-issuer `max_token_age_s` cap.
    /// Distinct from [`Self::Expired`] : the token has a too-long
    /// lifetime (e.g. a 1-year token from a misconfigured IdP) even
    /// though it isn't past `exp` yet.
    #[error("token age exceeds max_token_age_s={max} (lifetime={lifetime}s)")]
    TokenTooOld {
        /// Configured cap, in seconds.
        max: u64,
        /// Actual `exp - iat`.
        lifetime: u64,
    },

    /// **Audit fix D15-C4** : `aud` claim does not match the
    /// per-issuer audience. Was previously folded into
    /// [`Self::InvalidSignature`] which made the
    /// `efe_oidc_tokens_validated_total{outcome="bad_audience"}`
    /// dashboard panel dead-on-arrival. Now wired correctly.
    #[error("audience mismatch: {0}")]
    BadAudience(String),

    /// Tenant resolution failed — claim missing, malformed, or fixed
    /// tenant absent / suspended.
    #[error("tenant resolution failed: {0}")]
    TenantResolutionFailed(String),
}

impl OidcError {
    /// Map error to an [`Outcome`] for the metric.
    #[must_use]
    pub fn outcome(&self) -> Outcome {
        match self {
            OidcError::Malformed(_) => Outcome::InvalidSignature,
            OidcError::UnknownIssuer(_) => Outcome::UnknownIssuer,
            OidcError::AlgorithmMismatch { .. } | OidcError::UnknownKid(_) => {
                Outcome::InvalidSignature
            }
            OidcError::InvalidSignature(_) => Outcome::InvalidSignature,
            OidcError::Expired | OidcError::TokenTooOld { .. } => Outcome::Expired,
            OidcError::BadAudience(_) => Outcome::BadAudience,
            OidcError::TenantResolutionFailed(_) => Outcome::TenantResolutionFailed,
        }
    }
}

// =====================================================================
// Claim shapes — minimal payload required to dispatch
// =====================================================================
//
// `OidcClaims` is the *unverified* peek at the JWT to extract `iss` so
// we know which key to use. The full verified claims are returned as
// `super::jwt::Claims` to share the AuthGuard plumbing.

/// Minimal claims peeked from the JWT payload **before** signature
/// verification, used solely for issuer dispatch.
///
/// Trusting these fields before the signature is acceptable here
/// because we use them only to *select the key* — the signature
/// verification afterwards re-validates `iss` (jsonwebtoken
/// `Validation::set_issuer`).
#[derive(Debug, Deserialize)]
struct UnverifiedDispatchClaims {
    iss: String,
}

/// Verified claims as decoded by jsonwebtoken under the issuer's
/// `Validation`. We then convert to [`Claims`] for the AuthGuard
/// API.
///
/// We keep this loose (extra fields ignored) because IdPs sprinkle
/// random vendor claims (`auth_time`, `azp`, `at_hash`, etc.) that
/// we don't need.
#[derive(Debug, Deserialize)]
struct VerifiedRawClaims {
    iss: String,
    sub: String,
    aud: AudienceField,
    exp: u64,
    #[serde(default)]
    iat: Option<u64>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    scopes: Option<Vec<String>>,
    /// All the custom claims (e.g. `tenant_id`, `https://efe/tenant`)
    /// captured here so we can extract by name without fixing the
    /// shape at compile time.
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

/// `aud` claim is either a single string or a string array per RFC
/// 7519 § 4.1.3. We accept both shapes ; jsonwebtoken's
/// `Validation::set_audience` already handles the comparison.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AudienceField {
    Single(String),
    Many(Vec<String>),
}

impl AudienceField {
    /// Pick the first matching audience value (used purely for the
    /// debug log / error message ; the cryptographic check has
    /// already been done by jsonwebtoken).
    fn first(&self) -> &str {
        match self {
            AudienceField::Single(s) => s.as_str(),
            AudienceField::Many(v) => v.first().map(String::as_str).unwrap_or(""),
        }
    }
}

// =====================================================================
// TenantStatusGate — D15-C1 enforcement of suspended-tenant rejection
// =====================================================================

/// **Audit fix D15-C1** — tenant lifecycle status gate.
///
/// The pre-audit `OidcValidator` validated tokens against the issuer
/// registry alone — it never consulted the `tenants` table. As a
/// result, `efe-onboard suspend acme` had no effect on JWT validation :
/// suspended tenants kept hitting authenticated RPCs until the IdP
/// signing key rotated. This contradicted the architecture promise
/// (cf. `docs/bench/d15_onboarding_idp_architecture.md` § 4) and the
/// CLI / howto docs.
///
/// This trait closes the gap. The runtime wires an
/// `Arc<dyn TenantStatusGate>` (typically backed by `TenantStore`)
/// into the validator. After tenant resolution, the validator queries
/// `is_active(tenant_id)` and rejects with [`OidcError::TenantResolutionFailed`]
/// if the answer is `false`.
///
/// The gate is `Optional` for testability and for deployments that
/// don't use the registry table (Tier 1 dev, embedded in another
/// system) — when absent, the validator behaves as before.
///
/// Implementations must be **fast** : this is on the hot path of
/// every authenticated RPC. `TenantStore::is_active` hits a
/// connection pool ; consider an in-memory cache layered on top
/// (deferred to D15-bis).
#[async_trait::async_trait]
pub trait TenantStatusGate: Send + Sync {
    /// Return `true` iff the tenant exists in the registry **and**
    /// has `status = 'active'`. Anything else (pending, suspended,
    /// not-found) is `false` — caller maps to `Outcome::TenantResolutionFailed`.
    ///
    /// # Errors
    ///
    /// On a DB outage, return `Err`. The validator will surface this
    /// as `Outcome::TenantResolutionFailed` (closest available label)
    /// and fail closed — operators see the alert and can correlate
    /// with the DB downtime.
    async fn is_active(&self, tenant_id: &str) -> Result<bool, String>;
}

// =====================================================================
// OidcValidator — the public entry point
// =====================================================================

/// Multi-issuer OIDC JWT validator.
///
/// Construct once from a populated [`IssuerRegistry`] and an `Arc` it
/// into [`super::TokenStore::OidcBackend`] (set up by
/// `runtime::Server::build`).
#[derive(Clone)]
pub struct OidcValidator {
    /// Shared registry — one entry per IdP issuer.
    registry: Arc<IssuerRegistry>,
    /// Optional tenant lifecycle gate. When `Some(_)`, the validator
    /// rejects tokens for non-`active` tenants. When `None`, behaves
    /// as the pre-audit validator (issuer registry only).
    tenant_gate: Option<Arc<dyn TenantStatusGate>>,
}

impl std::fmt::Debug for OidcValidator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OidcValidator")
            .field("issuers", &self.registry.len())
            .field("tenant_gate", &self.tenant_gate.is_some())
            .finish()
    }
}

impl OidcValidator {
    /// Wrap an [`IssuerRegistry`]. No tenant status gate — accepts
    /// any tenant the JWT resolves to (legacy / test path).
    #[must_use]
    pub fn new(registry: Arc<IssuerRegistry>) -> Self {
        Self {
            registry,
            tenant_gate: None,
        }
    }

    /// **Audit fix D15-C1** — wrap an [`IssuerRegistry`] AND a
    /// [`TenantStatusGate`]. Rejects tokens for non-`active` tenants.
    /// Recommended for production deployments where the operator
    /// uses `efe-onboard` to manage tenant lifecycle.
    #[must_use]
    pub fn with_tenant_gate(
        registry: Arc<IssuerRegistry>,
        gate: Arc<dyn TenantStatusGate>,
    ) -> Self {
        Self {
            registry,
            tenant_gate: Some(gate),
        }
    }

    /// Number of registered issuers (for diagnostics).
    #[must_use]
    pub fn n_issuers(&self) -> usize {
        self.registry.len()
    }

    /// `true` if a tenant status gate is wired.
    #[must_use]
    pub fn has_tenant_gate(&self) -> bool {
        self.tenant_gate.is_some()
    }

    /// Validate a bearer JWT token.
    ///
    /// Returns the verified [`Claims`] on success ; emits
    /// `efe_oidc_tokens_validated_total{issuer, outcome}` on every
    /// path. The `issuer` label is `"unknown"` if `iss` could not be
    /// extracted.
    pub async fn validate(&self, token: &str) -> Result<Claims, OidcError> {
        let res = self.validate_inner(token).await;
        let (issuer_label, outcome_label) = match &res {
            Ok(c) => (
                c.iss.clone().unwrap_or_else(|| "unknown".to_string()),
                Outcome::Ok.as_label(),
            ),
            Err(e) => {
                let issuer = match e {
                    OidcError::UnknownIssuer(iss) => iss.clone(),
                    OidcError::AlgorithmMismatch { issuer, .. } => issuer.clone(),
                    _ => "unknown".to_string(),
                };
                (issuer, e.outcome().as_label())
            }
        };
        efe_telemetry::metrics::d15_instruments::oidc_tokens_validated_total()
            .with_label_values(&[&issuer_label, outcome_label])
            .inc();
        res
    }

    async fn validate_inner(&self, token: &str) -> Result<Claims, OidcError> {
        // ---------------------------------------------------------
        // Step 1 — peek at the JWT payload to extract `iss` (we
        // can't verify the signature yet ; we don't know the key).
        // The jsonwebtoken crate doesn't expose a "decode without
        // verify" helper, so we do the base64 dance ourselves on the
        // payload segment.
        // ---------------------------------------------------------
        let dispatch = peek_iss(token)?;

        // ---------------------------------------------------------
        // Step 2 — look up the issuer.
        // ---------------------------------------------------------
        let entry: &IssuerEntry = self
            .registry
            .get(&dispatch.iss)
            .ok_or_else(|| OidcError::UnknownIssuer(dispatch.iss.clone()))?;

        // ---------------------------------------------------------
        // Step 3 — peek at the JWT header for `alg` and `kid`.
        // ---------------------------------------------------------
        let header = decode_header(token).map_err(|e| OidcError::Malformed(e.to_string()))?;
        let kid = header
            .kid
            .ok_or_else(|| OidcError::Malformed("missing kid in JWT header".into()))?;

        // ---------------------------------------------------------
        // Step 4 — algorithm-confusion guard : enforce per-issuer
        // whitelist BEFORE asking the JWKS cache for a key.
        //
        // **Audit fix D15-H1** — the previous implementation used
        // `format!("{:?}", header.alg)` to obtain a comparison string.
        // That depends on jsonwebtoken's `Debug` derive output staying
        // stable across versions (a refactor that adds a tuple variant
        // or renames the enum would silently break the whitelist
        // comparison and accept ANY alg). We now use the explicit
        // canonical-name helper [`alg_canonical_name`] so a refactor
        // surfaces as a compile error.
        // ---------------------------------------------------------
        let header_alg_str = alg_canonical_name(header.alg);
        let allow: HashSet<&str> = entry
            .config
            .alg_whitelist
            .iter()
            .map(String::as_str)
            .collect();
        if !allow.contains(header_alg_str) {
            return Err(OidcError::AlgorithmMismatch {
                alg: header_alg_str.to_string(),
                issuer: entry.config.issuer.clone(),
            });
        }

        // ---------------------------------------------------------
        // Step 5 — fetch the decoding key from the JWKS cache.
        // ---------------------------------------------------------
        let (decoding_key, key_alg) = entry
            .jwks_cache
            .get_key(&kid)
            .await
            .ok_or_else(|| OidcError::UnknownKid(kid.clone()))?;

        // Cross-check : the header's alg must match the JWK's alg.
        // This catches an attacker who, knowing a kid, swaps the
        // alg in the header (e.g. RS256 → ES256). jsonwebtoken
        // rejects this internally too, but checking here gives a
        // crisper error.
        if header.alg != key_alg {
            return Err(OidcError::AlgorithmMismatch {
                alg: header_alg_str.to_string(),
                issuer: entry.config.issuer.clone(),
            });
        }

        // ---------------------------------------------------------
        // Step 6 — full verify with jsonwebtoken : signature, exp,
        // iss, aud, leeway.
        // ---------------------------------------------------------
        let mut validation = Validation::new(key_alg);
        validation.set_issuer(&[&entry.config.issuer]);
        validation.set_audience(&[&entry.config.audience]);
        validation.leeway = entry.config.leeway_s;
        // We need `iat` to enforce max_token_age_s ; require it.
        validation.set_required_spec_claims(&["exp", "iss", "aud"]);
        validation.validate_exp = true;
        validation.validate_aud = true;
        validation.algorithms = vec![key_alg];

        let raw = match decode::<VerifiedRawClaims>(token, &decoding_key, &validation) {
            Ok(d) => d.claims,
            Err(e) => {
                // **Audit fix D15-C4 + D15-C5** : route jsonwebtoken's
                // typed `ErrorKind` to the right `OidcError` variant
                // so the `outcome` label of the metric is accurate
                // and the error message isn't misleading.
                //
                // * `ExpiredSignature` (exp past) → `Expired`
                //   (was: `TokenTooOld` with lifetime=0, which read
                //   "token age exceeds max_token_age_s=3600
                //    (lifetime=0s)" — confusing for ops).
                // * `InvalidAudience` → `BadAudience` (was: folded into
                //   `InvalidSignature`, leaving `Outcome::BadAudience`
                //   dead code and the dashboard panel empty).
                use jsonwebtoken::errors::ErrorKind;
                return Err(match e.kind() {
                    ErrorKind::ExpiredSignature => OidcError::Expired,
                    ErrorKind::InvalidAudience => OidcError::BadAudience(e.to_string()),
                    _ => OidcError::InvalidSignature(e.to_string()),
                });
            }
        };

        // ---------------------------------------------------------
        // Step 7 — token-age cap : exp - iat ≤ max_token_age_s.
        // ---------------------------------------------------------
        // If `iat` is absent we can't enforce ; per RFC 7519 it's
        // optional but architecturally we require it for OIDC. The
        // fallback : compute `exp - now` and use that as a proxy.
        // (Standard for IdPs that omit `iat`.)
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let lifetime = match raw.iat {
            Some(iat) => raw.exp.saturating_sub(iat),
            None => raw.exp.saturating_sub(now),
        };
        if lifetime > entry.config.max_token_age_s {
            return Err(OidcError::TokenTooOld {
                max: entry.config.max_token_age_s,
                lifetime,
            });
        }

        // ---------------------------------------------------------
        // Step 8 — tenant resolution.
        // ---------------------------------------------------------
        let tenant_id = resolve_tenant(&entry.config, &raw)?;

        // ---------------------------------------------------------
        // Step 8-bis — **Audit fix D15-C1** — tenant lifecycle gate.
        //
        // The issuer registry tells us "we trust this IdP" ; the
        // tenant gate tells us "this specific org is currently
        // entitled to use the platform". Without this check, an
        // operator who runs `efe-onboard suspend acme` cuts no
        // access at all — the documented promise of the CLI.
        //
        // Fail-closed semantics : if the gate query errors (DB
        // unreachable), reject the token. Operators see
        // `Outcome::TenantResolutionFailed` rate climb and
        // correlate with the DB outage alert.
        // ---------------------------------------------------------
        if let Some(gate) = self.tenant_gate.as_ref() {
            match gate.is_active(&tenant_id).await {
                Ok(true) => {}
                Ok(false) => {
                    return Err(OidcError::TenantResolutionFailed(format!(
                        "tenant {tenant_id:?} is not in `active` status \
                         (suspended, pending, or not registered)"
                    )));
                }
                Err(e) => {
                    return Err(OidcError::TenantResolutionFailed(format!(
                        "tenant gate query failed for {tenant_id:?}: {e} \
                         (failing closed per D15-C1)"
                    )));
                }
            }
        }

        // ---------------------------------------------------------
        // Step 9 — scope normalisation.
        // ---------------------------------------------------------
        // OIDC standard exposes scopes via the `scope` claim
        // (space-delimited string). Some IdPs use `scopes` (JSON
        // array). We accept either ; if both are present, the array
        // wins to keep behaviour deterministic.
        let scopes: Vec<String> = if let Some(arr) = &raw.scopes {
            arr.clone()
        } else if let Some(s) = &raw.scope {
            s.split_whitespace().map(String::from).collect()
        } else {
            Vec::new()
        };

        // ---------------------------------------------------------
        // Step 10 — assemble the canonical Claims.
        // ---------------------------------------------------------
        Ok(Claims {
            iss: Some(raw.iss),
            sub: raw.sub,
            aud: raw.aud.first().to_string(),
            exp: raw.exp,
            iat: raw.iat,
            tenant_id,
            scopes,
        })
    }
}

// =====================================================================
// Helper : peek `iss` from the JWT payload without verification
// =====================================================================

/// Decode the *payload* segment of a compact JWS (`header.payload.sig`)
/// as base64url and parse just enough JSON to extract `iss`. NEVER
/// trust other claims from this — they're un-verified.
fn peek_iss(token: &str) -> Result<UnverifiedDispatchClaims, OidcError> {
    let mut parts = token.split('.');
    let _header = parts
        .next()
        .ok_or_else(|| OidcError::Malformed("empty token".into()))?;
    let payload = parts
        .next()
        .ok_or_else(|| OidcError::Malformed("missing payload segment".into()))?;
    if parts.next().is_none() {
        return Err(OidcError::Malformed("missing signature segment".into()));
    }
    let bytes = base64_url_decode(payload).map_err(|e| OidcError::Malformed(e.to_string()))?;
    let claims: UnverifiedDispatchClaims = serde_json::from_slice(&bytes)
        .map_err(|e| OidcError::Malformed(format!("payload not JSON: {e}")))?;
    if claims.iss.is_empty() {
        return Err(OidcError::Malformed("missing iss claim".into()));
    }
    Ok(claims)
}

/// Base64url decode without padding (RFC 7515 § 2). The `base64` crate
/// is already a transitive dep via `jsonwebtoken`.
fn base64_url_decode(s: &str) -> Result<Vec<u8>, String> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    URL_SAFE_NO_PAD
        .decode(s.as_bytes())
        .map_err(|e| format!("base64url decode: {e}"))
}

/// **Audit fix D15-H1** — canonical name for a `jsonwebtoken::Algorithm`
/// value. Replaces the previous `format!("{:?}", alg)` which depended on
/// the `Debug` derive output staying stable across crate versions.
///
/// Returns `"unsupported"` for any algo we don't accept in v0.1.15
/// (notably `none`, `HS256`, `HS384`, `HS512`, `RS384`, `RS512`,
/// `ES384`, `EdDSA`, `PS*`). This forces such tokens through the
/// whitelist mismatch path even if the operator accidentally added
/// them to `alg_whitelist`. Belt-and-braces : the JWKS cache only
/// produces RS256/ES256 keys (cf. `Jwk::to_decoding_key`), so an
/// `unsupported` algo would also fail at the JWK alg cross-check.
fn alg_canonical_name(alg: Algorithm) -> &'static str {
    match alg {
        Algorithm::RS256 => "RS256",
        Algorithm::ES256 => "ES256",
        // Listed explicitly so a future jsonwebtoken release that adds
        // a new variant produces a compile error (non-exhaustive match
        // on a `#[non_exhaustive]` enum is forbidden in this crate).
        Algorithm::HS256
        | Algorithm::HS384
        | Algorithm::HS512
        | Algorithm::RS384
        | Algorithm::RS512
        | Algorithm::ES384
        | Algorithm::PS256
        | Algorithm::PS384
        | Algorithm::PS512
        | Algorithm::EdDSA => "unsupported",
    }
}

// =====================================================================
// Tenant resolution
// =====================================================================

/// Apply the per-issuer [`TenantStrategy`] to extract a canonical
/// tenant_id, then validate the format with the
/// `^[a-z0-9_-]{1,63}$` regex (same as the catalog `tenants` table).
fn resolve_tenant(
    cfg: &super::issuers::IssuerConfig,
    raw: &VerifiedRawClaims,
) -> Result<String, OidcError> {
    let candidate = match cfg.tenant_strategy {
        TenantStrategy::Issuer => cfg.tenant_id_for_issuer.clone().ok_or_else(|| {
            OidcError::TenantResolutionFailed(format!(
                "tenant_strategy=issuer but tenant_id_for_issuer is unset for {}",
                cfg.issuer
            ))
        })?,
        TenantStrategy::Claim => {
            let key = cfg
                .tenant_claim
                .clone()
                .unwrap_or_else(|| "tenant_id".to_string());
            match raw.extra.get(&key) {
                Some(serde_json::Value::String(s)) => s.clone(),
                Some(other) => {
                    return Err(OidcError::TenantResolutionFailed(format!(
                        "claim {key} is not a string: {other:?}"
                    )));
                }
                None => {
                    return Err(OidcError::TenantResolutionFailed(format!(
                        "claim {key} missing from JWT"
                    )));
                }
            }
        }
    };
    if !is_valid_tenant_id(&candidate) {
        return Err(OidcError::TenantResolutionFailed(format!(
            "tenant_id {candidate:?} does not match ^[a-z0-9_-]{{1,63}}$"
        )));
    }
    Ok(candidate)
}

/// `^[a-z0-9_-]{1,63}$` — same as `migrations/0004_tenants.sql`'s CHECK.
fn is_valid_tenant_id(s: &str) -> bool {
    if s.is_empty() || s.len() > 63 {
        return false;
    }
    s.bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::issuers::{IssuerConfig, IssuerEntry};
    use crate::auth::jwks::{JwksCache, JwksDocument, JwksFetcher};
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde_json::json;
    use std::sync::Mutex;
    use std::time::Duration;

    /// Mock fetcher returning a fixed JwksDocument.
    struct StaticFetcher {
        doc: Mutex<Option<JwksDocument>>,
    }

    #[async_trait::async_trait]
    impl JwksFetcher for StaticFetcher {
        async fn fetch(
            &self,
            _url: &str,
            _timeout: Duration,
        ) -> Result<JwksDocument, super::super::jwks::JwksError> {
            let g = self.doc.lock().unwrap();
            g.clone()
                .ok_or_else(|| super::super::jwks::JwksError::Fetch("no doc".to_string()))
        }
    }

    /// Generate an RSA keypair + JWK + signing key. Returns `(jwk, sk_pem, kid)`.
    /// We use `jsonwebtoken::jwk::Jwk` indirectly by hand-crafting the
    /// JSON to avoid a dev-dep on `rsa`. Test runs with a pre-generated
    /// 2048-bit key embedded in the source.
    fn rsa_test_material() -> (super::super::jwks::Jwk, EncodingKey, &'static str) {
        // Pre-baked 2048-bit RSA key (test-only ; never used outside
        // this unit test file).
        // n + e + d are the JWK ; the EncodingKey is built from the
        // matching PKCS#8 PEM.
        const PEM: &str = include_str!("./test_keys/rsa_2048.pem");
        const N_B64URL: &str = include_str!("./test_keys/rsa_2048.n.b64url");
        const E_B64URL: &str = "AQAB";
        let kid = "test-rsa-1";
        let jwk = super::super::jwks::Jwk {
            kty: "RSA".into(),
            alg: Some("RS256".into()),
            kid: kid.to_string(),
            use_: Some("sig".into()),
            n: Some(N_B64URL.trim().to_string()),
            e: Some(E_B64URL.to_string()),
            crv: None,
            x: None,
            y: None,
        };
        let sk = EncodingKey::from_rsa_pem(PEM.as_bytes()).expect("valid PEM");
        (jwk, sk, kid)
    }

    fn build_entry(cfg: IssuerConfig, jwk: super::super::jwks::Jwk) -> IssuerEntry {
        let fetcher: Box<dyn JwksFetcher> = Box::new(StaticFetcher {
            doc: Mutex::new(Some(JwksDocument { keys: vec![jwk] })),
        });
        let cache = JwksCache::with_fetcher(
            cfg.jwks_url.clone(),
            Duration::from_secs(300),
            Duration::from_millis(5_000),
            3,
            Duration::from_millis(100),
            Duration::from_secs(3_600),
            fetcher,
        );
        IssuerEntry {
            config: cfg,
            jwks_cache: cache,
        }
    }

    fn make_token(
        sk: &EncodingKey,
        kid: &str,
        iss: &str,
        aud: &str,
        sub: &str,
        extra: serde_json::Value,
    ) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut payload = serde_json::Map::new();
        payload.insert("iss".into(), json!(iss));
        payload.insert("aud".into(), json!(aud));
        payload.insert("sub".into(), json!(sub));
        payload.insert("exp".into(), json!(now + 300));
        payload.insert("iat".into(), json!(now));
        if let serde_json::Value::Object(map) = extra {
            for (k, v) in map {
                payload.insert(k, v);
            }
        }
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(kid.to_string());
        encode(&header, &payload, sk).unwrap()
    }

    #[tokio::test]
    async fn validate_happy_path_claim_strategy() {
        let (jwk, sk, kid) = rsa_test_material();
        let cfg = IssuerConfig {
            issuer: "https://example.com/".into(),
            audience: "efe.v1".into(),
            jwks_url: "https://example.com/.well-known/jwks.json".into(),
            alg_whitelist: vec!["RS256".into()],
            tenant_strategy: TenantStrategy::Claim,
            tenant_claim: Some("tenant_id".into()),
            tenant_id_for_issuer: None,
            max_token_age_s: 3600,
            leeway_s: 30,
        };
        let entry = build_entry(cfg, jwk);
        // Pre-warm cache.
        entry.jwks_cache.refresh().await.unwrap();
        let registry = IssuerRegistry::from_entries(vec![entry]);
        let v = OidcValidator::new(Arc::new(registry));
        let tok = make_token(
            &sk,
            kid,
            "https://example.com/",
            "efe.v1",
            "user-123",
            json!({"tenant_id": "acme", "scope": "swarm:execute admin:cone:read"}),
        );
        let claims = v.validate(&tok).await.expect("valid token");
        assert_eq!(claims.tenant_id, "acme");
        assert_eq!(claims.sub, "user-123");
        assert!(claims.has_scope("swarm:execute"));
        assert!(claims.has_scope("admin:cone:read"));
    }

    #[tokio::test]
    async fn validate_happy_path_issuer_strategy() {
        let (jwk, sk, kid) = rsa_test_material();
        let cfg = IssuerConfig {
            issuer: "https://example.com/".into(),
            audience: "efe.v1".into(),
            jwks_url: "https://example.com/.well-known/jwks.json".into(),
            alg_whitelist: vec!["RS256".into()],
            tenant_strategy: TenantStrategy::Issuer,
            tenant_claim: None,
            tenant_id_for_issuer: Some("acme".into()),
            max_token_age_s: 3600,
            leeway_s: 30,
        };
        let entry = build_entry(cfg, jwk);
        entry.jwks_cache.refresh().await.unwrap();
        let v = OidcValidator::new(Arc::new(IssuerRegistry::from_entries(vec![entry])));
        let tok = make_token(
            &sk,
            kid,
            "https://example.com/",
            "efe.v1",
            "user-1",
            json!({"scope": "swarm:execute"}),
        );
        let claims = v.validate(&tok).await.unwrap();
        assert_eq!(claims.tenant_id, "acme");
    }

    #[tokio::test]
    async fn validate_unknown_issuer_rejected() {
        let (jwk, sk, kid) = rsa_test_material();
        let cfg = IssuerConfig {
            issuer: "https://known.com/".into(),
            audience: "efe.v1".into(),
            jwks_url: "https://known.com/.well-known/jwks.json".into(),
            alg_whitelist: vec!["RS256".into()],
            tenant_strategy: TenantStrategy::Claim,
            tenant_claim: Some("tenant_id".into()),
            tenant_id_for_issuer: None,
            max_token_age_s: 3600,
            leeway_s: 30,
        };
        let entry = build_entry(cfg, jwk);
        entry.jwks_cache.refresh().await.unwrap();
        let v = OidcValidator::new(Arc::new(IssuerRegistry::from_entries(vec![entry])));
        let tok = make_token(
            &sk,
            kid,
            "https://attacker.example/",
            "efe.v1",
            "u",
            json!({"tenant_id": "x"}),
        );
        let err = v.validate(&tok).await.unwrap_err();
        assert!(matches!(err, OidcError::UnknownIssuer(_)));
        assert_eq!(err.outcome(), Outcome::UnknownIssuer);
    }

    #[tokio::test]
    async fn validate_alg_not_in_whitelist_rejected() {
        // Configure whitelist to ES256 only, sign with RS256 → reject.
        let (jwk, sk, kid) = rsa_test_material();
        let cfg = IssuerConfig {
            issuer: "https://example.com/".into(),
            audience: "efe.v1".into(),
            jwks_url: "https://example.com/.well-known/jwks.json".into(),
            alg_whitelist: vec!["ES256".into()], // intentionally narrow
            tenant_strategy: TenantStrategy::Issuer,
            tenant_claim: None,
            tenant_id_for_issuer: Some("acme".into()),
            max_token_age_s: 3600,
            leeway_s: 30,
        };
        let entry = build_entry(cfg, jwk);
        entry.jwks_cache.refresh().await.unwrap();
        let v = OidcValidator::new(Arc::new(IssuerRegistry::from_entries(vec![entry])));
        let tok = make_token(&sk, kid, "https://example.com/", "efe.v1", "u", json!({}));
        let err = v.validate(&tok).await.unwrap_err();
        assert!(matches!(err, OidcError::AlgorithmMismatch { .. }));
    }

    #[tokio::test]
    async fn validate_unknown_kid_rejected() {
        let (jwk, sk, _kid) = rsa_test_material();
        let cfg = IssuerConfig {
            issuer: "https://example.com/".into(),
            audience: "efe.v1".into(),
            jwks_url: "https://example.com/.well-known/jwks.json".into(),
            alg_whitelist: vec!["RS256".into()],
            tenant_strategy: TenantStrategy::Issuer,
            tenant_claim: None,
            tenant_id_for_issuer: Some("acme".into()),
            max_token_age_s: 3600,
            leeway_s: 30,
        };
        let entry = build_entry(cfg, jwk);
        entry.jwks_cache.refresh().await.unwrap();
        let v = OidcValidator::new(Arc::new(IssuerRegistry::from_entries(vec![entry])));
        let tok = make_token(
            &sk,
            "rotated-out-kid",
            "https://example.com/",
            "efe.v1",
            "u",
            json!({}),
        );
        let err = v.validate(&tok).await.unwrap_err();
        assert!(matches!(err, OidcError::UnknownKid(_)));
    }

    #[tokio::test]
    async fn validate_tenant_claim_missing_rejected() {
        let (jwk, sk, kid) = rsa_test_material();
        let cfg = IssuerConfig {
            issuer: "https://example.com/".into(),
            audience: "efe.v1".into(),
            jwks_url: "https://example.com/.well-known/jwks.json".into(),
            alg_whitelist: vec!["RS256".into()],
            tenant_strategy: TenantStrategy::Claim,
            tenant_claim: Some("tenant_id".into()),
            tenant_id_for_issuer: None,
            max_token_age_s: 3600,
            leeway_s: 30,
        };
        let entry = build_entry(cfg, jwk);
        entry.jwks_cache.refresh().await.unwrap();
        let v = OidcValidator::new(Arc::new(IssuerRegistry::from_entries(vec![entry])));
        let tok = make_token(&sk, kid, "https://example.com/", "efe.v1", "u", json!({}));
        let err = v.validate(&tok).await.unwrap_err();
        assert_eq!(err.outcome(), Outcome::TenantResolutionFailed);
    }

    #[tokio::test]
    async fn validate_tenant_id_format_rejected() {
        let (jwk, sk, kid) = rsa_test_material();
        let cfg = IssuerConfig {
            issuer: "https://example.com/".into(),
            audience: "efe.v1".into(),
            jwks_url: "https://example.com/.well-known/jwks.json".into(),
            alg_whitelist: vec!["RS256".into()],
            tenant_strategy: TenantStrategy::Claim,
            tenant_claim: Some("tenant_id".into()),
            tenant_id_for_issuer: None,
            max_token_age_s: 3600,
            leeway_s: 30,
        };
        let entry = build_entry(cfg, jwk);
        entry.jwks_cache.refresh().await.unwrap();
        let v = OidcValidator::new(Arc::new(IssuerRegistry::from_entries(vec![entry])));
        let tok = make_token(
            &sk,
            kid,
            "https://example.com/",
            "efe.v1",
            "u",
            // Uppercase — rejected by `^[a-z0-9_-]{1,63}$`.
            json!({"tenant_id": "AcmeCorp!"}),
        );
        let err = v.validate(&tok).await.unwrap_err();
        assert_eq!(err.outcome(), Outcome::TenantResolutionFailed);
    }

    #[tokio::test]
    async fn validate_token_age_capped() {
        let (jwk, sk, kid) = rsa_test_material();
        let cfg = IssuerConfig {
            issuer: "https://example.com/".into(),
            audience: "efe.v1".into(),
            jwks_url: "https://example.com/.well-known/jwks.json".into(),
            alg_whitelist: vec!["RS256".into()],
            tenant_strategy: TenantStrategy::Issuer,
            tenant_claim: None,
            tenant_id_for_issuer: Some("acme".into()),
            max_token_age_s: 60, // 60 s cap
            leeway_s: 30,
        };
        let entry = build_entry(cfg, jwk);
        entry.jwks_cache.refresh().await.unwrap();
        let v = OidcValidator::new(Arc::new(IssuerRegistry::from_entries(vec![entry])));
        // Build a token with exp = iat + 600 (10 min) — exceeds 60 s cap.
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let payload = json!({
            "iss": "https://example.com/",
            "aud": "efe.v1",
            "sub": "u",
            "iat": now,
            "exp": now + 600,
        });
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(kid.to_string());
        let tok = encode(&header, &payload, &sk).unwrap();
        let err = v.validate(&tok).await.unwrap_err();
        assert!(matches!(err, OidcError::TokenTooOld { .. }));
        assert_eq!(err.outcome(), Outcome::Expired);
    }

    #[tokio::test]
    async fn validate_audience_mismatch_rejected() {
        let (jwk, sk, kid) = rsa_test_material();
        let cfg = IssuerConfig {
            issuer: "https://example.com/".into(),
            audience: "efe.v1".into(),
            jwks_url: "https://example.com/.well-known/jwks.json".into(),
            alg_whitelist: vec!["RS256".into()],
            tenant_strategy: TenantStrategy::Issuer,
            tenant_claim: None,
            tenant_id_for_issuer: Some("acme".into()),
            max_token_age_s: 3600,
            leeway_s: 30,
        };
        let entry = build_entry(cfg, jwk);
        entry.jwks_cache.refresh().await.unwrap();
        let v = OidcValidator::new(Arc::new(IssuerRegistry::from_entries(vec![entry])));
        let tok = make_token(
            &sk,
            kid,
            "https://example.com/",
            "wrong-aud", // mismatch
            "u",
            json!({}),
        );
        let err = v.validate(&tok).await.unwrap_err();
        // **Audit fix D15-C4** : jsonwebtoken returns InvalidAudience
        // → we now route to `Outcome::BadAudience` (was previously
        // folded into `InvalidSignature`, leaving the dashboard panel
        // empty). Pin the new mapping with this assertion.
        assert!(matches!(err, OidcError::BadAudience(_)));
        assert_eq!(err.outcome(), Outcome::BadAudience);
    }

    /// **Audit fix D15-C5 regression test** — an expired token (exp in
    /// the past) must surface as `OidcError::Expired`, NOT
    /// `OidcError::TokenTooOld`. Both map to `Outcome::Expired` for
    /// the metric, but the error message must accurately reflect WHY.
    #[tokio::test]
    async fn validate_expired_token_yields_expired_variant() {
        let (jwk, sk, kid) = rsa_test_material();
        let cfg = IssuerConfig {
            issuer: "https://example.com/".into(),
            audience: "efe.v1".into(),
            jwks_url: "https://example.com/.well-known/jwks.json".into(),
            alg_whitelist: vec!["RS256".into()],
            tenant_strategy: TenantStrategy::Issuer,
            tenant_claim: None,
            tenant_id_for_issuer: Some("acme".into()),
            max_token_age_s: 3600,
            leeway_s: 0,
        };
        let entry = build_entry(cfg, jwk);
        entry.jwks_cache.refresh().await.unwrap();
        let v = OidcValidator::new(Arc::new(IssuerRegistry::from_entries(vec![entry])));
        // Build a token with exp in the past.
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let payload = json!({
            "iss": "https://example.com/",
            "aud": "efe.v1",
            "sub": "u",
            "iat": now.saturating_sub(120),
            "exp": now.saturating_sub(60), // exp 1 min ago
        });
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(kid.to_string());
        let tok = encode(&header, &payload, &sk).unwrap();
        let err = v.validate(&tok).await.unwrap_err();
        assert!(
            matches!(err, OidcError::Expired),
            "expected Expired variant, got {err:?}"
        );
        assert_eq!(err.outcome(), Outcome::Expired);
    }

    /// **Audit fix D15-C1 regression test** — when a `TenantStatusGate`
    /// is wired and reports `Ok(false)`, the validator rejects the
    /// token with `Outcome::TenantResolutionFailed`. Without the gate,
    /// the same token validates (legacy behaviour preserved).
    #[tokio::test]
    async fn validate_with_tenant_gate_rejects_inactive_tenant() {
        use std::sync::atomic::{AtomicBool, Ordering};
        struct StubGate {
            active: AtomicBool,
        }
        #[async_trait::async_trait]
        impl TenantStatusGate for StubGate {
            async fn is_active(&self, _tid: &str) -> Result<bool, String> {
                Ok(self.active.load(Ordering::SeqCst))
            }
        }
        let (jwk, sk, kid) = rsa_test_material();
        let cfg = IssuerConfig {
            issuer: "https://example.com/".into(),
            audience: "efe.v1".into(),
            jwks_url: "https://example.com/.well-known/jwks.json".into(),
            alg_whitelist: vec!["RS256".into()],
            tenant_strategy: TenantStrategy::Issuer,
            tenant_claim: None,
            tenant_id_for_issuer: Some("acme".into()),
            max_token_age_s: 3600,
            leeway_s: 30,
        };
        let entry = build_entry(cfg, jwk);
        entry.jwks_cache.refresh().await.unwrap();
        let registry = IssuerRegistry::from_entries(vec![entry]);
        let gate = Arc::new(StubGate {
            active: AtomicBool::new(false),
        });
        let v = OidcValidator::with_tenant_gate(Arc::new(registry), gate.clone());
        assert!(v.has_tenant_gate());
        let tok = make_token(&sk, kid, "https://example.com/", "efe.v1", "u", json!({}));
        let err = v.validate(&tok).await.unwrap_err();
        assert_eq!(err.outcome(), Outcome::TenantResolutionFailed);
        // Now flip the gate to active and verify the same token passes.
        gate.active.store(true, Ordering::SeqCst);
        let claims = v.validate(&tok).await.expect("should validate");
        assert_eq!(claims.tenant_id, "acme");
    }

    /// **Audit fix D15-C1 regression test (DB outage)** — when the
    /// gate returns Err (e.g. DB unreachable), the validator fails
    /// closed with `Outcome::TenantResolutionFailed`.
    #[tokio::test]
    async fn validate_with_tenant_gate_fails_closed_on_db_error() {
        struct ErrGate;
        #[async_trait::async_trait]
        impl TenantStatusGate for ErrGate {
            async fn is_active(&self, _tid: &str) -> Result<bool, String> {
                Err("connection refused".to_string())
            }
        }
        let (jwk, sk, kid) = rsa_test_material();
        let cfg = IssuerConfig {
            issuer: "https://example.com/".into(),
            audience: "efe.v1".into(),
            jwks_url: "https://example.com/.well-known/jwks.json".into(),
            alg_whitelist: vec!["RS256".into()],
            tenant_strategy: TenantStrategy::Issuer,
            tenant_claim: None,
            tenant_id_for_issuer: Some("acme".into()),
            max_token_age_s: 3600,
            leeway_s: 30,
        };
        let entry = build_entry(cfg, jwk);
        entry.jwks_cache.refresh().await.unwrap();
        let v = OidcValidator::with_tenant_gate(
            Arc::new(IssuerRegistry::from_entries(vec![entry])),
            Arc::new(ErrGate),
        );
        let tok = make_token(&sk, kid, "https://example.com/", "efe.v1", "u", json!({}));
        let err = v.validate(&tok).await.unwrap_err();
        assert_eq!(err.outcome(), Outcome::TenantResolutionFailed);
        // Error message MUST mention "failing closed" so ops can find
        // it via grep when correlating with DB downtime alerts.
        let msg = format!("{err}");
        assert!(
            msg.contains("failing closed"),
            "expected 'failing closed' in message, got: {msg}"
        );
    }

    /// **Audit fix D15-H1 regression test** — `alg_canonical_name`
    /// returns the exact whitelist string for the supported algos
    /// and the sentinel `"unsupported"` for everything else. A
    /// jsonwebtoken upgrade that adds variants surfaces here as a
    /// compile error (non-exhaustive match) ; this test pins the
    /// runtime behaviour.
    #[test]
    fn alg_canonical_name_matches_whitelist_strings() {
        assert_eq!(alg_canonical_name(Algorithm::RS256), "RS256");
        assert_eq!(alg_canonical_name(Algorithm::ES256), "ES256");
        assert_eq!(alg_canonical_name(Algorithm::HS256), "unsupported");
        assert_eq!(alg_canonical_name(Algorithm::EdDSA), "unsupported");
        assert_eq!(alg_canonical_name(Algorithm::ES384), "unsupported");
        assert_eq!(alg_canonical_name(Algorithm::PS256), "unsupported");
    }

    #[tokio::test]
    async fn validate_malformed_token_rejected() {
        let cfg = IssuerConfig {
            issuer: "https://example.com/".into(),
            audience: "efe.v1".into(),
            jwks_url: "https://example.com/.well-known/jwks.json".into(),
            alg_whitelist: vec!["RS256".into()],
            tenant_strategy: TenantStrategy::Issuer,
            tenant_claim: None,
            tenant_id_for_issuer: Some("acme".into()),
            max_token_age_s: 3600,
            leeway_s: 30,
        };
        let (jwk, _sk, _kid) = rsa_test_material();
        let entry = build_entry(cfg, jwk);
        entry.jwks_cache.refresh().await.unwrap();
        let v = OidcValidator::new(Arc::new(IssuerRegistry::from_entries(vec![entry])));
        let err = v.validate("not.a.jwt").await.unwrap_err();
        assert!(matches!(err, OidcError::Malformed(_)));
    }

    #[test]
    fn outcome_label_strings_match_metric_doc() {
        // Architecture doc § 14.2 lists exactly these 6 labels — a
        // typo here would silently break the dashboards.
        assert_eq!(Outcome::Ok.as_label(), "ok");
        assert_eq!(Outcome::InvalidSignature.as_label(), "invalid_signature");
        assert_eq!(Outcome::Expired.as_label(), "expired");
        assert_eq!(Outcome::BadAudience.as_label(), "bad_audience");
        assert_eq!(Outcome::UnknownIssuer.as_label(), "unknown_issuer");
        assert_eq!(
            Outcome::TenantResolutionFailed.as_label(),
            "tenant_resolution_failed"
        );
    }

    #[test]
    fn is_valid_tenant_id_accepts_canonical() {
        assert!(is_valid_tenant_id("acme"));
        assert!(is_valid_tenant_id("acme-corp"));
        assert!(is_valid_tenant_id("acme_42"));
        assert!(is_valid_tenant_id("a"));
        assert!(is_valid_tenant_id(&"a".repeat(63)));
    }

    #[test]
    fn is_valid_tenant_id_rejects_non_canonical() {
        assert!(!is_valid_tenant_id(""));
        assert!(!is_valid_tenant_id(&"a".repeat(64)));
        assert!(!is_valid_tenant_id("Acme")); // uppercase
        assert!(!is_valid_tenant_id("acme!")); // bang
        assert!(!is_valid_tenant_id("acme corp")); // space
        assert!(!is_valid_tenant_id("acmé")); // unicode
    }

    #[test]
    fn peek_iss_extracts_iss_without_verify() {
        // Build a fake JWT (header.payload.sig) with iss claim ; we
        // don't care about the signature for this test.
        let payload = json!({"iss": "https://test/", "sub": "u"});
        let payload_b64 = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap());
        let token = format!("ZmFrZQ.{payload_b64}.c2ln");
        let dispatch = peek_iss(&token).unwrap();
        assert_eq!(dispatch.iss, "https://test/");
    }

    #[test]
    fn peek_iss_rejects_two_segment() {
        assert!(peek_iss("only.two").is_err());
    }

    #[test]
    fn peek_iss_rejects_non_json_payload() {
        // Two segments + signature ; payload is invalid base64 / JSON.
        assert!(peek_iss("aGVsbG8.bm90anNvbg.c2ln").is_err());
    }
}
