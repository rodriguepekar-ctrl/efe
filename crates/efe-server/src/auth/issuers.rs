//! Sprint **D15** — Issuer registry for OIDC validation.
//!
//! Mirror of the design document
//! [`docs/bench/d15_onboarding_idp_architecture.md`](../../../docs/bench/d15_onboarding_idp_architecture.md)
//! § 12 + § 13.
//!
//! The registry is the **single source of truth** for which IdPs the
//! cluster trusts. Loaded from a JSON file at boot ; immutable for
//! the lifetime of the process (a SIGHUP-triggered reload could be
//! added in D17+ but isn't needed for v0.1.15 — operators rebuild
//! the deployment to add an IdP).
//!
//! # Format
//!
//! See `auth/issuers.json` example in the architecture doc § 13.2.
//! In short : a JSON array of [`IssuerConfig`] entries.
//!
//! # Validation
//!
//! Every issuer is validated at load time :
//!
//! * `issuer` URL is HTTPS (HTTP allowed only when
//!   `EFE_OIDC_ALLOW_HTTP=true` for Tier 1 dev).
//! * `audience` non-empty (refuses common typo `aud` vs `audience`).
//! * `jwks_url` HTTPS.
//! * `alg_whitelist` ⊂ {RS256, ES256} (we don't support EdDSA in
//!   v0.1.15 ; the validate refuses any other algo to mitigate
//!   algorithm-confusion attacks per § 10 of the architecture doc).
//! * `tenant_strategy` ∈ {`claim`, `issuer`}.
//! * If `claim` → `tenant_claim` non-empty.
//! * If `issuer` → `tenant_id_for_issuer` matches the
//!   `^[a-z0-9_-]{1,63}$` regex (consistent with D14 audit fix C1).

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::jwks::JwksCache;

// =====================================================================
// IssuerConfig — entry in `issuers.json`
// =====================================================================

/// One IdP issuer's configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssuerConfig {
    /// Issuer URL. Must match the JWT `iss` claim exactly.
    pub issuer: String,
    /// Expected JWT `aud` claim value.
    pub audience: String,
    /// JWKS endpoint URL (typically `${issuer}/.well-known/jwks.json`,
    /// but Auth0 / Cognito have non-standard paths so we don't
    /// derive it).
    pub jwks_url: String,
    /// Permitted `alg` values for the JWT header. Restricting per
    /// issuer mitigates algorithm-confusion attacks (cf. § 10 of
    /// the architecture doc).
    pub alg_whitelist: Vec<String>,
    /// How to resolve the EFE tenant_id from the JWT.
    pub tenant_strategy: TenantStrategy,
    /// Claim name to read when `tenant_strategy = "claim"`. Common
    /// values : `"tenant_id"`, `"https://efe.example/tenant"`.
    #[serde(default)]
    pub tenant_claim: Option<String>,
    /// Static tenant_id when `tenant_strategy = "issuer"` (each issuer
    /// = one tenant ; useful for Auth0 organizations / Keycloak
    /// realms).
    #[serde(default)]
    pub tenant_id_for_issuer: Option<String>,
    /// Maximum allowed JWT lifetime, in seconds. Tokens whose
    /// `exp - iat` exceeds this are rejected even if the signature
    /// is valid. Default 3600 (1 h).
    #[serde(default = "default_max_token_age_s")]
    pub max_token_age_s: u64,
    /// `exp` / `nbf` clock skew tolerance, in seconds. Default 30.
    #[serde(default = "default_leeway_s")]
    pub leeway_s: u64,
}

fn default_max_token_age_s() -> u64 {
    3600
}
fn default_leeway_s() -> u64 {
    30
}

/// How to resolve the EFE tenant_id from a JWT.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantStrategy {
    /// Read a custom claim from the JWT payload.
    Claim,
    /// Map the issuer URL to a fixed tenant_id (1 IdP = 1 tenant).
    Issuer,
}

