//! PR #21 § 12.3 + AC22 — PR comment formatter.
//!
//! Emits a Markdown table for the GitHub Actions `pull-request-comment`
//! action.  AC22 : every scenario field is HTML-escaped before
//! interpolation (T89 mitigation).

use crate::compare::{ComparisonReport, MetricStatus};

/// Build a markdown-formatted PR comment for one or more comparison reports.
#[must_use]
pub fn build_pr_comment(reports: &[ComparisonReport], harness_version: &str) -> String {
    let mut out = String::new();
    out.push_str("## 🔬 Bench results — `efe-bench-harness@");
    out.push_str(&html_escape(harness_version));
    out.push_str("`\n\n");
    out.push_str("| Scenario | Verdict | p50 (ms) | p95 (ms) | p99 (ms) | error_rate | throughput (qps) |\n");
    out.push_str("|---|---|---|---|---|---|---|\n");
    for r in reports {
        out.push_str(&format_row(r));
        out.push('\n');
    }
    out.push_str("\n🔗 Full results : [bench/results/](/bench/results/)\n");
    out.push_str("🔗 Methodology : [bench/README.md](/bench/README.md)\n");
    out
}

fn format_row(r: &ComparisonReport) -> String {
    use crate::scenario::ToleranceMetric::*;

    let v = r.overall;
    let glyph = v.glyph();
    let verdict_label = format!("{} {}", glyph, v.as_str());

    let p50 = fmt_cell(r, P50Ms);
    let p95 = fmt_cell(r, P95Ms);
    let p99 = fmt_cell(r, P99Ms);
    let err = fmt_cell(r, ErrorRate);
    let tput = fmt_cell(r, ThroughputQps);

    format!(
        "| `{}` | {} | {} | {} | {} | {} | {} |",
        html_escape(&r.scenario_id),
        verdict_label,
        p50,
        p95,
        p99,
        err,
        tput
    )
}

fn fmt_cell(r: &ComparisonReport, m: crate::scenario::ToleranceMetric) -> String {
    let Some(row) = r.rows.get(&m) else {
        return "—".into();
    };
    let actual = row.actual.map(|v| format_value(m, v)).unwrap_or_else(|| "—".into());
    let delta = row
        .delta_pct
        .map(|p| format!(" ({p:+.1}%)"))
        .unwrap_or_default();
    let status = match row.status {
        MetricStatus::Pass => "",
        MetricStatus::Warn => " ⚠",
        MetricStatus::Fail => " ❌",
        MetricStatus::NoBaseline => " ℹ",
    };
    format!("{actual}{delta}{status}")
}

fn format_value(m: crate::scenario::ToleranceMetric, v: f64) -> String {
    use crate::scenario::ToleranceMetric::*;
    match m {
        ErrorRate => format!("{:.3}%", v * 100.0),
        ThroughputQps => format!("{v:.1}"),
        _ => format!("{v:.1}"),
    }
}

/// Minimal HTML-escape (T89 / AC22).  Targets the characters that
/// would break GitHub-flavoured markdown or inject HTML.
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '|' => out.push_str("&#124;"),
            '`' => out.push_str("&#96;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compare::Verdict;
    use std::collections::BTreeMap;

    fn make_report(id: &str, verdict: Verdict) -> ComparisonReport {
        ComparisonReport {
            scenario_id: id.into(),
            baseline_chart_version: Some("0.6.0".into()),
            baseline_git_commit: Some("abc".into()),
            rows: BTreeMap::new(),
            overall: verdict,
        }
    }

    #[test]
    fn html_escape_neutralises_pipe_and_backtick() {
        let evil = "Sxx|<script>alert('x')</script>`";
        let safe = html_escape(evil);
        assert!(!safe.contains('|'));
        assert!(!safe.contains('<'));
        assert!(!safe.contains('`'));
    }

    #[test]
    fn build_pr_comment_includes_verdict_glyph() {
        let r1 = make_report("S01_test", Verdict::Pass);
        let r2 = make_report("S04_chaos", Verdict::Fail);
        let comment = build_pr_comment(&[r1, r2], "0.1.0");
        assert!(comment.contains("✅ pass"));
        assert!(comment.contains("❌ fail"));
    }

    #[test]
    fn build_pr_comment_html_escapes_harness_version() {
        let evil = "0.1.0<img src=x>";
        let comment = build_pr_comment(&[], evil);
        assert!(!comment.contains("<img"));
        assert!(comment.contains("&lt;img"));
    }
}
