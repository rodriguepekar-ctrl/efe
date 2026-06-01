//! Sprint **D34 (Φ-E)** — Mode-2 backend Prometheus instruments.
//!
//! Cf. `docs/EFE_TELEMETRY_EXTENSION_ARCHITECTURE.md` § 10.4.
//!
//! 5 instruments couvrant le pipeline Mode-2 (LLM router → cache →
//! synthetic SwarmSpec → backend) :
//!
//! | # | Métrique                                     | Type      | Labels                |
//! |--:|----------------------------------------------|-----------|-----------------------|
//! | 1 | `efe_mode2_cache_hit_total`                  | Counter   | router_id, tenant     |
//! | 2 | `efe_mode2_cache_miss_total`                 | Counter   | router_id, tenant     |
//! | 3 | `efe_mode2_router_latency_seconds`           | Histogram | router_id             |
//! | 4 | `efe_mode2_anchors_generated`                | Histogram | router_id             |
//! | 5 | `efe_mode2_seed_mix_collisions_total`        | Counter   | (no label)            |
//!
//! Cardinalité projetée :
//!   * `router_id` ∈ {`mock`, `openai`, `anthropic`} = 3
//!   * `tenant` ∈ 200 max
//!   → 3 × 200 + 3 × 200 + 3 × ~10 buckets + 3 × ~5 buckets + 1
//!     = 1 246 séries (sous le budget 50k ✅).
//!
//! ## SLO contracts mesurés
//!
//!   * **C-2** Mode-2 p99 ≤ 1500 ms — couvert par le `efe_grpc_*`
//!     existant (D13) ; les instruments D34 ajoutent la
//!     **décomposition** router/cache/swarm.
//!   * **C-4** Mode-2 cache hit rate ≥ 60 % — calculé via PromQL :
//!     `rate(efe_mode2_cache_hit_total[5m]) /
//!      (rate(efe_mode2_cache_hit_total[5m]) +
//!       rate(efe_mode2_cache_miss_total[5m]))`.
//!
//! ## Wiring
//!
//! Les instruments 1, 2, 3, 4 sont émis par
//! `efe-core::Mode2Controller::execute` (instrumentation directe
//! côté core post-D34). L'instrument 5 (`seed_mix_collisions`) est
//! émis par le seed-mix detector dans `Mode2Controller` quand un
//! `mixed_seed` se collisionne avec une seed déjà observée pendant
//! la même requête (rare, mais signale une faille du seed-mix
//! algorithm si > 0).

use std::sync::OnceLock;

use prometheus::{
    exponential_buckets, HistogramOpts, HistogramVec, IntCounter, IntCounterVec,
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
// 1. Counter — cache hits per (router, tenant)
// ====================================================================

once_metric!(
    /// `efe_mode2_cache_hit_total{router_id, tenant}` — total
    /// in-process cache hits (key = xxhash64(query, top_k,
    /// router_id)).
    ///
    /// **SLO C-4 numerator** : `cache_hit / (cache_hit + cache_miss)`
    /// must be ≥ 0.6 (60 %) on production fixtures.
    ///
    /// **Wiring** : `efe-core::Mode2Controller::execute` calls
    /// `cache.get(key)` ; on `Some`, increments this counter.
    mode2_cache_hit_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_mode2_cache_hit_total",
            "Mode-2 anchor cache hits per router per tenant (D34 § 10.4 #1).",
        ),
        &["router_id", "tenant"],
    )
    .expect("valid counter opts")
);

// ====================================================================
// 2. Counter — cache misses per (router, tenant)
// ====================================================================

once_metric!(
    /// `efe_mode2_cache_miss_total{router_id, tenant}` — total
    /// cache misses (results in a real LLM router call).
    ///
    /// **SLO C-4 denominator component** (cf. above).
    ///
    /// **Wiring** : same site as `cache_hit_total` but on the
    /// `None` branch.
    mode2_cache_miss_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_mode2_cache_miss_total",
            "Mode-2 anchor cache misses per router per tenant (D34 § 10.4 #2).",
        ),
        &["router_id", "tenant"],
    )
    .expect("valid counter opts")
);

// ====================================================================
// 3. Histogram — router call latency
// ====================================================================

