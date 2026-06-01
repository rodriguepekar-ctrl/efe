//! PR #17 Group B — Step metrics (3 series).
//!
//! Architecture reference : `docs/EFE_A1_PHASE2_PR17_ARCHITECTURE.md` § 7.
//!
//! | Series                                       | Type         | Labels                                  |
//! | -------------------------------------------- | ------------ | --------------------------------------- |
//! | `efe_meta_swarm_step_duration_seconds`       | HistogramVec | `participant_id`,`result`               |
//! | `efe_meta_swarm_step_total`                  | IntCounterVec| `participant_id`,`result`,`failure_reason`|
//! | `efe_meta_swarm_step_payload_bytes`          | HistogramVec | `direction`                             |
//!
//! The `participant_id` label is OPEN at registration time ; pre-touch
//! is deferred to [`super::MetricsRegistry::pretouch_for_manifest`],
//! which knows the manifest's `participants` list.

use std::time::Duration;

use prometheus::{
    register_histogram_vec_with_registry, register_int_counter_vec_with_registry, HistogramVec,
    IntCounterVec, Registry,
};

use super::taxonomies::{FailureReason, Outcome, PayloadDirection};

// ─────────────────────────────────────────────────────────────────────
// Bucket policies — frozen per chart version (AD6 / AD16).
// ─────────────────────────────────────────────────────────────────────

/// Narrow-range latency buckets : 0.5 ms → 2.5 s, 12 buckets.  Sized for
/// per-participant Step calls whose realistic upper bound is < 1 s.
pub const STEP_DURATION_BUCKETS_SECONDS: &[f64] = &[
    0.000_5, 0.001, 0.002_5, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5,
];

/// Payload-size buckets : 64 B → 1 MB, 8 buckets.  Phase-2 tensor sizes
/// (dim ∈ [4, 64], f32 ⇒ 16 B → 256 B) live in the low end ; the high
/// end (256 KB → 1 MB) catches anomaly tensors before the wire-
/// validation layer rejects them at the 1 MB cap (AD27).
pub const STEP_PAYLOAD_BUCKETS_BYTES: &[f64] = &[
    64.0,
    256.0,
    1_024.0,
    4_096.0,
    16_384.0,
    65_536.0,
    262_144.0,
    1_048_576.0,
];

// ─────────────────────────────────────────────────────────────────────
// StepMetrics — the 3-series bundle.
// ─────────────────────────────────────────────────────────────────────

/// Group B — 3 Prometheus series covering per-participant Step call
/// observability.
#[derive(Debug, Clone)]
pub struct StepMetrics {
    /// `efe_meta_swarm_step_duration_seconds` — wall-clock latency of a
    /// single Step gRPC call, labelled by participant + outcome.
    pub duration_seconds: HistogramVec,
    /// `efe_meta_swarm_step_total` — counter of Step completions.
    pub total: IntCounterVec,
    /// `efe_meta_swarm_step_payload_bytes` — histogram of tensor payload
    /// sizes crossing the wire, labelled by direction.
    pub payload_bytes: HistogramVec,
}

impl StepMetrics {
    /// Register every series on `reg`.
    ///
    /// Closed-taxonomy pre-touch covers `direction` only (a 2-value
    /// enum) ; the per-participant pre-touch is performed by the
    /// parent registry at manifest-load time.
    pub fn register(reg: &Registry) -> prometheus::Result<Self> {
        let duration_seconds = register_histogram_vec_with_registry!(
            "efe_meta_swarm_step_duration_seconds",
            "Per-participant Step gRPC call duration in seconds, partitioned by participant and outcome.",
            &["participant_id", "result"],
            STEP_DURATION_BUCKETS_SECONDS.to_vec(),
            reg,
        )?;

        let total = register_int_counter_vec_with_registry!(
            "efe_meta_swarm_step_total",
            "Total Step calls completed, partitioned by participant, outcome, and failure reason.",
            &["participant_id", "result", "failure_reason"],
            reg,
        )?;

        let payload_bytes = register_histogram_vec_with_registry!(
            "efe_meta_swarm_step_payload_bytes",
            "Size in bytes of the tensor payload crossing the Step wire, partitioned by direction (`in` = pull tensor received from controller; `out` = post-step tensor sent back).",
            &["direction"],
            STEP_PAYLOAD_BUCKETS_BYTES.to_vec(),
            reg,
        )?;

        let m = Self {
            duration_seconds,
            total,
            payload_bytes,
        };
        m.pretouch_direction_only();
        Ok(m)
    }

    /// Pre-touch the `direction` label only.  Participant-id pre-touch
    /// is deferred to [`Self::pretouch_for_participants`].  The
    /// `participant_id`-labelled series are seeded with a sentinel
    /// `"(unknown)"` so the HELP+TYPE comments appear in `/metrics`
    /// before the first Step observation (otherwise the Prometheus
    /// text encoder skips empty families).
    fn pretouch_direction_only(&self) {
        for d in PayloadDirection::all() {
            self.payload_bytes
                .with_label_values(&[d.as_str()])
                .observe(0.0);
        }
        for outcome in Outcome::all() {
            self.duration_seconds
                .with_label_values(&["(unknown)", outcome.as_str()])
                .observe(0.0);
            for reason in FailureReason::all() {
                self.total
                    .with_label_values(&["(unknown)", outcome.as_str(), reason.as_str()])
                    .inc_by(0);
            }
        }
    }

