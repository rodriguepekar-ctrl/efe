//! PR #17 Group C — Safety metrics (1 series).
//!
//! Architecture reference : `docs/EFE_A1_PHASE2_PR17_ARCHITECTURE.md` § 8.
//!
//! | Series                                          | Type           | Labels                       |
//! | ----------------------------------------------- | -------------- | ---------------------------- |
//! | `efe_meta_swarm_safety_violations_total`        | IntCounterVec  | `kind`,`participant_id`      |
//!
//! Safety violations are rare in Phase 2 (the algorithm is contractive
//! by construction).  Expected steady-state rate : < 1 per hour per
//! pod.  The metric is a **rate-of-rate-of-rate** alerting target
//! (AD23) — operators care about acceleration, not absolute count.

use prometheus::{register_int_counter_vec_with_registry, IntCounterVec, Registry};

use super::taxonomies::SafetyKind;

/// Group C — 1 Prometheus series tracking safety-set rejections.
#[derive(Debug, Clone)]
pub struct SafetyMetrics {
    /// `efe_meta_swarm_safety_violations_total` — counter of safety
    /// projection rejections, labelled by safety variant + offending
    /// participant.
    pub violations_total: IntCounterVec,
}

impl SafetyMetrics {
    /// Register the series on `reg`.
    ///
    /// Per-participant pre-touch is deferred to
    /// [`Self::pretouch_for_participants`] ; here we seed each `kind`
    /// with a sentinel `participant_id="(unknown)"` so the HELP+TYPE
    /// comments appear in `/metrics` before the first manifest
    /// load.
    pub fn register(reg: &Registry) -> prometheus::Result<Self> {
        let violations_total = register_int_counter_vec_with_registry!(
            "efe_meta_swarm_safety_violations_total",
            "Total safety-set violations detected during Iterate, partitioned by safety kind and offending participant.",
            &["kind", "participant_id"],
            reg,
        )?;
        for k in SafetyKind::all() {
            violations_total
                .with_label_values(&[k.as_str(), "(unknown)"])
                .inc_by(0);
        }
        Ok(Self { violations_total })
    }

    /// Pre-touch every (kind × participant_id) tuple.  Called by the
    /// parent registry after the manifest is loaded.
    pub fn pretouch_for_participants<I, S>(&self, participant_ids: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for id_owned in participant_ids {
            let id = id_owned.as_ref();
            for k in SafetyKind::all() {
                self.violations_total
                    .with_label_values(&[k.as_str(), id])
                    .inc_by(0);
            }
        }
    }

    /// Increment the violation counter for a (kind, participant_id)
    /// tuple.  Called from `controller::iterate::safety_check` on
    /// rejection.
    pub fn record_violation(&self, kind: SafetyKind, participant_id: &str) {
        self.violations_total
            .with_label_values(&[kind.as_str(), participant_id])
            .inc();
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
        TextEncoder::new()
            .encode_to_string(&reg.gather())
            .expect("encode")
    }

    #[test]
    fn register_creates_violations_total() {
        let r = Registry::new();
        let _m = SafetyMetrics::register(&r).expect("register");
        let b = body(&r);
        assert!(b.contains("efe_meta_swarm_safety_violations_total"));
    }

    #[test]
    fn pretouch_for_participants_emits_all_kinds() {
        let r = Registry::new();
        let m = SafetyMetrics::register(&r).expect("register");
        m.pretouch_for_participants(["p1", "p2"]);
        let b = body(&r);
        for kind in SafetyKind::all() {
            for id in ["p1", "p2"] {
                let needle = format!(
                    "efe_meta_swarm_safety_violations_total{{kind=\"{}\",participant_id=\"{}\"}} 0",
                    kind.as_str(),
                    id,
                );
                assert!(b.contains(&needle), "missing pretouch : {needle}\n{b}");
            }
        }
    }

    #[test]
    fn record_violation_increments_correct_label() {
        let r = Registry::new();
        let m = SafetyMetrics::register(&r).expect("register");
        m.pretouch_for_participants(["p1"]);
        m.record_violation(SafetyKind::Box, "p1");
        m.record_violation(SafetyKind::Box, "p1");
        m.record_violation(SafetyKind::Poly, "p1");
        let box_count = m
            .violations_total
            .get_metric_with_label_values(&["box", "p1"])
            .unwrap()
            .get();
        let poly_count = m
            .violations_total
            .get_metric_with_label_values(&["poly", "p1"])
            .unwrap()
            .get();
        let hull_count = m
            .violations_total
            .get_metric_with_label_values(&["hull", "p1"])
            .unwrap()
            .get();
        assert_eq!(box_count, 2);
        assert_eq!(poly_count, 1);
        assert_eq!(hull_count, 0);
    }
}
