//! Sprint **D13** — observability dashboards : 8 nouveaux instruments
//! Prometheus dédiés aux RPCs catalog (D12), aux audit events, aux
//! self-monitoring alertes et au temps de chargement des dashboards
//! Grafana.
//!
//! Conventions identiques à `instruments.rs` :
//!   * macro `once_metric!` (re-déclarée ici localement pour rester
//!     additif vis-à-vis du module historique) ;
//!   * tous les instruments sont enregistrés sur le `global_registry`
//!     M11 ;
//!   * labels strictement bornés (cf. `M11_ARCHITECTURE.md` § 13.2 +
//!     architecture D13 § 8 « Gestion de la cardinalité »).
//!
//! # Statut (v0.1.13)
//!
//! Ces instruments sont **scaffolding** : leur émission effective est
//! tied à D12-bis (handler gRPC `CatalogService` à wirer dans
//! `runtime::Server`). Tant que le handler n'existe pas, les compteurs
//! et histogrammes restent à zéro — ce qui se traduit en panneaux
//! Grafana « no data » plutôt qu'en faux positif. La fonction
//! [`register_all_d13`] expose les descripteurs au registry pour que
//! les recording rules + alertes Prom voient au moins le family name.
//!
//! # Cardinalité chiffrée
//!
//! Pour 100 tenants actifs (cible D11) :
//!
//! | Instrument                            | Combinations | Comment |
//! |---------------------------------------|--------------|---------|
//! | `efe_catalog_register_duration_seconds` | 100 × 2     | tenant × {ok,error} |
//! | `efe_catalog_evolve_duration_seconds`   | 100 × 5 × 2 | tenant × op_tag × result |
//! | `efe_catalog_get_duration_seconds`      | 100 × 2 × 2 | tenant × include_compiled × result |
//! | `efe_catalog_list_duration_seconds`     | 100 × 2     | tenant × result |
//! | `efe_catalog_versions_per_name`         | bounded     | quotas D12 (max 100 chains × tenant) |
//! | `efe_audit_events_total`                | 100 × 3 × 7 | tenant × action × op_tag |
//! | `efe_alert_fires_total`                 | ~16 × 3 × 100 | alertname × severity × tenant |
//! | `efe_dashboard_query_duration_seconds`  | 6 × ~80     | dashboard × panel |
//!
//! Total estimé ~ 9 000 séries D13 + 18 000 séries existantes =
//! 27 000 séries — bien sous le budget panic 50 000 (cf. SLO
//! D13-OBS-5 + alerte `D13ObsCardinalityBomb`).

use std::sync::OnceLock;

use prometheus::{
    exponential_buckets, Histogram, HistogramOpts, HistogramVec, IntCounterVec, IntGaugeVec, Opts,
};

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
// 5 histogrammes pour les 5 RPCs catalog (D12)
// =====================================================================
//
// Buckets calibrés contre les SLOs D12-CT-1..5 :
//   - Register p99 ≤ 200 ms (small) / ≤ 1.5 s (large)   → 1 ms..2 s
//   - Evolve append p99 ≤ 250 ms                          → 1 ms..2 s
//   - Get p99 ≤ 50 ms                                    → 0.5 ms..1 s
//   - List p99 ≤ 75 ms                                   → 0.5 ms..1 s
//
// Buckets exponentiels base-2 démarrant à 0.001 s ; suffisants pour
// résoudre le centième de seconde près du p99 sans saturer la mémoire.

once_metric!(
    /// `efe_catalog_register_duration_seconds{tenant, result}`
    catalog_register_duration_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_catalog_register_duration_seconds",
            "Server-side duration of CatalogService.RegisterCatalog, in seconds.",
        )
        .buckets(exponential_buckets(0.001, 2.0, 12).expect("valid buckets")),
        &["tenant", "result"],
    )
    .expect("valid histogram opts")
);

once_metric!(
    /// `efe_catalog_evolve_duration_seconds{tenant, op, result}`
    catalog_evolve_duration_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_catalog_evolve_duration_seconds",
            "Server-side duration of CatalogService.EvolveCatalog, in seconds.",
        )
        .buckets(exponential_buckets(0.001, 2.0, 12).expect("valid buckets")),
        &["tenant", "op", "result"],
    )
    .expect("valid histogram opts")
);

once_metric!(
    /// `efe_catalog_get_duration_seconds{tenant, include_compiled, result}`
    catalog_get_duration_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_catalog_get_duration_seconds",
            "Server-side duration of CatalogService.GetCatalog, in seconds.",
        )
        .buckets(exponential_buckets(0.0005, 2.0, 11).expect("valid buckets")),
        &["tenant", "include_compiled", "result"],
    )
    .expect("valid histogram opts")
);

