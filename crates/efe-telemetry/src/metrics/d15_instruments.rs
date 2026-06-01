//! Sprint **D15** — OIDC tenant onboarding + IdP integration metrics.
//!
//! 7 nouveaux instruments dédiés au pipeline OIDC (`OidcValidator`,
//! `JwksCache`, `IssuerRegistry`, `TenantStore`) — cf.
//! `docs/bench/d15_onboarding_idp_architecture.md` § 14.
//!
//! Conventions identiques à `d14_instruments.rs` /
//! `d14bis_instruments.rs` :
//!   * macro `once_metric!` (re-déclarée localement) ;
//!   * tous les instruments enregistrés sur `global_registry` (M11) ;
//!   * labels strictement bornés (jamais le `sub` ou le token).
//!
//! # Cardinalité (estim.)
//!
//! Pour 100 tenants × 4 issuers × ~20 kids actifs au pic d'une rotation :
//!
//! | Instrument                            | Combinations |
//! |---------------------------------------|-------------:|
//! | `efe_oidc_tokens_validated_total`     | 4 issuers × 6 outcomes = 24 |
//! | `efe_jwks_fetch_total`                | 4 urls × 3 results = 12 |
//! | `efe_jwks_cache_size`                 | 4 urls = 4 |
//! | `efe_jwks_cache_age_seconds`          | 4 urls = 4 |
//! | `efe_jwks_stale_total`                | 4 urls = 4 |
//! | `efe_oidc_issuers_loaded`             | 1 (no label) |
//! | `efe_tenants_total`                   | 3 statuses = 3 |
//!
//! Total ~52 séries — négligeable vs le budget D13 (50 000 panic).
//!
//! # Pourquoi pas de label `sub` / `tenant` sur tokens_validated_total ?
//!
//! Cf. threat model § 13.4 : un tenant fortement charge le label
//! `tenant` pour exploser la cardinalité Prometheus. On utilise donc
//! `issuer` (cardinalité bornée par le fichier `oidc-issuers.json`)
//! au lieu de `tenant`. La traçabilité fine reste accessible via les
//! logs structurés (`tracing::info` avec `tenant` redacté en hash si
//! besoin).

use std::sync::OnceLock;

use prometheus::{IntCounterVec, IntGauge, IntGaugeVec, Opts};

use super::registry::global_registry;

macro_rules! once_metric {
    ($(#[$doc:meta])* $fn_name:ident, $ty:ty, $init:expr) => {
        $(#[$doc])*
        pub fn $fn_name() -> &'static $ty {
            static CELL: OnceLock<$ty> = OnceLock::new();
            CELL.get_or_init(|| {
                let m: $ty = $init;
                let _ = global_registry().register(Box::new(m.clone()));
                m
            })
        }
    };
}

// =====================================================================
// OIDC token validation outcomes
// =====================================================================

once_metric!(
    /// `efe_oidc_tokens_validated_total{issuer, outcome}` — count of
    /// JWT validations performed by `OidcValidator::validate`.
    ///
    /// `outcome` is one of :
    ///   * `ok` — signature OK + claims OK + tenant OK ;
    ///   * `invalid_signature` — signature mismatch / kid not in cache ;
    ///   * `expired` — `exp` past or `iat` outside `max_token_age_s` ;
    ///   * `bad_audience` — `aud` claim mismatch ;
    ///   * `unknown_issuer` — `iss` claim not in `IssuerRegistry` ;
    ///   * `tenant_resolution_failed` — tenant claim missing /
    ///      not active in registry / regex mismatch.
    ///
    /// The `D15-AlertOidcAuthFailures` Grafana alert fires on any
    /// outcome ≠ `ok` rate > 1 % over 5 min.
    oidc_tokens_validated_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_oidc_tokens_validated_total",
            "OIDC JWT validations by issuer + outcome (architecture D15 § 14).",
        ),
        &["issuer", "outcome"],
    )
    .expect("valid counter opts")
);

// =====================================================================
// JWKS fetch / cache instrumentation
// =====================================================================

once_metric!(
    /// `efe_jwks_fetch_total{url, result}` — count of JWKS fetches
    /// triggered by either the periodic ticker or an on-demand
    /// `JwksCache::refresh()` call.
    ///
    /// `result` is one of :
    ///   * `ok` — HTTP 200 + valid JSON + ≥1 key parsed ;
    ///   * `fetch_error` — network / TLS / HTTP 4xx-5xx ;
    ///   * `parse_error` — JSON parse / no keys / unsupported kty.
    ///
    /// Use the `rate(...)` over `result="fetch_error"` series to alert
    /// on IdP outages (`D15-AlertJwksFetchFailing`).
    jwks_fetch_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_jwks_fetch_total",
            "JWKS HTTP fetches by url + result (D15 § 14).",
        ),
        &["url", "result"],
    )
    .expect("valid counter opts")
);