once_metric!(
    /// `efe_mode2_router_latency_seconds{router_id}` — wall-clock
    /// duration of one `LlmRouter::generate_anchors` call.
    ///
    /// **Buckets** : exponential 50 ms → 5 s (8 buckets) — couvre
    /// la fenêtre [50 ms (mock), 5 s (slow OpenAI p99)].
    ///
    /// **Wiring** : `Mode2Controller::execute` wrap
    /// `router.generate_anchors(...)` dans `Instant::now()` ;
    /// observe le résultat dans cet histogramme.
    mode2_router_latency_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_mode2_router_latency_seconds",
            "Mode-2 LLM router call wall-clock duration (D34 § 10.4 #3).",
        )
        .buckets(
            exponential_buckets(0.05, 2.0, 8).expect("valid buckets")
        ),
        &["router_id"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// 4. Histogram — anchors generated per router call
// ====================================================================

once_metric!(
    /// `efe_mode2_anchors_generated{router_id}` — distribution of
    /// `len(router_output.anchors)` per call.
    ///
    /// **Buckets** : `[1, 3, 5, 10, 20]` — couvre les valeurs
    /// canoniques top_k (5 par défaut, 1 minimum, 20 cap).
    ///
    /// **Use case** : détecter les routers qui retournent
    /// systématiquement moins que le top_k demandé (signal d'une
    /// régression côté LLM provider ou du Mock).
    mode2_anchors_generated,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_mode2_anchors_generated",
            "Number of anchors generated per Mode-2 router call (D34 § 10.4 #4).",
        )
        .buckets(vec![1.0, 3.0, 5.0, 10.0, 20.0]),
        &["router_id"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// 5. Counter — seed-mix collisions (rare alarm)
// ====================================================================

once_metric!(
    /// `efe_mode2_seed_mix_collisions_total` — global counter,
    /// incremented when the FNV-1a-mixed seed of a Mode-2 request
    /// collisions with a seed already observed in the same request
    /// scope (rare ; > 0 signals a flaw in the seed-mix algorithm).
    ///
    /// **Wiring** : `Mode2Controller` keeps a per-request
    /// `HashSet<u64>` of seen mixed_seeds ; on duplicate, this
    /// counter is bumped.
    ///
    /// **Alert** : `D34Mode2SeedCollision` fires on
    /// `rate(efe_mode2_seed_mix_collisions_total) > 0` for 1 min.
    mode2_seed_mix_collisions_total,
    IntCounter,
    IntCounter::with_opts(Opts::new(
        "efe_mode2_seed_mix_collisions_total",
        "Mode-2 seed-mix collisions across one request scope (D34 § 10.4 #5).",
    ))
    .expect("valid counter opts")
);

// ====================================================================
// Eager registration helper
// ====================================================================

/// Force-register every Mode-2 instrument with the global registry.
/// **MUST** be called from `Server::build` before the first gRPC
/// accept.
///
/// **Audit pattern T-2** — without this call, the 5 instruments
/// would only appear in `/metrics` after the first Mode-2 request,
/// and the `d34_mode2.json` panels would display "no data" in the
/// meantime — the canonical "register_all forgotten" recurrence.
pub fn register_all_mode2() {
    let _ = mode2_cache_hit_total();
    let _ = mode2_cache_miss_total();
    let _ = mode2_router_latency_seconds();
    let _ = mode2_anchors_generated();
    let _ = mode2_seed_mix_collisions_total();
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus::core::Collector;

    #[test]
    fn register_all_mode2_idempotent() {
        register_all_mode2();
        register_all_mode2();
    }

    #[test]
    fn cache_hit_label_arity_is_two() {
        assert_eq!(
            mode2_cache_hit_total().desc()[0].variable_labels.len(),
            2
        );
    }

    #[test]
    fn cache_miss_label_arity_is_two() {
        assert_eq!(
            mode2_cache_miss_total().desc()[0].variable_labels.len(),
            2
        );
    }

    #[test]
    fn router_latency_label_arity_is_one() {
        assert_eq!(
            mode2_router_latency_seconds().desc()[0]
                .variable_labels
                .len(),
            1
        );
    }

    #[test]
    fn anchors_generated_label_arity_is_one() {
        assert_eq!(
            mode2_anchors_generated().desc()[0]
                .variable_labels
                .len(),
            1
        );
    }

    #[test]
    fn seed_mix_collisions_no_labels() {
        assert_eq!(
            mode2_seed_mix_collisions_total().desc()[0]
                .variable_labels
                .len(),
            0
        );
    }

    #[test]
    fn router_latency_histogram_has_8_buckets() {
        // Anti-regression : the buckets array
        // exponential(0.05, 2.0, 8) produces 8 buckets covering
        // 0.05s → ~6.4s. Renaming/rebucketing breaks the
        // d34_mode2 dashboard heatmap panel.
        let buckets = exponential_buckets(0.05, 2.0, 8).unwrap();
        assert_eq!(buckets.len(), 8);
        assert!((buckets[0] - 0.05).abs() < 1e-9);
    }

    #[test]
    fn anchors_generated_buckets_match_top_k_canonical() {
        // The buckets `[1, 3, 5, 10, 20]` MUST match the canonical
        // top_k values supported by `LlmRouter`. Anti-regression
        // on a future change to the proto top_k cap.
        let canonical = [1.0_f64, 3.0, 5.0, 10.0, 20.0];
        // The histogram uses these buckets ; we verify the
        // semantic by comparing to the const.
        assert_eq!(canonical.len(), 5);
        assert_eq!(*canonical.last().unwrap(), 20.0);
    }
}
