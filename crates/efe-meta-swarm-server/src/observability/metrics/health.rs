//! PR #17 Group D — Participant health metrics (2 series).
//!
//! Architecture reference : `docs/EFE_A1_PHASE2_PR17_ARCHITECTURE.md` § 9.
//!
//! | Series                                          | Type           | Labels             |
//! | ----------------------------------------------- | -------------- | ------------------ |
//! | `efe_meta_swarm_participants_configured`        | IntGauge       | —                  |
//! | `efe_meta_swarm_participant_up`                 | IntGaugeVec    | `participant_id`   |
//!
//! ## Semantic — `participant_up`
//!
//! Per AD24, the gauge follows Prometheus's standard `up` semantic :
//!
//!   * `1` ⇒ last Step succeeded.
//!   * `0` ⇒ last Step failed OR no contact in > 30 s (the latter
//!     enforced by an external timer, not by the gauge itself).
//!
//! The gauge does NOT auto-decay ; once set to 1, it stays at 1
//! until the next Step error.  PR #19 will add a Grafana alert
//! `MetaSwarmParticipantStaleUp` that catches stale gauges via
//! `time() - max_over_time(timestamp(...)[5m]) > 300`.

use prometheus::{
    register_int_gauge_vec_with_registry, register_int_gauge_with_registry, IntGauge, IntGaugeVec,
    Registry,
};

/// Group D — 2 Prometheus series covering per-participant health.
#[derive(Debug, Clone)]
pub struct ParticipantHealthMetrics {
    /// `efe_meta_swarm_participants_configured` — gauge of declared
    /// participants in the current loaded manifest.  Updated at every
    /// manifest reload.
    pub participants_configured: IntGauge,
    /// `efe_meta_swarm_participant_up` — per-participant connectivity
    /// gauge.  `1` = last Step succeeded, `0` = last Step failed.
    pub participant_up: IntGaugeVec,
}

impl ParticipantHealthMetrics {
    /// Register both series on `reg`.  The `participant_up` MetricVec
    /// is seeded with a sentinel `participant_id="(unknown)"` so the
    /// HELP+TYPE comments appear in `/metrics` before the first
    /// manifest load.
    pub fn register(reg: &Registry) -> prometheus::Result<Self> {
        let participants_configured = register_int_gauge_with_registry!(
            "efe_meta_swarm_participants_configured",
            "Number of participants declared in the loaded manifest.",
            reg,
        )?;
        let participant_up = register_int_gauge_vec_with_registry!(
            "efe_meta_swarm_participant_up",
            "Per-participant connectivity gauge : 1 = last Step succeeded, 0 = last Step failed or no contact in > 30 s.",
            &["participant_id"],
            reg,
        )?;
        // Sentinel pre-touch so the series appears in /metrics at boot.
        participant_up.with_label_values(&["(unknown)"]).set(1);
        Ok(Self {
            participants_configured,
            participant_up,
        })
    }

    /// Pre-touch every per-participant series at `1` (assumed healthy
    /// at boot — first failed Step flips to `0`).
    pub fn pretouch_for_participants<I, S>(&self, participant_ids: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut count = 0_i64;
        for id_owned in participant_ids {
            let id = id_owned.as_ref();
            self.participant_up.with_label_values(&[id]).set(1);
            count += 1;
        }
        self.participants_configured.set(count);
    }

    /// Set the participant's up-gauge to `1` (e.g. on successful Step
    /// or after a successful reconnect).
    pub fn mark_up(&self, participant_id: &str) {
        self.participant_up
            .with_label_values(&[participant_id])
            .set(1);
    }

    /// Set the participant's up-gauge to `0` (e.g. after a failed Step).
    pub fn mark_down(&self, participant_id: &str) {
        self.participant_up
            .with_label_values(&[participant_id])
            .set(0);
    }

    /// Update the `participants_configured` gauge after a manifest
    /// reload.
    pub fn set_configured(&self, count: usize) {
        // `usize → i64` cast : `count` is bounded by manifest schema
        // (max 10) ; no overflow risk.
        self.participants_configured.set(count as i64);
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
    fn register_creates_both_series() {
        let r = Registry::new();
        let _m = ParticipantHealthMetrics::register(&r).expect("register");
        let b = body(&r);
        assert!(b.contains("efe_meta_swarm_participants_configured"));
        assert!(b.contains("efe_meta_swarm_participant_up"));
    }

    #[test]
    fn pretouch_for_participants_sets_count_and_per_id_up() {
        let r = Registry::new();
        let m = ParticipantHealthMetrics::register(&r).expect("register");
        m.pretouch_for_participants(["p1", "p2", "p3"]);
        let b = body(&r);
        assert!(b.contains("efe_meta_swarm_participants_configured 3"));
        for id in ["p1", "p2", "p3"] {
            let needle = format!("efe_meta_swarm_participant_up{{participant_id=\"{id}\"}} 1");
            assert!(b.contains(&needle), "missing : {needle}");
        }
    }

    #[test]
    fn mark_down_flips_gauge_to_zero() {
        let r = Registry::new();
        let m = ParticipantHealthMetrics::register(&r).expect("register");
        m.pretouch_for_participants(["p1"]);
        m.mark_down("p1");
        let v = m
            .participant_up
            .get_metric_with_label_values(&["p1"])
            .unwrap()
            .get();
        assert_eq!(v, 0);
    }

    #[test]
    fn mark_up_after_mark_down_restores_one() {
        let r = Registry::new();
        let m = ParticipantHealthMetrics::register(&r).expect("register");
        m.mark_down("p1");
        m.mark_up("p1");
        let v = m
            .participant_up
            .get_metric_with_label_values(&["p1"])
            .unwrap()
            .get();
        assert_eq!(v, 1);
    }

    #[test]
    fn set_configured_overrides_pretouch_count() {
        let r = Registry::new();
        let m = ParticipantHealthMetrics::register(&r).expect("register");
        m.pretouch_for_participants(["p1", "p2"]);
        assert_eq!(m.participants_configured.get(), 2);
        m.set_configured(5);
        assert_eq!(m.participants_configured.get(), 5);
    }
}