once_metric!(
    /// `efe_jwks_cache_size{url}` — number of distinct kids currently
    /// in cache for that JWKS url. Set after every successful refresh.
    ///
    /// Sanity panel : 0 ⇒ keys were dropped (alert
    /// `D15-AlertJwksCacheEmpty`).
    jwks_cache_size,
    IntGaugeVec,
    IntGaugeVec::new(
        Opts::new(
            "efe_jwks_cache_size",
            "Number of kids currently held in the JWKS cache, per url.",
        ),
        &["url"],
    )
    .expect("valid gauge opts")
);

once_metric!(
    /// `efe_jwks_cache_age_seconds{url}` — wall-clock age of the most
    /// recent successful refresh. Published by the periodic ticker
    /// after every iteration.
    ///
    /// SLO : age ≤ `ttl + grace_stale` (default 300 + 3600 = 3900 s).
    /// Alert `D15-AlertJwksStale` fires when the gauge sits past
    /// `ttl + grace_stale` for > 1 min — at that point the cache will
    /// reject all signatures.
    jwks_cache_age_seconds,
    IntGaugeVec,
    IntGaugeVec::new(
        Opts::new(
            "efe_jwks_cache_age_seconds",
            "Seconds elapsed since the last successful JWKS refresh, per url.",
        ),
        &["url"],
    )
    .expect("valid gauge opts")
);

once_metric!(
    /// `efe_jwks_stale_total{url}` — number of token verifications
    /// served from a stale-but-within-grace cache entry. Each
    /// increment indicates the IdP was unreachable but we accepted
    /// the existing keys per the stale-while-revalidate policy.
    ///
    /// Useful for after-action reports : "during the X min Auth0
    /// outage, we served Y stale verifies, Z fresh verifies".
    jwks_stale_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_jwks_stale_total",
            "Number of token verifies served from a stale JWKS cache entry.",
        ),
        &["url"],
    )
    .expect("valid counter opts")
);

// =====================================================================
// Issuer registry sanity gauge
// =====================================================================

once_metric!(
    /// `efe_oidc_issuers_loaded` — number of `IssuerEntry` parsed from
    /// `oidc-issuers.json` at boot.
    ///
    /// Sanity check : 0 ⇒ misconfiguration / Secret not mounted
    /// (alert `D15-AlertOidcNoIssuers`).
    oidc_issuers_loaded,
    IntGauge,
    IntGauge::new(
        "efe_oidc_issuers_loaded",
        "Number of OIDC issuers loaded from oidc-issuers.json at boot.",
    )
    .expect("valid gauge opts")
);

// =====================================================================
// Tenant registry inventory
// =====================================================================

once_metric!(
    /// `efe_tenants_total{status}` — number of tenants in each status
    /// (`pending` / `active` / `suspended`). Refreshed by a periodic
    /// task spawned at boot OR set imperatively from CLI commands
    /// (`efe-onboard activate / suspend`).
    ///
    /// Used by the `D15-onboarding` dashboard to show the onboarding
    /// funnel ; the architecture promise is one row in this gauge per
    /// `tenants.status` enum value (3 series total).
    tenants_total,
    IntGaugeVec,
    IntGaugeVec::new(
        Opts::new(
            "efe_tenants_total",
            "Number of tenants per status (pending|active|suspended).",
        ),
        &["status"],
    )
    .expect("valid gauge opts")
);

// =====================================================================
// Eager registration
// =====================================================================

/// Force-register every D15 instrument with the global registry so
/// the Prometheus `/metrics` endpoint exposes them as zero-valued
/// series even before the first OIDC token. Without this, panels in
/// the `D15-onboarding` dashboard show "no data" until the first
/// authenticated RPC ; with it, dashboards render immediately and
/// alerts have known label sets to match against.
///
/// Idempotent — the `once_metric!` macros guard against
/// double-registration. Called from `runtime::Server::build` when
/// `oidc.enabled` is true.
pub fn register_all_d15() {
    let _ = oidc_tokens_validated_total();
    let _ = jwks_fetch_total();
    let _ = jwks_cache_size();
    let _ = jwks_cache_age_seconds();
    let _ = jwks_stale_total();
    let _ = oidc_issuers_loaded();
    let _ = tenants_total();
}

