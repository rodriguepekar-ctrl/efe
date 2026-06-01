//! Sprint **D16** — embedder bundling instrumentation.
//!
//! 9 nouveaux instruments dédiés au pipeline bundle (load, signature
//! verify, hash check, encode, resident memory). Cf.
//! `docs/bench/d16_embedder_bundling_architecture.md` § 13.
//!
//! Conventions identiques à `d14bis_instruments.rs` /
//! `d15_instruments.rs` :
//!   * macro `once_metric!` (re-déclarée localement) ;
//!   * tous les instruments enregistrés sur `global_registry` ;
//!   * labels strictement bornés ;
//!   * outcome strings pinned by `LoadOutcome::as_label` côté
//!     `efe-server::embedder::bundle`.
//!
//! # Cardinalité (estim.)
//!
//! Pour 5 bundles × 100 tenants × 4 keys (rotation in flight) :
//!
//! | Instrument                                     | Combinaisons |
//! |------------------------------------------------|-------------:|
//! | `efe_bundle_load_total`                        | 5 × 6 = 30 |
//! | `efe_bundle_load_duration_seconds`             | 5 × 6 = 30 |
//! | `efe_bundle_signature_verifications_total`     | 5 × 4 × 3 = 60 |
//! | `efe_bundle_hash_verifications_total`          | 5 × 4 = 20 |
//! | `efe_embedder_encode_duration_seconds`         | 5 × 100 = 500 |
//! | `efe_embedder_encode_total`                    | 5 × 100 × 4 = 2 000 |
//! | `efe_embedder_active_bundles`                  | 1 |
//! | `efe_embedder_resident_bytes`                  | 5 |
//! | `efe_embedder_tokenizer_truncations_total`     | 5 × 100 = 500 |
//!
//! Total ~3 150 séries — sous le budget D13 50 000 panic. Au-dessus
//! de 5 bundles × 100 tenants, basculer le label `tenant` en hash
//! bucket (cf. architecture § 10.4).

use std::sync::OnceLock;

use prometheus::{
    exponential_buckets, HistogramOpts, HistogramVec, IntCounterVec, IntGauge,
    IntGaugeVec, Opts,
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
// Bundle load
// =====================================================================

once_metric!(
    /// `efe_bundle_load_total{bundle_id, outcome}` — count of bundle
    /// load attempts.
    ///
    /// `outcome` is one of (per `LoadOutcome::as_label`) :
    ///   * `ok` — load + verify + sign + runtime init all succeeded ;
    ///   * `manifest_invalid` — IO / parse / invariant failure ;
    ///   * `signature_failed` — sig missing when required, key
    ///     mismatch, expired key ;
    ///   * `hash_mismatch` — component SHA-256 / size mismatch ;
    ///   * `runtime_init_failed` — ORT / safetensors session crash ;
    ///   * `policy_violation` — SBOM required but absent / license
    ///     forbids redistribution without ACK.
    ///
    /// Alert `D16BundleLoadFailing` fires on rate of any non-ok.
    bundle_load_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_bundle_load_total",
            "Bundle load attempts by bundle_id + outcome (D16 § 13).",
        ),
        &["bundle_id", "outcome"],
    )
    .expect("valid counter opts")
);

once_metric!(
    /// `efe_bundle_load_duration_seconds{bundle_id, outcome}` — wall-
    /// clock duration of a bundle load step (parse + verify + sign +
    /// runtime init).
    ///
    /// Buckets : 0.1, 0.2, 0.4, … 25.6 s (8 buckets, base 2). Covers
    /// the SLO **D16-BND-1** ≤ 5 s p99.
    bundle_load_duration_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_bundle_load_duration_seconds",
            "Wall-clock duration of one bundle load (D16 SLO BND-1 ≤ 5 s p99).",
        )
        .buckets(exponential_buckets(0.1, 2.0, 8).expect("valid buckets")),
        &["bundle_id", "outcome"],
    )
    .expect("valid histogram opts")
);

// =====================================================================
// Signature + hash verify
// =====================================================================

once_metric!(
    /// `efe_bundle_signature_verifications_total{bundle_id, key_id, result}`
    /// — count of detached-sig verify attempts.
    ///
    /// `key_id` is the trust-store entry that validated (or
    /// `unknown` if no key matched). Useful for visualising the
    /// rotation timeline (architecture § 5.3) and detecting
    /// single-key reliance (R-16-5 mitigation).
    ///
    /// `result` is one of `ok` / `bad_signature` / `key_expired` /
    /// `no_currently_valid_key`.
    ///
    /// Alert `D16BundleSignatureMismatch` fires page-level on any
    /// non-ok rate > 0.
    bundle_signature_verifications_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_bundle_signature_verifications_total",
            "Detached-sig verify attempts by bundle_id + key_id + result.",
        ),
        &["bundle_id", "key_id", "result"],
    )
    .expect("valid counter opts")
);

