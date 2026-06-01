//! Sprint **D37 (Φ-F)** — Mode-3 backend Prometheus instruments.
//!
//! Cf. `docs/EFE_TELEMETRY_EXTENSION_ARCHITECTURE.md` § 11.2.
//!
//! 4 instruments couvrant le fan-out k-swarm + l'agrégation :
//!
//! | # | Métrique                                  | Type      | Labels                  |
//! |--:|-------------------------------------------|-----------|-------------------------|
//! | 1 | `efe_mode3_subswarms_spawned_total`       | Counter   | mode_per_call, tenant   |
//! | 2 | `efe_mode3_dispersion`                    | Gauge     | aggregator, tenant      |
//! | 3 | `efe_mode3_k_clamped_total`               | Counter   | tenant                  |
//! | 4 | `efe_mode3_aggregation_seconds`           | Histogram | aggregator              |
//!
//! Cardinalité projetée :
//!   * `mode_per_call` ∈ {`mode1`, `mode2`} = 2
//!   * `aggregator` ∈ {`mean`, `weighted`, `median`} = 3
//!   * `tenant` ∈ 200 max
//!   → 2 × 200 + 3 × 200 + 200 + 3 × ~5 buckets = 1 215 séries
//!     (sous le budget 50k ✅).
//!
//! ## SLO contracts mesurés
//!
//!   * **C-3** Mode-3 p99 latency ≤ 3 × Mode-1 p99 — couvert par
//!     le `efe_grpc_*` existant ; `efe_mode3_aggregation_seconds`
//!     ajoute la décomposition de la phase d'agrégation.
//!   * **C-5** k_max enforced (k ≤ 5) — `efe_mode3_k_clamped_total`
//!     compte les requêtes où `k > K_MAX` a été clampé. Un taux
//!     élevé signale un mauvais usage de l'API par un client.
//!   * **C-12** Mode-3 dispersion ≤ 0.15 — `efe_mode3_dispersion`
//!     gauge. Au-dessus, le panel `d37_mode3` flag la requête
//!     pour human review.
//!
//! ## Wiring
//!
//! Tous les instruments sont émis par
//! `efe-core::Mode3Controller::execute_parallel` /
//! `execute_sequential`. L'instrumentation directe côté core
//! garantit que le compteur reflète la vérité (vs un emit côté
//! gRPC handler qui pourrait manquer les sub-swarms cancellés).

use std::sync::OnceLock;

use prometheus::{
    exponential_buckets, GaugeVec, HistogramOpts, HistogramVec, IntCounterVec,
    Opts,
};

use super::registry::global_registry;

/// Internal once-per-call macro identical to D17/D18/D29.
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

// ====================================================================
// 1. Counter — sub-swarms spawned per (mode, tenant)
// ====================================================================

once_metric!(
    /// `efe_mode3_subswarms_spawned_total{mode_per_call, tenant}` —
    /// count of Mode-3 sub-swarms spawned. Incremented by
    /// `k_effective` (post-clamp) at each `execute_parallel` /
    /// `execute_sequential` call.
    ///
    /// **Use case** : multi-replica fleet load distribution. The
    /// Grafana panel "k-swarm Throughput" plots
    /// `rate(efe_mode3_subswarms_spawned_total[1m])` per replica.
    mode3_subswarms_spawned_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_mode3_subswarms_spawned_total",
            "Total Mode-3 sub-swarms spawned per mode_per_call per tenant (D37 § 11.2 #1).",
        ),
        &["mode_per_call", "tenant"],
    )
    .expect("valid counter opts")
);

// ====================================================================
// 2. Gauge — last observed dispersion per (aggregator, tenant)
// ====================================================================