    /// Pre-touch every (participant_id × outcome) and (participant_id ×
    /// outcome × failure_reason) combination.  Called by the parent
    /// registry on manifest load + every reload.
    pub fn pretouch_for_participants<I, S>(&self, participant_ids: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for id_owned in participant_ids {
            let id = id_owned.as_ref();
            for outcome in Outcome::all() {
                self.duration_seconds
                    .with_label_values(&[id, outcome.as_str()])
                    .observe(0.0);
                for reason in FailureReason::all() {
                    self.total
                        .with_label_values(&[id, outcome.as_str(), reason.as_str()])
                        .inc_by(0);
                }
            }
        }
    }

    /// Observe a Step call's duration.
    pub fn observe_duration(&self, duration: Duration, participant_id: &str, outcome: Outcome) {
        self.duration_seconds
            .with_label_values(&[participant_id, outcome.as_str()])
            .observe(duration.as_secs_f64());
    }

    /// Increment the per-participant Step counter.
    pub fn record_total(&self, participant_id: &str, outcome: Outcome, reason: FailureReason) {
        self.total
            .with_label_values(&[participant_id, outcome.as_str(), reason.as_str()])
            .inc();
    }

    /// Observe a payload-size sample on the wire.
    pub fn observe_payload_bytes(&self, bytes: usize, direction: PayloadDirection) {
        self.payload_bytes
            .with_label_values(&[direction.as_str()])
            // Cast widening : `bytes` ≤ 1 MB, well within f64 mantissa.
            .observe(bytes as f64);
    }
}

// ─────────────────────────────────────────────────────────────────────
// Tests.
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus::TextEncoder;

    fn body(reg: &Registry) -> String {
        let encoded = TextEncoder::new()
            .encode_to_string(&reg.gather())
            .expect("encode");
        encoded
    }

    #[test]
    fn register_exposes_all_three_series() {
        let r = Registry::new();
        let _m = StepMetrics::register(&r).expect("register");
        let b = body(&r);
        for n in [
            "efe_meta_swarm_step_duration_seconds",
            "efe_meta_swarm_step_total",
            "efe_meta_swarm_step_payload_bytes",
        ] {
            assert!(b.contains(n), "missing : {n}");
        }
    }

    #[test]
    fn direction_pretouch_emits_in_and_out() {
        let r = Registry::new();
        let _m = StepMetrics::register(&r).expect("register");
        let b = body(&r);
        assert!(b.contains("direction=\"in\""));
        assert!(b.contains("direction=\"out\""));
    }

    #[test]
    fn pretouch_for_participants_materialises_per_id_series() {
        let r = Registry::new();
        let m = StepMetrics::register(&r).expect("register");
        m.pretouch_for_participants(["supplier-scoring", "contract-risk"]);
        let b = body(&r);
        // After pretouch : every (id, outcome) tuple emitted.
        for id in ["supplier-scoring", "contract-risk"] {
            for outcome in Outcome::all() {
                let needle = format!(
                    "efe_meta_swarm_step_duration_seconds_count{{participant_id=\"{}\",result=\"{}\"}} 1",
                    id,
                    outcome.as_str(),
                );
                assert!(b.contains(&needle), "missing pretouch : {needle}\n{b}");
            }
        }
    }

    #[test]
    fn observe_duration_records_sample() {
        let r = Registry::new();
        let m = StepMetrics::register(&r).expect("register");
        m.pretouch_for_participants(["p1"]);
        m.observe_duration(Duration::from_millis(7), "p1", Outcome::Ok);
        let count = m
            .duration_seconds
            .get_metric_with_label_values(&["p1", "ok"])
            .unwrap()
            .get_sample_count();
        assert_eq!(count, 2); // pretouch (1) + observation (1)
    }

    #[test]
    fn record_total_increments_per_participant() {
        let r = Registry::new();
        let m = StepMetrics::register(&r).expect("register");
        m.pretouch_for_participants(["p1", "p2"]);
        m.record_total("p1", Outcome::Error, FailureReason::DimMismatch);
        m.record_total("p2", Outcome::Ok, FailureReason::Other);
        m.record_total("p1", Outcome::Error, FailureReason::DimMismatch);
        let p1 = m
            .total
            .get_metric_with_label_values(&["p1", "error", "dim_mismatch"])
            .unwrap()
            .get();
        assert_eq!(p1, 2);
        let p2 = m
            .total
            .get_metric_with_label_values(&["p2", "ok", "other"])
            .unwrap()
            .get();
        assert_eq!(p2, 1);
    }

    #[test]
    fn observe_payload_bytes_records_with_direction() {
        let r = Registry::new();
        let m = StepMetrics::register(&r).expect("register");
        m.observe_payload_bytes(256, PayloadDirection::In);
        m.observe_payload_bytes(1024, PayloadDirection::Out);
        m.observe_payload_bytes(64 * 1024, PayloadDirection::Out);
        let in_count = m
            .payload_bytes
            .get_metric_with_label_values(&["in"])
            .unwrap()
            .get_sample_count();
        let out_count = m
            .payload_bytes
            .get_metric_with_label_values(&["out"])
            .unwrap()
            .get_sample_count();
        assert_eq!(in_count, 2); // pretouch + 1 observation
        assert_eq!(out_count, 3); // pretouch + 2 observations
    }

    #[test]
    fn bucket_lists_match_architecture() {
        assert_eq!(STEP_DURATION_BUCKETS_SECONDS.len(), 12);
        assert_eq!(STEP_PAYLOAD_BUCKETS_BYTES.len(), 8);
        // 1 MB cap matches AD27.
        assert_eq!(STEP_PAYLOAD_BUCKETS_BYTES.last(), Some(&1_048_576.0));
    }
}