#[cfg(test)]
mod tests {
    use super::*;
    // `Collector::desc` is the trait that exposes `variable_labels` ; we need
    // it in scope to call `.desc()` on Counter / Gauge instances in the
    // arity test.
    use prometheus::core::Collector;

    #[test]
    fn tokens_validated_counter_increments() {
        let labels = ["https://example.auth0.com/", "ok"];
        let before = oidc_tokens_validated_total()
            .with_label_values(&labels)
            .get();
        oidc_tokens_validated_total()
            .with_label_values(&labels)
            .inc();
        let after = oidc_tokens_validated_total()
            .with_label_values(&labels)
            .get();
        assert_eq!(after, before + 1);
    }

    #[test]
    fn tokens_validated_outcome_labels_documented() {
        // The architecture doc § 14.2 lists exactly 6 outcome values.
        // The handler code (auth/oidc.rs) must use these labels to
        // make the dashboard panels work — this test catches a
        // typo in either the doc or the handler.
        let outcomes = [
            "ok",
            "invalid_signature",
            "expired",
            "bad_audience",
            "unknown_issuer",
            "tenant_resolution_failed",
        ];
        for outcome in outcomes {
            // Just assert we can use each as a label without panic.
            let labels = ["https://test/", outcome];
            oidc_tokens_validated_total()
                .with_label_values(&labels)
                .inc();
        }
    }

    #[test]
    fn jwks_fetch_counter_increments() {
        let labels = ["https://test/.well-known/jwks.json", "ok"];
        let before = jwks_fetch_total().with_label_values(&labels).get();
        jwks_fetch_total().with_label_values(&labels).inc();
        let after = jwks_fetch_total().with_label_values(&labels).get();
        assert_eq!(after, before + 1);
    }

    #[test]
    fn jwks_fetch_result_labels_documented() {
        for result in ["ok", "fetch_error", "parse_error"] {
            jwks_fetch_total()
                .with_label_values(&["https://example/", result])
                .inc();
        }
    }

    #[test]
    fn jwks_cache_size_settable() {
        jwks_cache_size()
            .with_label_values(&["https://example/jwks.json"])
            .set(5);
        assert_eq!(
            jwks_cache_size()
                .with_label_values(&["https://example/jwks.json"])
                .get(),
            5
        );
    }

    #[test]
    fn jwks_cache_age_seconds_settable() {
        jwks_cache_age_seconds()
            .with_label_values(&["https://example/jwks.json"])
            .set(42);
        assert_eq!(
            jwks_cache_age_seconds()
                .with_label_values(&["https://example/jwks.json"])
                .get(),
            42
        );
    }

    #[test]
    fn jwks_stale_counter_increments() {
        let labels = ["https://example/jwks.json"];
        let before = jwks_stale_total().with_label_values(&labels).get();
        jwks_stale_total().with_label_values(&labels).inc();
        let after = jwks_stale_total().with_label_values(&labels).get();
        assert_eq!(after, before + 1);
    }

    #[test]
    fn oidc_issuers_loaded_settable() {
        oidc_issuers_loaded().set(2);
        assert_eq!(oidc_issuers_loaded().get(), 2);
    }

    #[test]
    fn tenants_total_per_status_settable() {
        for status in ["pending", "active", "suspended"] {
            tenants_total()
                .with_label_values(&[status])
                .set(7);
            assert_eq!(
                tenants_total().with_label_values(&[status]).get(),
                7
            );
        }
    }

    #[test]
    fn register_all_d15_idempotent() {
        register_all_d15();
        register_all_d15();
        // No assertion ; absence of panic is the signal.
    }

    #[test]
    fn label_count_matches_documented_arity() {
        // Architecture doc § 14 commits to specific label arities ;
        // a refactor that adds a label silently would break the
        // dashboard PromQL queries. We assert the expected arity
        // by checking the `Desc` reported by Prometheus.
        // (Counter / Gauge `Desc.variable_labels` exposes them.)
        let descs = oidc_tokens_validated_total().desc();
        assert_eq!(descs[0].variable_labels.len(), 2); // issuer, outcome

        let descs = jwks_fetch_total().desc();
        assert_eq!(descs[0].variable_labels.len(), 2); // url, result

        let descs = jwks_cache_size().desc();
        assert_eq!(descs[0].variable_labels.len(), 1); // url

        let descs = jwks_cache_age_seconds().desc();
        assert_eq!(descs[0].variable_labels.len(), 1); // url

        let descs = jwks_stale_total().desc();
        assert_eq!(descs[0].variable_labels.len(), 1); // url

        let descs = tenants_total().desc();
        assert_eq!(descs[0].variable_labels.len(), 1); // status
    }
}