once_metric!(
    /// `efe_bundle_hash_verifications_total{bundle_id, result}` —
    /// count of per-component hash check passes.
    ///
    /// `result` ∈ {`ok`, `mismatch_required`, `mismatch_optional`,
    /// `missing_required`, `missing_optional`}.
    bundle_hash_verifications_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_bundle_hash_verifications_total",
            "Per-component hash / size verify outcomes.",
        ),
        &["bundle_id", "result"],
    )
    .expect("valid counter opts")
);

// =====================================================================
// Encode hot path
// =====================================================================

once_metric!(
    /// `efe_embedder_encode_total{bundle_id, tenant, outcome}` —
    /// hot-path encoder success / failure counter.
    ///
    /// `outcome` ∈ {`ok`, `tokenizer_failed`, `forward_failed`,
    /// `empty_input`, `nan_output`, `truncated`}. The `truncated`
    /// label fires when the input had to be cut to `max_seq_len` ;
    /// it's not a failure but a signal for client-side bug detection.
    embedder_encode_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_embedder_encode_total",
            "Per-RPC encoder counter by bundle + tenant + outcome.",
        ),
        &["bundle_id", "tenant", "outcome"],
    )
    .expect("valid counter opts")
);

once_metric!(
    /// `efe_embedder_encode_duration_seconds{bundle_id, tenant}` —
    /// hot-path encoder latency histogram.
    ///
    /// Buckets : 0.001, 0.002, … 4.096 s (12 buckets, base 2). SLO
    /// **D16-BND-2** : p99 ≤ 50 ms CPU ; ≤ 5 ms GPU.
    embedder_encode_duration_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_embedder_encode_duration_seconds",
            "Per-RPC encoder latency (D16 SLO BND-2).",
        )
        .buckets(exponential_buckets(0.001, 2.0, 12).expect("valid buckets")),
        &["bundle_id", "tenant"],
    )
    .expect("valid histogram opts")
);

// =====================================================================
// Tokenizer + sanity gauges
// =====================================================================

once_metric!(
    /// `efe_embedder_tokenizer_truncations_total{bundle_id, tenant}`
    /// — number of inputs that exceeded `runtime.max_seq_len` and
    /// got truncated. High rate ⇒ client-side bug (sending essays
    /// to a sentence-length embedder).
    embedder_tokenizer_truncations_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_embedder_tokenizer_truncations_total",
            "Number of encode calls whose input was truncated to max_seq_len.",
        ),
        &["bundle_id", "tenant"],
    )
    .expect("valid counter opts")
);

once_metric!(
    /// `efe_embedder_active_bundles` — number of bundles currently
    /// loaded in the [`crate::embedder::EmbedderRegistry`]. 0 ⇒ no
    /// bundle = nothing to serve = page-level alert
    /// `D16BundleNoActive`.
    embedder_active_bundles,
    IntGauge,
    IntGauge::new(
        "efe_embedder_active_bundles",
        "Number of bundles currently loaded in the registry.",
    )
    .expect("valid gauge opts")
);

once_metric!(
    /// `efe_embedder_resident_bytes{bundle_id}` — RSS attribution per
    /// bundle. Set after each load. Used for capacity planning and
    /// alert `D16EmbedderResidentSurge`.
    embedder_resident_bytes,
    IntGaugeVec,
    IntGaugeVec::new(
        Opts::new(
            "efe_embedder_resident_bytes",
            "Resident memory bytes attributable to each loaded bundle.",
        ),
        &["bundle_id"],
    )
    .expect("valid gauge opts")
);

// =====================================================================
// Eager registration
// =====================================================================