// =====================================================================
// IssuerEntry — runtime form (registered with JwksCache)
// =====================================================================

/// Runtime entry — pairs an [`IssuerConfig`] with its [`JwksCache`].
pub struct IssuerEntry {
    pub config: IssuerConfig,
    pub jwks_cache: JwksCache,
}

impl std::fmt::Debug for IssuerEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IssuerEntry")
            .field("issuer", &self.config.issuer)
            .field("audience", &self.config.audience)
            .field("alg_whitelist", &self.config.alg_whitelist)
            .field("tenant_strategy", &self.config.tenant_strategy)
            .finish()
    }
}

// =====================================================================
// IssuerRegistry — the loaded set of issuers
// =====================================================================

/// Maps `iss` URL → [`IssuerEntry`]. Look up O(1) by JWT `iss` claim.
pub struct IssuerRegistry {
    by_iss: HashMap<String, IssuerEntry>,
}

impl IssuerRegistry {
    /// Construct from a list of pre-built entries (used by the
    /// integration tests after wiring the JwksCache mocks).
    #[must_use]
    pub fn from_entries(entries: Vec<IssuerEntry>) -> Self {
        let mut by_iss = HashMap::with_capacity(entries.len());
        for e in entries {
            by_iss.insert(e.config.issuer.clone(), e);
        }
        Self { by_iss }
    }

    /// Number of registered issuers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_iss.len()
    }

    /// True iff no issuer is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_iss.is_empty()
    }

    /// Lookup an entry by issuer URL. The URL must match the JWT
    /// `iss` claim byte-for-byte (we do NOT normalise trailing
    /// slashes — operators must configure the exact issuer URL the
    /// IdP emits).
    #[must_use]
    pub fn get(&self, iss: &str) -> Option<&IssuerEntry> {
        self.by_iss.get(iss)
    }

    /// Iterate over every entry. Used by the runtime to spawn one
    /// `JwksCache::periodic_refresh` task per issuer.
    pub fn iter(&self) -> impl Iterator<Item = &IssuerEntry> {
        self.by_iss.values()
    }
}

impl std::fmt::Debug for IssuerRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IssuerRegistry")
            .field("len", &self.by_iss.len())
            .field("issuers", &self.by_iss.keys().collect::<Vec<_>>())
            .finish()
    }
}

// =====================================================================
// Errors
// =====================================================================

#[derive(Debug, Error)]
pub enum IssuerLoadError {
    #[error("issuers file IO error at {path:?}: {source}")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("issuers file at {path:?} is not valid JSON: {source}")]
    Json {
        path: std::path::PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("issuers file at {path:?} is empty (no issuers configured)")]
    Empty { path: std::path::PathBuf },

    #[error("issuer {issuer:?} is invalid: {reason}")]
    Invalid { issuer: String, reason: String },

    #[error("duplicate issuer {issuer:?} (defined more than once in the file)")]
    Duplicate { issuer: String },
}

// =====================================================================
// Loader (validates + builds JwksCache for each entry)
// =====================================================================

/// Sprint **D15** parameters for the loader. Replicate the operator-
/// level OidcConfig values that affect every JwksCache.
#[derive(Clone, Debug)]
pub struct LoadParams {
    pub jwks_ttl_s: u64,
    pub jwks_fetch_timeout_ms: u64,
    pub jwks_retry_max: u32,
    pub jwks_backoff_ms: u64,
    pub grace_stale_s: u64,
}

impl LoadParams {
    /// Convenience builder from [`crate::config::OidcConfig`].
    #[must_use]
    pub fn from_oidc_config(cfg: &crate::config::OidcConfig) -> Self {
        Self {
            jwks_ttl_s: cfg.jwks_ttl_s,
            jwks_fetch_timeout_ms: cfg.jwks_fetch_timeout_ms,
            jwks_retry_max: cfg.jwks_retry_max,
            jwks_backoff_ms: cfg.jwks_backoff_ms,
            grace_stale_s: cfg.grace_stale_s,
        }
    }
}

