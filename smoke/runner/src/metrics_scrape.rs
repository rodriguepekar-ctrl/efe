//! PR #23 § 14 — Prometheus scrape parser for S6.

use std::collections::BTreeSet;
use std::time::Duration;

use crate::checks::CheckContext;
use crate::error::{SmokeError, SmokeResult};

/// Fetch the controller's `/metrics` endpoint and parse the Prometheus
/// text-exposition format, returning the unique family names.
///
/// Phase-2 uses a port-forward via `kubectl` ; Phase-3 (G.4) hooks into
/// the cluster's actual Prom for in-namespace scraping.
pub async fn scrape_controller(ctx: &CheckContext) -> SmokeResult<BTreeSet<String>> {
    // For Phase-2 we proxy via kubectl exec → curl from inside the pod.
    // This sidesteps mTLS handshake noise.
    let pod_label = "app.kubernetes.io/name=efe-meta-swarm-controller";
    let out = std::process::Command::new("kubectl")
        .args([
            "--kubeconfig",
            ctx.kubeconfig.to_string_lossy().as_ref(),
            "exec",
            "-n",
            &ctx.namespace,
            "-l",
            pod_label,
            "--",
            "curl",
            "-s",
            "http://localhost:8080/metrics",
        ])
        .output()
        .map_err(|e| SmokeError::Source(format!("kubectl exec curl /metrics : {e}")))?;
    if !out.status.success() {
        return Err(SmokeError::Source(format!(
            "kubectl exec failed : {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(parse_prom_families(&String::from_utf8_lossy(&out.stdout)))
}

/// Parse a Prometheus text-exposition body and return the unique
/// metric family names (lines starting with `# HELP <name>` or
/// directly the metric name on a sample line).
#[must_use]
pub fn parse_prom_families(body: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("# HELP ") {
            if let Some((name, _help)) = rest.split_once(' ') {
                out.insert(name.to_string());
                continue;
            }
        }
        if line.starts_with('#') {
            continue;
        }
        // Sample line : `<name>{labels} <value>` or `<name> <value>`.
        let name_end = line
            .bytes()
            .position(|b| !(b.is_ascii_alphanumeric() || b == b'_' || b == b':'))
            .unwrap_or(line.len());
        if name_end > 0 {
            out.insert(line[..name_end].to_string());
        }
    }
    out
}

#[allow(dead_code)]
const _BUDGET_NOTE: Duration = Duration::from_secs(10);

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_PROM: &str = r#"# HELP efe_l2_request_total Total L2 requests.
# TYPE efe_l2_request_total counter
efe_l2_request_total{status="success"} 1234
efe_l2_request_total{status="server_error"} 12
# HELP efe_controller_uptime_seconds Controller uptime.
# TYPE efe_controller_uptime_seconds gauge
efe_controller_uptime_seconds 3600.0
"#;

    #[test]
    fn parse_extracts_two_families() {
        let s = parse_prom_families(SAMPLE_PROM);
        assert!(s.contains("efe_l2_request_total"));
        assert!(s.contains("efe_controller_uptime_seconds"));
    }

    #[test]
    fn parse_ignores_blanks_and_comments() {
        let s = parse_prom_families("\n# unrelated\n# HELP a foo\n# TYPE a counter\na 1\n");
        assert!(s.contains("a"));
    }

    #[test]
    fn parse_empty_body_returns_empty() {
        let s = parse_prom_families("");
        assert!(s.is_empty());
    }

    #[test]
    fn parse_handles_namespaced_name() {
        let s = parse_prom_families("slo:L2_AVAILABILITY_5m 0.998\n");
        assert!(s.contains("slo:L2_AVAILABILITY_5m"));
    }
}