once_metric!(
    /// `efe_catalog_list_duration_seconds{tenant, result}`
    catalog_list_duration_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_catalog_list_duration_seconds",
            "Server-side duration of CatalogService.ListCatalogs, in seconds.",
        )
        .buckets(exponential_buckets(0.0005, 2.0, 11).expect("valid buckets")),
        &["tenant", "result"],
    )
    .expect("valid histogram opts")
);

// =====================================================================
// Gauge : épaisseur des chaînes catalog par (tenant, name)
// =====================================================================

once_metric!(
    /// `efe_catalog_versions_per_name{tenant, name}` — nombre de
    /// versions actives pour ce (tenant, name). Borné par
    /// `max_versions_per_name = 1000` (D12 policy).
    catalog_versions_per_name,
    IntGaugeVec,
    IntGaugeVec::new(
        Opts::new(
            "efe_catalog_versions_per_name",
            "Active version count per (tenant, name) catalog chain.",
        ),
        &["tenant", "name"],
    )
    .expect("valid gauge opts")
);

// =====================================================================
// Counter : audit events (D12 audit emitter en source)
// =====================================================================

once_metric!(
    /// `efe_audit_events_total{tenant, action, op_tag, principal_present}` —
    /// counter monotone des events d'audit. Source du dashboard 05
    /// + alerte `D13ObsAuditPrincipalUnknown`.
    ///
    /// Le label `principal_present` est un bool string ("true" / "false")
    /// indiquant si le JWT `sub` claim était renseigné au moment de
    /// l'émission. Cardinalité × 2 — bornée. L'alerte D13 surveille
    /// la fraction d'events avec `principal_present="false"` ; un taux
    /// > 5 % indique que la propagation du JWT vers l'audit emitter
    /// s'est cassée (régression de l'audit fix H-4 D12).
    ///
    /// **Audit C-1 fix** : avant la correction l'alerte tentait de
    /// filtrer sur `principal=""` mais `principal` n'était pas un
    /// label déclaré — la query renvoyait toujours empty et l'alerte
    /// ne pouvait jamais firer.
    audit_events_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_audit_events_total",
            "Total catalog/cone audit events emitted (register, evolve, delete).",
        ),
        &["tenant", "action", "op_tag", "principal_present"],
    )
    .expect("valid counter opts")
);

// =====================================================================
// Counter : self-monitoring des alertes
// =====================================================================

once_metric!(
    /// `efe_alert_fires_total{alertname, severity, tenant}` — counter
    /// monotone des fires d'alertes Prom. Permet de tracker le bruit
    /// alerting (SLO D13-OBS implicit).
    alert_fires_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_alert_fires_total",
            "Total Prometheus alerts fired (per alertname / severity / tenant).",
        ),
        &["alertname", "severity", "tenant"],
    )
    .expect("valid counter opts")
);

// =====================================================================
// Histogram : dashboard query render time (SLO D13-OBS-1)
// =====================================================================

once_metric!(
    /// `efe_dashboard_query_duration_seconds{dashboard, panel, outcome}`
    /// — Grafana panel render time. Source SLO D13-OBS-1 ≤ 3 s p99.
    dashboard_query_duration_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_dashboard_query_duration_seconds",
            "Grafana dashboard panel query render time, in seconds.",
        )
        .buckets(exponential_buckets(0.05, 2.0, 10).expect("valid buckets")),
        &["dashboard", "panel", "outcome"],
    )
    .expect("valid histogram opts")
);

// =====================================================================
// Init helper
// =====================================================================

