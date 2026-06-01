//! Sprint **Φ-C-FFI Completion (D26)** — PyO3 FFI Prometheus instruments.
//!
//! Cf. `docs/EFE_PYO3_FFI_COMPLETION_ARCHITECTURE.md` § 12.
//!
//! 8 instruments couvrant l'in-process Python FFI :
//!
//! | # | Métrique                              | Type      | Labels                  |
//! |--:|---------------------------------------|-----------|-------------------------|
//! | 1 | `efe_ffi_classify_total`              | Counter   | endpoint, outcome       |
//! | 2 | `efe_ffi_classify_seconds`            | Histogram | endpoint                |
//! | 3 | `efe_ffi_gil_held_seconds`            | Histogram | op                      |
//! | 4 | `efe_ffi_gil_released_seconds`        | Histogram | op                      |
//! | 5 | `efe_ffi_marshal_in_seconds`          | Histogram | kind                    |
//! | 6 | `efe_ffi_marshal_out_seconds`         | Histogram | kind                    |
//! | 7 | `efe_ffi_bundle_cache_hits_total`     | Counter   | outcome                 |
//! | 8 | `efe_ffi_wheel_load_seconds`          | Histogram | (no label)              |
//!
//! Cardinalité projetée :
//!   * `endpoint` ∈ {sync, async} = 2
//!   * `outcome` ∈ {ok, err, hit, miss, evict} ≤ 5
//!   * `op` ∈ {classify, aclassify, load_bundle} ≤ 4
//!   * `kind` ∈ {str, bytes, array, verdict, els} ≤ 5
//!   → ≤ 2 × 2 + 2 + 2 × 4 + 2 × 5 + 2 × 5 + 5 + 1 = ~ 40 séries
//!     (largement sous le budget D13 50 000 séries ✅).
//!
//! ## Production callers (D-FFI-2 enforcement)
//!
//! Chaque instrument déclaré ci-dessous DOIT avoir au moins un
//! appelant production documenté hors `#[cfg(test)]`. Le mapping est
//! enforced par le pin test
//! `scripts/tests/test_audit_ffi_metrics_have_callers.py`.
//!
//! | Instrument                          | Callers production            |
//! |-------------------------------------|--------------------------------|
//! | `efe_ffi_classify_total`            | `efe-ffi::client::classify`    |
//! | `efe_ffi_classify_seconds`          | `efe-ffi::client::classify`    |
//! | `efe_ffi_gil_held_seconds`          | `efe-ffi::controller::execute` |
//! | `efe_ffi_gil_released_seconds`      | `efe-ffi::controller::execute` |
//! | `efe_ffi_marshal_in_seconds`        | `efe-ffi::classify`            |
//! | `efe_ffi_marshal_out_seconds`       | `efe-ffi::verdict::from_synthetic` |
//! | `efe_ffi_bundle_cache_hits_total`   | `efe-ffi::cache::{get,evict}`  |
//! | `efe_ffi_wheel_load_seconds`        | `efe-ffi::lib::efe_native init`|
//!
//! Sans ces appelants, les panels FFI du dashboard Grafana
//! `efe-ffi.json` afficheraient "no data" — exactement le pattern
//! Φ-D.B1/B2 que nous avons éradiqué côté GPU.

use std::sync::OnceLock;

use prometheus::{
    exponential_buckets, HistogramOpts, HistogramVec, IntCounterVec, Opts,
};

use super::registry::global_registry;

/// Internal once-per-call macro — pattern identique à `gpu_instruments`.
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
// 1. Counter — total classify calls (per endpoint × outcome)
// ====================================================================

once_metric!(
    /// `efe_ffi_classify_total{endpoint,outcome}` — total number of
    /// `efe.classify_zero_copy` / `efe.aclassify_zero_copy` calls.
    ///
    /// `endpoint` ∈ {sync, async}.
    /// `outcome`  ∈ {ok, err}.
    ///
    /// **Production caller** : incremented in `efe-ffi::client::classify`
    /// after `controller.execute` returns (success → ok, PyErr → err).
    ffi_classify_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_ffi_classify_total",
            "Total efe-ffi classify calls (Φ-C-FFI § 12.1 #1).",
        ),
        &["endpoint", "outcome"],
    )
    .expect("valid counter opts")
);

// ====================================================================
// 2. Histogram — end-to-end classify latency
// ====================================================================