/// Force-register every D16 instrument with the global registry so
/// the Prometheus `/metrics` endpoint exposes them as zero-valued
/// series even before the first encode. Pattern identical to D15
/// audit fix H5 — dashboards render at boot.
///
/// Idempotent — the `once_metric!` macros guard against
/// double-registration. Called from `runtime::Server::build`.
pub fn register_all_d16() {
    let _ = bundle_load_total();
    let _ = bundle_load_duration_seconds();
    let _ = bundle_signature_verifications_total();
    let _ = bundle_hash_verifications_total();
    let _ = embedder_encode_total();
    let _ = embedder_encode_duration_seconds();
    let _ = embedder_tokenizer_truncations_total();
    let _ = embedder_active_bundles();
    let _ = embedder_resident_bytes();
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus::core::Collector;

    #[test]
    fn bundle_load_total_increments() {
        let labels = [
            "sentence-transformers/all-MiniLM-L6-v2@1.0.0/onnx/384",
            "ok",
        ];
        let before = bundle_load_total().with_label_values(&labels).get();
        bundle_load_total().with_label_values(&labels).inc();
        let after = bundle_load_total().with_label_values(&labels).get();
        assert_eq!(after, before + 1);
    }

    #[test]
    fn outcomes_are_documented() {
        // Architecture § 13.3 : exactly 6 outcome values for the load
        // counter. Use them all here ; a typo would produce extra
        // series the dashboards don't know about.
        let outcomes = [
            "ok",
            "manifest_invalid",
            "signature_failed",
            "hash_mismatch",
            "runtime_init_failed",
            "policy_violation",
        ];
        for o in outcomes {
            bundle_load_total()
                .with_label_values(&["x/y@1.0.0/onnx/64", o])
                .inc();
        }
    }

    #[test]
    fn hash_verification_results_documented() {
        for r in [
            "ok",
            "mismatch_required",
            "mismatch_optional",
            "missing_required",
            "missing_optional",
        ] {
            bundle_hash_verifications_total()
                .with_label_values(&["x/y@1.0.0/onnx/64", r])
                .inc();
        }
    }

    #[test]
    fn signature_verification_results_documented() {
        for r in ["ok", "bad_signature", "key_expired", "no_currently_valid_key"] {
            bundle_signature_verifications_total()
                .with_label_values(&["x/y@1.0.0/onnx/64", "test-key", r])
                .inc();
        }
    }

    #[test]
    fn encode_outcomes_documented() {
        for o in [
            "ok",
            "tokenizer_failed",
            "forward_failed",
            "empty_input",
            "nan_output",
            "truncated",
        ] {
            embedder_encode_total()
                .with_label_values(&["x/y@1.0.0/onnx/64", "tenant1", o])
                .inc();
        }
    }

    #[test]
    fn active_bundles_settable() {
        embedder_active_bundles().set(3);
        assert_eq!(embedder_active_bundles().get(), 3);
    }

    #[test]
    fn resident_bytes_settable() {
        embedder_resident_bytes()
            .with_label_values(&["x/y@1.0.0/onnx/64"])
            .set(123_456);
        assert_eq!(
            embedder_resident_bytes()
                .with_label_values(&["x/y@1.0.0/onnx/64"])
                .get(),
            123_456,
        );
    }

    #[test]
    fn duration_histogram_observes() {
        bundle_load_duration_seconds()
            .with_label_values(&["x/y@1.0.0/onnx/64", "ok"])
            .observe(0.4);
        let metric =
            bundle_load_duration_seconds().with_label_values(&["x/y@1.0.0/onnx/64", "ok"]);
        assert!(metric.get_sample_count() >= 1);
    }

    #[test]
    fn label_arity_pinned() {
        // Architecture § 13 commits to specific label arities.
        // A refactor that adds a label silently would break the
        // dashboard PromQL queries.
        assert_eq!(bundle_load_total().desc()[0].variable_labels.len(), 2);
        assert_eq!(
            bundle_load_duration_seconds().desc()[0].variable_labels.len(),
            2
        );
        assert_eq!(
            bundle_signature_verifications_total().desc()[0]
                .variable_labels
                .len(),
            3
        );
        assert_eq!(
            bundle_hash_verifications_total().desc()[0].variable_labels.len(),
            2
        );
        assert_eq!(embedder_encode_total().desc()[0].variable_labels.len(), 3);
        assert_eq!(
            embedder_encode_duration_seconds().desc()[0].variable_labels.len(),
            2
        );
        assert_eq!(
            embedder_tokenizer_truncations_total().desc()[0]
                .variable_labels
                .len(),
            2
        );
        assert_eq!(embedder_resident_bytes().desc()[0].variable_labels.len(), 1);
        // active_bundles is a bare IntGauge with no labels.
    }

    #[test]
    fn register_all_d16_idempotent() {
        register_all_d16();
        register_all_d16();
        // No assertion ; absence of panic is the signal.
    }
}