/// Parse + validate the issuers JSON file.
///
/// Each entry's [`JwksCache`] is constructed but **NOT pre-warmed** ;
/// the runtime spawns `periodic_refresh` per entry to fetch the
/// keys lazily. Tokens validation through the registry triggers an
/// on-demand fetch the first time a kid is missing from the cache.
pub fn load_issuers_file(
    path: &Path,
    params: &LoadParams,
) -> Result<IssuerRegistry, IssuerLoadError> {
    let raw = std::fs::read_to_string(path).map_err(|e| IssuerLoadError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let configs: Vec<IssuerConfig> =
        serde_json::from_str(&raw).map_err(|e| IssuerLoadError::Json {
            path: path.to_path_buf(),
            source: e,
        })?;
    if configs.is_empty() {
        return Err(IssuerLoadError::Empty {
            path: path.to_path_buf(),
        });
    }

    let mut entries: Vec<IssuerEntry> = Vec::with_capacity(configs.len());
    let mut seen: HashMap<String, ()> = HashMap::new();

    for cfg in configs {
        validate_issuer_config(&cfg)?;
        if seen.insert(cfg.issuer.clone(), ()).is_some() {
            return Err(IssuerLoadError::Duplicate {
                issuer: cfg.issuer.clone(),
            });
        }
        let cache = JwksCache::new(
            cfg.jwks_url.clone(),
            std::time::Duration::from_secs(params.jwks_ttl_s),
            std::time::Duration::from_millis(params.jwks_fetch_timeout_ms),
            params.jwks_retry_max,
            std::time::Duration::from_millis(params.jwks_backoff_ms),
            std::time::Duration::from_secs(params.grace_stale_s),
        );
        entries.push(IssuerEntry {
            config: cfg,
            jwks_cache: cache,
        });
    }

    Ok(IssuerRegistry::from_entries(entries))
}

/// Per-entry validation. Checks every invariant from § 13.3 of the
/// architecture doc.
pub fn validate_issuer_config(cfg: &IssuerConfig) -> Result<(), IssuerLoadError> {
    fn invalid(issuer: &str, reason: &str) -> IssuerLoadError {
        IssuerLoadError::Invalid {
            issuer: issuer.to_owned(),
            reason: reason.to_owned(),
        }
    }
    if cfg.issuer.is_empty() {
        return Err(invalid("(empty)", "issuer URL must be non-empty"));
    }
    if !is_valid_url(&cfg.issuer) {
        return Err(invalid(
            &cfg.issuer,
            "issuer must be an HTTP(S) URL ; got non-URL string",
        ));
    }
    if cfg.audience.is_empty() {
        return Err(invalid(&cfg.issuer, "audience must be non-empty"));
    }
    if cfg.jwks_url.is_empty() {
        return Err(invalid(&cfg.issuer, "jwks_url must be non-empty"));
    }
    if !is_valid_url(&cfg.jwks_url) {
        return Err(invalid(
            &cfg.issuer,
            "jwks_url must be an HTTP(S) URL ; got non-URL string",
        ));
    }
    if cfg.alg_whitelist.is_empty() {
        return Err(invalid(
            &cfg.issuer,
            "alg_whitelist must contain at least one algorithm",
        ));
    }
    for alg in &cfg.alg_whitelist {
        if !matches!(alg.as_str(), "RS256" | "ES256") {
            return Err(invalid(
                &cfg.issuer,
                &format!(
                    "alg {alg:?} not supported in v0.1.15 ; use RS256 or ES256 \
                     (HS256 stays in the legacy backend, EdDSA reserved D17+)",
                ),
            ));
        }
    }
    match cfg.tenant_strategy {
        TenantStrategy::Claim => match &cfg.tenant_claim {
            Some(c) if !c.is_empty() => {}
            _ => {
                return Err(invalid(
                    &cfg.issuer,
                    "tenant_strategy = claim requires tenant_claim to be non-empty",
                ));
            }
        },
        TenantStrategy::Issuer => {
            let tid = cfg.tenant_id_for_issuer.as_deref().unwrap_or("");
            if tid.is_empty() {
                return Err(invalid(
                    &cfg.issuer,
                    "tenant_strategy = issuer requires tenant_id_for_issuer to be non-empty",
                ));
            }
            if !is_valid_tenant_id(tid) {
                return Err(invalid(
                    &cfg.issuer,
                    "tenant_id_for_issuer must match `^[a-z0-9_-]{1,63}$`",
                ));
            }
        }
    }
    if cfg.max_token_age_s == 0 {
        return Err(invalid(
            &cfg.issuer,
            "max_token_age_s must be > 0 (typical : 3600 = 1h)",
        ));
    }
    Ok(())
}

/// **Audit fix D15-H3** — full URL validation via `url::Url::parse`.
///
/// The pre-audit version did a bare `starts_with("https://")` which
/// accepted nonsense like `"https://"` (empty host) or `"https:// "`
/// (space in host) — the `JwksCache::refresh()` call would then fail
/// with an opaque DNS error, leaving operators puzzled.
///
/// The contract :
///
///   * Only `https`, or `http` when `EFE_OIDC_ALLOW_HTTP=true` (Tier 1
///     dev / integration tests).
///   * Host must be present and non-empty.
///   * Parsing through `url::Url` rejects URLs with unreasonable shapes
///     (whitespace, missing host, invalid percent-encoding).
fn is_valid_url(s: &str) -> bool {
    let parsed = match url::Url::parse(s) {
        Ok(u) => u,
        Err(_) => return false,
    };
    let scheme = parsed.scheme();
    if parsed.host_str().map_or(true, str::is_empty) {
        return false;
    }
    match scheme {
        "https" => true,
        "http" => std::env::var("EFE_OIDC_ALLOW_HTTP")
            .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
            .unwrap_or(false),
        _ => false,
    }
}

fn is_valid_tenant_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && s.bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-'))
}