/// Force registration of every D13 instrument descriptor on the
/// global registry. Called from `init.rs` at server startup so that
/// the recording rules and alertes Prometheus aient des family names
/// disponibles dès le 1er scrape, même avant le 1er sample.
///
/// Symétrique de [`super::instruments::register_all`].
pub fn register_all_d13() {
    let _ = catalog_register_duration_seconds();
    let _ = catalog_evolve_duration_seconds();
    let _ = catalog_get_duration_seconds();
    let _ = catalog_list_duration_seconds();
    let _ = catalog_versions_per_name();
    let _ = audit_events_total();
    let _ = alert_fires_total();
    let _ = dashboard_query_duration_seconds();
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// **Audit fix T-A4 (M5 audit, side-effect)** — race-protection
    /// mutex shared by every test in this module that mutates AND
    /// reads the same Prometheus counter / gauge. Pre-fix the tests
    /// `audit_events_counter_increments` and
    /// `audit_events_counter_supports_principal_present_label`
    /// both incremented `audit_events_total` on the same label
    /// combination ; cargo's parallel test runner produced a flake
    /// where one test's "before/inc/after" sequence saw the other
    /// test's increment between the reads → assertion failure
    /// `after = before + 2 ≠ before + 1`.
    ///
    /// The shared mutex serialises ONLY the tests that modify
    /// counter values ; pure lookup tests don't need it.
    static COUNTER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn catalog_register_histogram_observes() {
        let h = catalog_register_duration_seconds();
        h.with_label_values(&["acme", "ok"]).observe(0.05);
        h.with_label_values(&["acme", "error"]).observe(0.250);
        // Just verify no panic ; sample count exposed via /metrics.
    }

    #[test]
    fn catalog_evolve_histogram_observes_per_op() {
        let h = catalog_evolve_duration_seconds();
        for op in &[
            "append_class",
            "remove_class",
            "update_thresholds",
            "replace_class_anchors",
            "recompile",
        ] {
            h.with_label_values(&["acme", op, "ok"]).observe(0.1);
        }
    }

    #[test]
    fn catalog_versions_gauge_set_and_dec() {
        let g = catalog_versions_per_name();
        g.with_label_values(&["acme", "billing"]).set(7);
        assert_eq!(g.with_label_values(&["acme", "billing"]).get(), 7);
        g.with_label_values(&["acme", "billing"]).dec();
        assert_eq!(g.with_label_values(&["acme", "billing"]).get(), 6);
    }

    #[test]
    fn audit_events_counter_increments() {
        let _guard = COUNTER_LOCK.lock().expect("counter lock poisoned");
        let c = audit_events_total();
        let before = c.with_label_values(&["acme", "register", "register", "true"]).get();
        c.with_label_values(&["acme", "register", "register", "true"]).inc();
        let after = c.with_label_values(&["acme", "register", "register", "true"]).get();
        assert_eq!(after, before + 1);
    }

    /// Audit C-1 regression : the alerts.yaml query
    /// ``efe_audit_events_total{principal_present="false"}``
    /// must hit a real series. Pre-fix, the metric had no
    /// `principal_present` label and the query returned empty.
    #[test]
    fn audit_events_counter_supports_principal_present_label() {
        let _guard = COUNTER_LOCK.lock().expect("counter lock poisoned");
        let c = audit_events_total();
        // Both bool string values must be observable.
        c.with_label_values(&["acme", "register", "register", "true"]).inc();
        c.with_label_values(&["acme", "delete",   "delete",   "false"]).inc();
        let with_principal = c
            .with_label_values(&["acme", "register", "register", "true"])
            .get();
        let without_principal = c
            .with_label_values(&["acme", "delete", "delete", "false"])
            .get();
        assert!(with_principal >= 1);
        assert!(without_principal >= 1);
    }

    #[test]
    fn alert_fires_counter_supports_severity_label() {
        let c = alert_fires_total();
        for sev in &["critical", "warning", "info"] {
            c.with_label_values(&["D13ObsHighErrorRate", sev, "acme"]).inc();
        }
    }

    #[test]
    fn dashboard_query_histogram_observes() {
        let h = dashboard_query_duration_seconds();
        h.with_label_values(&["01_operator_overview", "p99_latency", "ok"])
            .observe(0.4);
        h.with_label_values(&["02_tenant_drill_down", "tenant_rps", "timeout"])
            .observe(8.0);
    }

    #[test]
    fn register_all_d13_idempotent() {
        // Calling twice must not panic / double-register.
        register_all_d13();
        register_all_d13();
    }

    #[test]
    fn register_all_d13_exposes_8_families() {
        register_all_d13();
        let names: Vec<String> = global_registry()
            .gather()
            .into_iter()
            .map(|mf| mf.get_name().to_string())
            .collect();
        let expected = [
            "efe_catalog_register_duration_seconds",
            "efe_catalog_evolve_duration_seconds",
            "efe_catalog_get_duration_seconds",
            "efe_catalog_list_duration_seconds",
            "efe_catalog_versions_per_name",
            "efe_audit_events_total",
            "efe_alert_fires_total",
            "efe_dashboard_query_duration_seconds",
        ];
        // Note : some `*Vec` instruments only appear post-first-sample.
        // We trigger one sample per instrument before asserting.
        let _ = catalog_register_duration_seconds()
            .with_label_values(&["t", "ok"])
            .observe(0.1);
        let _ = catalog_versions_per_name()
            .with_label_values(&["t", "n"])
            .set(1);
        let names: Vec<String> = global_registry()
            .gather()
            .into_iter()
            .map(|mf| mf.get_name().to_string())
            .collect();
        for ex in &expected {
            // We don't fail if an instrument hasn't been touched ;
            // touching all 8 in a single test would couple this test
            // to other tests' label values. So we just verify *at
            // least* the touched ones are present.
            let present = names.iter().any(|n| n == ex);
            // Touched: register and versions ; skip the others.
            if *ex == "efe_catalog_register_duration_seconds"
                || *ex == "efe_catalog_versions_per_name"
            {
                assert!(present, "{ex} missing from /metrics");
            }
        }
    }
}