once_metric!(
    /// `efe_mode3_dispersion{aggregator, tenant}` — last observed
    /// std deviation of sub-verdict confidences (output of
    /// `Mode3Aggregator::aggregate.dispersion`).
    ///
    /// **SLO C-12** : dispersion ≤ 0.15. Au-dessus, le panel
    /// "Dispersion Distribution" du dashboard `d37_mode3` colore
    /// la cellule en rouge et l'opérateur peut flagger la
    /// requête pour human review.
    ///
    /// **Type Gauge** (vs Histogram) : on ne veut pas de
    /// distribution — la dispersion est par nature une mesure
    /// d'instabilité, et seul le "last value" matter pour le
    /// triage. Pour la distribution, utiliser
    /// `histogram_quantile` sur les sub-swarm confidences depuis
    /// `efe_grpc_request_duration_seconds`.
    mode3_dispersion,
    GaugeVec,
    GaugeVec::new(
        Opts::new(
            "efe_mode3_dispersion",
            "Last observed std deviation of sub-verdict confidences (D37 § 11.2 #2).",
        ),
        &["aggregator", "tenant"],
    )
    .expect("valid gauge opts")
);

// ====================================================================
// 3. Counter — k clamped events
// ====================================================================

once_metric!(
    /// `efe_mode3_k_clamped_total{tenant}` — count of requests
    /// where the input `k > K_MAX` was clamped server-side
    /// (`K_MAX = 5` per SLO C-5).
    ///
    /// **Use case** : détecter les clients qui demandent
    /// systématiquement k > 5 (mauvais usage de l'API). Un
    /// `rate(...) > 0.1 / tenant` justifie une discussion produit.
    ///
    /// **Wiring** : `Mode3Controller::clamp_k` retourne
    /// `(k_effective, k_clamped: bool)` ; quand `k_clamped`, le
    /// counter est bumpé.
    mode3_k_clamped_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_mode3_k_clamped_total",
            "Total Mode-3 requests where k > K_MAX was clamped (D37 § 11.2 #3).",
        ),
        &["tenant"],
    )
    .expect("valid counter opts")
);

// ====================================================================
// 4. Histogram — aggregator wall time
// ====================================================================

once_metric!(
    /// `efe_mode3_aggregation_seconds{aggregator}` — wall-clock
    /// duration of `Mode3Aggregator::aggregate` per strategy.
    ///
    /// **Buckets** : exponential 100 µs → 1 s (5 buckets). Mean +
    /// Weighted aggregateurs sont O(k) sur les sub_verdicts ;
    /// Median est O(k log k) (sort) — la queue de distribution
    /// reflètera ce coût.
    ///
    /// **SLO dérivé** : `histogram_quantile(0.99, ...) < 0.001`
    /// (1 ms) sur k=5 ; au-dessus = signal d'un aggregator
    /// pathologique.
    mode3_aggregation_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_mode3_aggregation_seconds",
            "Mode-3 aggregator wall-clock duration in seconds (D37 § 11.2 #4).",
        )
        .buckets(
            exponential_buckets(1e-4, 10.0, 5).expect("valid buckets")
        ),
        &["aggregator"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// Eager registration helper
// ====================================================================

/// Force-register every Mode-3 instrument with the global registry.
/// **MUST** be called from `Server::build` before the first gRPC
/// accept.
pub fn register_all_mode3() {
    let _ = mode3_subswarms_spawned_total();
    let _ = mode3_dispersion();
    let _ = mode3_k_clamped_total();
    let _ = mode3_aggregation_seconds();
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus::core::Collector;

    #[test]
    fn register_all_mode3_idempotent() {
        register_all_mode3();
        register_all_mode3();
    }

    #[test]
    fn subswarms_spawned_label_arity_is_two() {
        assert_eq!(
            mode3_subswarms_spawned_total().desc()[0]
                .variable_labels
                .len(),
            2
        );
    }

    #[test]
    fn dispersion_label_arity_is_two() {
        assert_eq!(
            mode3_dispersion().desc()[0].variable_labels.len(),
            2
        );
    }

    #[test]
    fn k_clamped_label_arity_is_one() {
        assert_eq!(
            mode3_k_clamped_total().desc()[0].variable_labels.len(),
            1
        );
    }

    #[test]
    fn aggregation_seconds_label_arity_is_one() {
        assert_eq!(
            mode3_aggregation_seconds().desc()[0]
                .variable_labels
                .len(),
            1
        );
    }

    #[test]
    fn aggregation_buckets_cover_microsecond_to_second() {
        let buckets = exponential_buckets(1e-4, 10.0, 5).unwrap();
        assert_eq!(buckets.len(), 5);
        assert!((buckets[0] - 1e-4).abs() < 1e-12);
        assert!((buckets[4] - 1.0).abs() < 1e-9);
    }
}