// =====================================================================
// Tests — pure unit, no HTTP, no DB.
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_json(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    fn default_params() -> LoadParams {
        LoadParams {
            jwks_ttl_s: 300,
            jwks_fetch_timeout_ms: 5000,
            jwks_retry_max: 3,
            jwks_backoff_ms: 1000,
            grace_stale_s: 3600,
        }
    }

    fn good_issuer_json() -> &'static str {
        r#"[
  {
    "issuer": "https://acme.auth0.com/",
    "audience": "efe.v1",
    "jwks_url": "https://acme.auth0.com/.well-known/jwks.json",
    "alg_whitelist": ["RS256"],
    "tenant_strategy": "claim",
    "tenant_claim": "tenant_id"
  }
]"#
    }

    #[test]
    fn load_single_issuer_happy_path() {
        let f = write_json(good_issuer_json());
        let registry = load_issuers_file(f.path(), &default_params()).unwrap();
        assert_eq!(registry.len(), 1);
        assert!(registry.get("https://acme.auth0.com/").is_some());
    }

    #[test]
    fn load_two_issuers_distinct() {
        let json = r#"[
  {
    "issuer": "https://acme.auth0.com/",
    "audience": "efe.v1",
    "jwks_url": "https://acme.auth0.com/.well-known/jwks.json",
    "alg_whitelist": ["RS256"],
    "tenant_strategy": "claim",
    "tenant_claim": "tenant_id"
  },
  {
    "issuer": "https://keycloak.globex.com/realms/efe",
    "audience": "efe.v1",
    "jwks_url": "https://keycloak.globex.com/realms/efe/protocol/openid-connect/certs",
    "alg_whitelist": ["RS256", "ES256"],
    "tenant_strategy": "issuer",
    "tenant_id_for_issuer": "globex"
  }
]"#;
        let f = write_json(json);
        let r = load_issuers_file(f.path(), &default_params()).unwrap();
        assert_eq!(r.len(), 2);
        assert!(r.get("https://acme.auth0.com/").is_some());
        assert!(r.get("https://keycloak.globex.com/realms/efe").is_some());
    }

    #[test]
    fn rejects_empty_array() {
        let f = write_json("[]");
        let err = load_issuers_file(f.path(), &default_params()).unwrap_err();
        assert!(matches!(err, IssuerLoadError::Empty { .. }));
    }

    #[test]
    fn rejects_invalid_json() {
        let f = write_json("not json");
        let err = load_issuers_file(f.path(), &default_params()).unwrap_err();
        assert!(matches!(err, IssuerLoadError::Json { .. }));
    }

    #[test]
    fn rejects_duplicate_issuers() {
        let json = format!(
            "[{0},{0}]",
            good_issuer_json().trim_matches(|c| c == '[' || c == ']')
        );
        let f = write_json(&json);
        let err = load_issuers_file(f.path(), &default_params()).unwrap_err();
        assert!(matches!(err, IssuerLoadError::Duplicate { .. }));
    }

    #[test]
    fn rejects_unknown_alg() {
        let json = r#"[
  {
    "issuer": "https://acme.auth0.com/",
    "audience": "efe.v1",
    "jwks_url": "https://acme.auth0.com/.well-known/jwks.json",
    "alg_whitelist": ["HS256"],
    "tenant_strategy": "claim",
    "tenant_claim": "tenant_id"
  }
]"#;
        let f = write_json(json);
        let err = load_issuers_file(f.path(), &default_params()).unwrap_err();
        assert!(matches!(err, IssuerLoadError::Invalid { .. }));
    }

    #[test]
    fn rejects_claim_strategy_without_tenant_claim() {
        let json = r#"[
  {
    "issuer": "https://acme.auth0.com/",
    "audience": "efe.v1",
    "jwks_url": "https://acme.auth0.com/.well-known/jwks.json",
    "alg_whitelist": ["RS256"],
    "tenant_strategy": "claim"
  }
]"#;
        let f = write_json(json);
        let err = load_issuers_file(f.path(), &default_params()).unwrap_err();
        assert!(matches!(err, IssuerLoadError::Invalid { .. }));
    }

    #[test]
    fn rejects_issuer_strategy_with_invalid_tenant_id() {
        let json = r#"[
  {
    "issuer": "https://kc.example/realms/x",
    "audience": "efe.v1",
    "jwks_url": "https://kc.example/realms/x/protocol/openid-connect/certs",
    "alg_whitelist": ["RS256"],
    "tenant_strategy": "issuer",
    "tenant_id_for_issuer": "INVALID UPPERCASE"
  }
]"#;
        let f = write_json(json);
        let err = load_issuers_file(f.path(), &default_params()).unwrap_err();
        assert!(matches!(err, IssuerLoadError::Invalid { .. }));
    }

    #[test]
    fn rejects_non_url_issuer() {
        let json = r#"[
  {
    "issuer": "not-a-url",
    "audience": "efe.v1",
    "jwks_url": "https://example.com/jwks",
    "alg_whitelist": ["RS256"],
    "tenant_strategy": "claim",
    "tenant_claim": "tenant_id"
  }
]"#;
        let f = write_json(json);
        let err = load_issuers_file(f.path(), &default_params()).unwrap_err();
        assert!(matches!(err, IssuerLoadError::Invalid { .. }));
    }

    #[test]
    fn rejects_missing_file() {
        let err = load_issuers_file(Path::new("/nonexistent/issuers.json"), &default_params())
            .unwrap_err();
        assert!(matches!(err, IssuerLoadError::Io { .. }));
    }
}