once_metric!(
    /// `efe_ffi_classify_seconds{endpoint}` — wall-clock duration of
    /// one `classify` call (from Python entry to Python return).
    ///
    /// Buckets : 100 µs → 10 s (10 buckets exponentielles ×4) ; couvre
    /// le D25 stub (≈ 30 ms) et le D29 réel (≈ 5–100 ms).
    ///
    /// **Production caller** : recorded in `efe-ffi::client::classify`
    /// at scope-end (= `let elapsed = started.elapsed()`).
    ffi_classify_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_ffi_classify_seconds",
            "End-to-end classify latency (Φ-C-FFI § 12.1 #2).",
        )
        .buckets(
            exponential_buckets(1e-4, 4.0, 10).expect("valid buckets")
        ),
        &["endpoint"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// 3. Histogram — time the GIL was held during one op
// ====================================================================

once_metric!(
    /// `efe_ffi_gil_held_seconds{op}` — fraction of an op's total
    /// time during which the Python GIL was held by efe-ffi (= NOT
    /// inside `py.allow_threads`).
    ///
    /// `op` ∈ {classify, aclassify, load_bundle, marshal}.
    ///
    /// SLO `SLO-FFI-GIL-RELEASED` : ratio (gil_released / total) ≥ 0.95.
    ///
    /// **Production caller** : `efe-ffi::controller::execute` wraps
    /// the `py.allow_threads` block, measures both held and released
    /// fractions.
    ffi_gil_held_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_ffi_gil_held_seconds",
            "Seconds the GIL was held during one efe-ffi op (Φ-C-FFI § 12.1 #3).",
        )
        .buckets(
            exponential_buckets(1e-5, 4.0, 10).expect("valid buckets")
        ),
        &["op"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// 4. Histogram — time the GIL was released during one op
// ====================================================================

once_metric!(
    /// `efe_ffi_gil_released_seconds{op}` — fraction of an op's total
    /// time during which the Python GIL was released (via
    /// `py.allow_threads`), making the op cooperative with other
    /// Python threads.
    ///
    /// **Production caller** : same scope as `ffi_gil_held_seconds`.
    ffi_gil_released_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_ffi_gil_released_seconds",
            "Seconds the GIL was released during one efe-ffi op (Φ-C-FFI § 12.1 #4).",
        )
        .buckets(
            exponential_buckets(1e-5, 4.0, 10).expect("valid buckets")
        ),
        &["op"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// 5. Histogram — marshal-in (Python → Rust conversion) latency
// ====================================================================

once_metric!(
    /// `efe_ffi_marshal_in_seconds{kind}` — time spent converting a
    /// Python argument to a Rust-side type (= str → &str, numpy
    /// array → &[f32], etc.).
    ///
    /// `kind` ∈ {str, bytes, array, dict}.
    ///
    /// **Production caller** : `efe-ffi::classify::classify_zero_copy`
    /// after tokenize+embedding marshal.
    ffi_marshal_in_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_ffi_marshal_in_seconds",
            "Python → Rust marshal latency (Φ-C-FFI § 12.1 #5).",
        )
        .buckets(
            exponential_buckets(1e-6, 4.0, 10).expect("valid buckets")
        ),
        &["kind"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// 6. Histogram — marshal-out (Rust → Python conversion) latency
// ====================================================================

once_metric!(
    /// `efe_ffi_marshal_out_seconds{kind}` — time spent converting
    /// a Rust return type to a Python object.
    ///
    /// `kind` ∈ {verdict, els, bundle, dict}.
    ///
    /// **Production caller** : `efe-ffi::verdict::from_synthetic`
    /// wrap (= measure construction time of the `PyVerdict` Python
    /// object).
    ffi_marshal_out_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_ffi_marshal_out_seconds",
            "Rust → Python marshal latency (Φ-C-FFI § 12.1 #6).",
        )
        .buckets(
            exponential_buckets(1e-6, 4.0, 10).expect("valid buckets")
        ),
        &["kind"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// 7. Counter — LRU bundle cache outcomes
// ====================================================================

once_metric!(
    /// `efe_ffi_bundle_cache_hits_total{outcome}` — counts LRU bundle
    /// cache outcomes per call.
    ///
    /// `outcome` ∈ {hit, miss, evict}.
    ///
    /// **Production caller** : `efe-ffi::cache::BundleCache::get`
    /// (hit/miss) + `BundleCache::evict_lru` (evict).
    ffi_bundle_cache_hits_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_ffi_bundle_cache_hits_total",
            "LRU bundle cache outcomes (Φ-C-FFI § 12.1 #7).",
        ),
        &["outcome"],
    )
    .expect("valid counter opts")
);

// ====================================================================
// 8. Histogram — wheel cold load latency (boot-time, single shot)
// ====================================================================

once_metric!(
    /// `efe_ffi_wheel_load_seconds` — single-shot observation of the
    /// time it took to `import efe` (= the Rust `#[pymodule]` init
    /// function `efe_native()`).
    ///
    /// SLO `SLO-FFI-COLD-START` : ≤ 100 ms p95.
    ///
    /// **Production caller** : `efe-ffi::lib::efe_native` measures
    /// the wall-clock duration of its own body and records once.
    ///
    /// Note : `HistogramVec` even without labels — keeps the API
    /// symmetric with the other 7 instruments and lets us add a
    /// `python_version` label later without an ABI break.
    ffi_wheel_load_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_ffi_wheel_load_seconds",
            "Wheel cold load latency (boot-time) (Φ-C-FFI § 12.1 #8).",
        )
        .buckets(
            // 1 ms → 10 s : couvre l'import léger (≈ 30 ms) et le
            // pire cas dégradé (≈ 1 s sur disque cold).
            exponential_buckets(1e-3, 4.0, 8).expect("valid buckets")
        ),
        &["python_version"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// Force-register everything at server boot.
// ====================================================================

/// Force-register every FFI instrument with the global registry.
/// Idempotent (=`OnceLock` guards inside each `once_metric!`).
///
/// **Production caller** : `efe-server::runtime` calls this when the
/// `EFE_FFI_OBSERVABILITY=true` env is set (= co-located server
/// embedding the FFI substrate). Without the call, the Prometheus
/// `/metrics` endpoint would show "no data" for the FFI series until
/// the first classify lands → confuses Grafana baseline panels.
///
/// **D-FFI-2 enforcement** : the pin test
/// `test_audit_ffi_metrics_have_callers.py` runs `grep -r` for each
/// instrument helper and fails if no production caller is found.
pub fn register_all_ffi() {
    let _ = ffi_classify_total();
    let _ = ffi_classify_seconds();
    let _ = ffi_gil_held_seconds();
    let _ = ffi_gil_released_seconds();
    let _ = ffi_marshal_in_seconds();
    let _ = ffi_marshal_out_seconds();
    let _ = ffi_bundle_cache_hits_total();
    let _ = ffi_wheel_load_seconds();
}

// ====================================================================
// Operational helpers — used by efe-ffi callers + tests.
// ====================================================================

/// Increment `efe_ffi_classify_total{endpoint,outcome}` + record
/// `efe_ffi_classify_seconds{endpoint}`.
///
/// **Always call this exactly once per classify**, at the end of
/// `PyEfeNativeClient::classify` (regardless of success/error). The
/// `outcome` label distinguishes success from error ; total ≈ ok + err.
pub fn note_classify(endpoint: &str, outcome: &str, seconds: f64) {
    ffi_classify_total()
        .with_label_values(&[endpoint, outcome])
        .inc();
    ffi_classify_seconds()
        .with_label_values(&[endpoint])
        .observe(seconds.max(0.0));
}

/// Record one `op`'s GIL held + released split.
///
/// `held_seconds + released_seconds ≈ total op duration` (= the sum
/// is monotonically ≤ total ; the rest is overhead).
///
/// SLO-FFI-GIL-RELEASED targets `released / (held + released) ≥ 0.95`.
pub fn note_gil_split(op: &str, held_seconds: f64, released_seconds: f64) {
    ffi_gil_held_seconds()
        .with_label_values(&[op])
        .observe(held_seconds.max(0.0));
    ffi_gil_released_seconds()
        .with_label_values(&[op])
        .observe(released_seconds.max(0.0));
}

/// Record Python → Rust marshal latency.
pub fn note_marshal_in(kind: &str, seconds: f64) {
    ffi_marshal_in_seconds()
        .with_label_values(&[kind])
        .observe(seconds.max(0.0));
}

/// Record Rust → Python marshal latency.
pub fn note_marshal_out(kind: &str, seconds: f64) {
    ffi_marshal_out_seconds()
        .with_label_values(&[kind])
        .observe(seconds.max(0.0));
}

/// Increment the bundle-cache outcome counter.
///
/// `outcome` ∈ {hit, miss, evict}. The pin test verifies a non-empty
/// set of callers for each outcome variant.
pub fn note_bundle_cache(outcome: &str) {
    ffi_bundle_cache_hits_total()
        .with_label_values(&[outcome])
        .inc();
}

/// Record the wheel cold-load time. Called exactly once at module
/// init in `efe-ffi::lib::efe_native()`.
///
/// `python_version` is the runtime CPython version (e.g. "3.10",
/// "3.11", "3.12", "3.13") detected via `Python::version_info`.
pub fn note_wheel_load(python_version: &str, seconds: f64) {
    ffi_wheel_load_seconds()
        .with_label_values(&[python_version])
        .observe(seconds.max(0.0));
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus::core::Collector;
    use std::sync::Mutex;

    /// Φ-D-OPS audit fix : the FFI counters are **global statics** ;
    /// `cargo test` runs tests in parallel by default. Two tests
    /// observing the same label cell (`("sync", "ok")`) under read-
    /// modify-read patterns race. We serialise just the conflict
    /// set with this module-local mutex. Other tests stay parallel.
    static CLASSIFY_TOTAL_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn register_all_ffi_idempotent() {
        register_all_ffi();
        register_all_ffi();
    }

    #[test]
    fn register_all_ffi_covers_8_instruments() {
        register_all_ffi();
        // Count Φ-C-FFI families on the registry. Each `once_metric!`
        // generates exactly one MetricFamily.
        let mfs = global_registry().gather();
        let ffi_names: Vec<&str> = mfs
            .iter()
            .map(|mf| mf.get_name())
            .filter(|n| n.starts_with("efe_ffi_"))
            .collect();
        assert_eq!(
            ffi_names.len(),
            8,
            "expected 8 efe_ffi_* families, got {ffi_names:?}",
        );
    }

    #[test]
    fn note_classify_increments_total_and_records_histogram() {
        let _guard = CLASSIFY_TOTAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        register_all_ffi();
        // Snapshot the before-counts.
        let before = ffi_classify_total()
            .with_label_values(&["sync", "ok"])
            .get();
        note_classify("sync", "ok", 0.012);
        let after = ffi_classify_total()
            .with_label_values(&["sync", "ok"])
            .get();
        assert_eq!(after, before + 1);
    }

    #[test]
    fn note_classify_err_outcome_distinct() {
        let _guard = CLASSIFY_TOTAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        register_all_ffi();
        let ok_before = ffi_classify_total()
            .with_label_values(&["sync", "ok"])
            .get();
        note_classify("sync", "err", 0.001);
        let ok_after = ffi_classify_total()
            .with_label_values(&["sync", "ok"])
            .get();
        // ok branch should NOT have moved
        assert_eq!(ok_after, ok_before);
    }

    #[test]
    fn note_gil_split_records_both_sides() {
        register_all_ffi();
        // Sanity : the call doesn't panic on standard values.
        note_gil_split("classify", 0.001, 0.029);
        note_gil_split("classify", 0.0, 0.0); // boundary
    }

    #[test]
    fn note_marshal_in_records() {
        register_all_ffi();
        note_marshal_in("str", 0.000_200);
        note_marshal_in("array", 0.000_050);
    }

    #[test]
    fn note_marshal_out_records() {
        register_all_ffi();
        note_marshal_out("verdict", 0.000_100);
        note_marshal_out("els", 0.000_500);
    }

    #[test]
    fn note_bundle_cache_outcomes() {
        register_all_ffi();
        let hit_before = ffi_bundle_cache_hits_total()
            .with_label_values(&["hit"])
            .get();
        note_bundle_cache("hit");
        note_bundle_cache("hit");
        note_bundle_cache("miss");
        note_bundle_cache("evict");
        let hit_after = ffi_bundle_cache_hits_total()
            .with_label_values(&["hit"])
            .get();
        assert_eq!(hit_after, hit_before + 2);
    }

    #[test]
    fn note_wheel_load_observes_once() {
        register_all_ffi();
        // Pas de panic même sur valeurs limites.
        note_wheel_load("3.12", 0.025);
        note_wheel_load("3.10", 0.080);
    }

    #[test]
    fn negative_seconds_are_clamped_to_zero() {
        // **Φ-D-OPS audit fix** : this test increments the
        // `("sync","ok")` counter family ; under parallel test
        // execution it races with `note_classify_err_outcome_distinct`
        // which observes that label cell. Serialise both via the
        // shared module-local mutex.
        let _guard = CLASSIFY_TOTAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        register_all_ffi();
        // L'horloge monotonique peut produire un epsilon négatif
        // sur certains kernels Linux. Le helper doit clamp à 0
        // plutôt que de propager une valeur invalide au histogramme.
        note_classify("sync", "ok", -1e-9);
        note_marshal_in("str", -1e-9);
        note_marshal_out("verdict", -1e-9);
        note_gil_split("classify", -1e-9, -1e-9);
        note_wheel_load("3.12", -1e-9);
    }
}
