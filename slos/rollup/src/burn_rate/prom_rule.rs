//! PR #22 § 11.2 + Appendix D — emit PrometheusRule YAML for the 8
//! burn-rate alerts (4 SLOs × 2 windows).

use crate::SloId;

/// One alert (fast or slow) for one SLO.
pub struct AlertDef {
    pub name: String,
    pub expr: String,
    pub for_duration: String,
    pub severity: &'static str,
    pub slo: &'static str,
    pub burn: &'static str,
    pub summary: String,
    pub runbook: String,
}

/// Emit the canonical YAML for all 8 alerts.
#[must_use]
pub fn emit_yaml() -> String {
    let mut groups = Vec::new();
    for slo in SloId::all() {
        let (target, budget_str) = match slo {
            SloId::L2Availability => (
                crate::thresholds::L2_AVAILABILITY_TARGET,
                "0.005".to_string(),
            ),
            SloId::L2LatencyP95 => (
                crate::thresholds::L2_LATENCY_P95_COMPLIANCE,
                "0.01".to_string(),
            ),
            SloId::SwarmRecoveryTime => (
                crate::thresholds::SWARM_RECOVERY_TIME_COMPLIANCE,
                "0.01".to_string(),
            ),
            SloId::ReplicationLagP99 => (
                crate::thresholds::REPLICATION_LAG_COMPLIANCE,
                "0.01".to_string(),
            ),
        };
        let _ = target;
        let slo_id = slo.as_str();
        // Fast burn : (1 - slo:X_1h)/budget >= 14.4 AND (1 - slo:X_5m)/budget >= 14.4
        let fast = AlertDef {
            name: format!("SLOBurnRateFast_{slo_id}"),
            expr: format!(
                "(1 - slo:{slo_id}_1h{{source=\"prod\"}}) / {budget_str} >= 14.4 and (1 - slo:{slo_id}_5m{{source=\"prod\"}}) / {budget_str} >= 14.4"
            ),
            for_duration: "2m".into(),
            severity: "critical",
            slo: slo_id,
            burn: "fast",
            summary: format!("{slo_id} fast-burn (2-day budget exhaustion)"),
            runbook: format!("https://efe.github.io/slos/runbooks/burn-rate.html#fast-{}", slo_id.to_lowercase()),
        };
        let slow = AlertDef {
            name: format!("SLOBurnRateSlow_{slo_id}"),
            expr: format!(
                "(1 - slo:{slo_id}_6h{{source=\"prod\"}}) / {budget_str} >= 6 and (1 - slo:{slo_id}_30m{{source=\"prod\"}}) / {budget_str} >= 6"
            ),
            for_duration: "5m".into(),
            severity: "warning",
            slo: slo_id,
            burn: "slow",
            summary: format!("{slo_id} slow-burn (5-day budget exhaustion)"),
            runbook: format!("https://efe.github.io/slos/runbooks/burn-rate.html#slow-{}", slo_id.to_lowercase()),
        };
        groups.push((fast, slow));
    }

    let mut out = String::new();
    out.push_str("# PR #22 § 11.2 + Appendix D — 2-window burn-rate alerts.\n");
    out.push_str("# Replaces PR #19's slo-burn-rate.yaml (AD40).\n");
    out.push_str("# STABILITY_CONTRACT v1.4 § 22 — frozen alert names + expressions.\n");
    out.push_str("apiVersion: monitoring.coreos.com/v1\n");
    out.push_str("kind: PrometheusRule\n");
    out.push_str("metadata:\n");
    out.push_str("  name: slo-burn-rate-2window\n");
    out.push_str("  labels:\n");
    out.push_str("    stability_contract: v1.4\n");
    out.push_str("spec:\n");
    out.push_str("  groups:\n");
    out.push_str("    - name: slo-burn-rate-fast\n");
    out.push_str("      interval: 1m\n");
    out.push_str("      rules:\n");
    for (fast, _) in &groups {
        out.push_str(&render_rule(fast));
    }
    out.push_str("    - name: slo-burn-rate-slow\n");
    out.push_str("      interval: 1m\n");
    out.push_str("      rules:\n");
    for (_, slow) in &groups {
        out.push_str(&render_rule(slow));
    }
    out
}

fn render_rule(a: &AlertDef) -> String {
    format!(
        "        - alert: {name}\n          expr: |\n            {expr}\n          for: {for_dur}\n          labels:\n            severity: {sev}\n            slo: {slo}\n            burn: {burn}\n            stability_contract: v1.4\n          annotations:\n            summary: \"{summary}\"\n            runbook: \"{runbook}\"\n",
        name = a.name,
        expr = a.expr,
        for_dur = a.for_duration,
        sev = a.severity,
        slo = a.slo,
        burn = a.burn,
        summary = a.summary,
        runbook = a.runbook,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emit_yaml_contains_8_alerts() {
        let yaml = emit_yaml();
        // 4 SLOs × 2 windows = 8 alerts.
        let count = yaml.matches("- alert:").count();
        assert_eq!(count, 8);
    }

    #[test]
    fn yaml_contains_fast_threshold_14_4() {
        let yaml = emit_yaml();
        assert!(yaml.contains(">= 14.4"));
        assert!(yaml.contains(">= 6"));
    }

    #[test]
    fn yaml_carries_stability_contract_label() {
        let yaml = emit_yaml();
        assert!(yaml.contains("stability_contract: v1.4"));
    }

    #[test]
    fn yaml_has_separate_fast_and_slow_groups() {
        let yaml = emit_yaml();
        assert!(yaml.contains("name: slo-burn-rate-fast"));
        assert!(yaml.contains("name: slo-burn-rate-slow"));
    }

    #[test]
    fn yaml_filters_to_source_prod_only_ad18() {
        let yaml = emit_yaml();
        assert!(yaml.contains("source=\"prod\""));
    }

    #[test]
    fn parses_as_yaml() {
        let yaml = emit_yaml();
        let _: serde_yaml::Value = serde_yaml::from_str(&yaml).expect("must parse as YAML");
    }
}
