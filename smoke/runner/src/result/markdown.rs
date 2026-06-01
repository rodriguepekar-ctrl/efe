//! PR #23 § 9 + AD27 — PR-comment markdown formatter.  HTML-escaped
//! (T89 from PR #21 — anti-injection).

use crate::checks::{CheckVerdict, SmokeCheckId};
use crate::result::SmokeReport;

/// Render a markdown table summarising the smoke run.
#[must_use]
pub fn build_pr_comment(report: &SmokeReport) -> String {
    let mut out = String::new();
    let overall_glyph = match report.overall_verdict {
        CheckVerdict::Pass => "✅",
        CheckVerdict::Fail => "❌",
    };
    out.push_str(&format!(
        "## {} Smoke — `efe-smoke@{}` — chart `{}`\n\n",
        overall_glyph,
        html_escape(&report.harness_version),
        html_escape(&report.chart_version),
    ));
    out.push_str(&format!(
        "Duration : `{:.1}s` · Run ID : `{}` · Git SHA : `{}`\n\n",
        report.duration_s,
        html_escape(&report.run_id),
        html_escape(&report.git_sha),
    ));

    out.push_str("| Check | Verdict | Duration | Budget |\n");
    out.push_str("|---|---|---|---|\n");
    for id in SmokeCheckId::all() {
        let row = report.checks.get(id.short());
        let (glyph, verdict_str, dur, budget) = match row {
            Some(r) => {
                let g = match r.verdict {
                    CheckVerdict::Pass => "✅",
                    CheckVerdict::Fail => "❌",
                };
                (g, r.verdict.as_str(), format!("{:.1}s", r.duration_s), format!("{:.0}s", r.budget_s))
            }
            None => ("⏭", "skipped", "—".to_string(), "—".to_string()),
        };
        out.push_str(&format!(
            "| `{}` | {} {} | {} | {} |\n",
            html_escape(id.as_str()),
            glyph,
            verdict_str,
            dur,
            budget,
        ));
    }
    if matches!(report.overall_verdict, CheckVerdict::Fail) {
        out.push_str("\n### Failing checks\n\n");
        for id in report.failing_ids() {
            if let Some(r) = report.checks.get(id.short()) {
                out.push_str(&format!(
                    "- **{}** — {}\n",
                    html_escape(id.as_str()),
                    html_escape(&r.error_summary),
                ));
            }
        }
    }
    out.push_str(&format!(
        "\n_Stability tier : v{} — STABILITY_CONTRACT § 23._\n",
        report.stability_contract_version,
    ));
    out
}

/// Minimal HTML-escape (AD27 + PR #21 T89).
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            '|' => out.push_str("&#x7c;"),
            '`' => out.push_str("&#x60;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::CheckResult;
    use std::time::Duration;

    fn sample_report(verdict: CheckVerdict) -> SmokeReport {
        let mut results = Vec::new();
        for id in SmokeCheckId::all() {
            results.push(CheckResult::pass(id, Duration::from_secs(5), serde_json::json!({})));
        }
        if matches!(verdict, CheckVerdict::Fail) {
            results[3] = CheckResult::fail(
                SmokeCheckId::S4,
                Duration::from_secs(22),
                "2 dropped requests".into(),
                serde_json::json!({"dropped": 2}),
            );
        }
        SmokeReport::build(
            "r-test".into(),
            "0.6.0".into(),
            "abc1234".into(),
            Duration::from_secs(120),
            verdict,
            results,
        )
    }

    #[test]
    fn pass_report_has_green_glyph_in_header() {
        let s = build_pr_comment(&sample_report(CheckVerdict::Pass));
        assert!(s.contains("✅ Smoke"));
        assert!(s.contains("S1_boot"));
        assert!(s.contains("S9_shutdown"));
    }

    #[test]
    fn fail_report_lists_failing_checks() {
        let s = build_pr_comment(&sample_report(CheckVerdict::Fail));
        assert!(s.contains("❌ Smoke"));
        assert!(s.contains("Failing checks"));
        assert!(s.contains("S4_mtls_rotation"));
    }

    #[test]
    fn pr_comment_html_escapes_chart_version() {
        let mut report = sample_report(CheckVerdict::Pass);
        report.chart_version = "0.6.0<img src=x>".into();
        let s = build_pr_comment(&report);
        assert!(!s.contains("<img"));
        assert!(s.contains("&lt;img"));
    }

    #[test]
    fn pr_comment_carries_stability_tier() {
        let s = build_pr_comment(&sample_report(CheckVerdict::Pass));
        assert!(s.contains("v1.5"));
        assert!(s.contains("§ 23"));
    }
}
