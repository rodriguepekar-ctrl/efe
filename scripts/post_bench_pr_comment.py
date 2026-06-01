#!/usr/bin/env python3
"""scripts/post_bench_pr_comment.py — render Markdown PR comment.

Sprint D5 addition. Renders a bench report as a Markdown comment
suitable for `gh pr comment` and (optionally) posts it to the PR.

The Markdown layout:
- Header (SHA, host, mode, BLAS).
- SLO compliance counter (PASS / WARN / FAIL / SKIPPED) as a table.
- Top headline metrics (the 4 SLO with the highest target value,
  i.e. the most representative ones — small/medium/large e2e).
- Optional regression block when --baseline is provided.
- Footer with link to the workflow run.

Posting is only attempted when --pr is set ; otherwise the rendered
Markdown is printed to stdout (useful for local preview).

Usage (preview):
    python3 scripts/post_bench_pr_comment.py \\
        --report target/bench-report-aggregated.json

Usage (CI):
    python3 scripts/post_bench_pr_comment.py \\
        --report target/bench-report-aggregated.json \\
        --pr 1234 \\
        --baseline bench_baselines/v0.1.4/baseline.json \\
        --workflow-url https://github.com/.../actions/runs/...
"""

from __future__ import annotations
import argparse
import json
import math
import os
import subprocess
import sys
from typing import Any


def fmt_value(entry: dict[str, Any], value: float | int | None) -> str:
    """Sprint D6 — pretty-format an SLO value, distinguishing :

    * `display_as: "ratio"` SLO (accuracy / F1 / etc.) : the underlying
      number is `ratio × 1e9` — render as a 0..1 fraction.
    * Default : render as a duration in ns/µs/ms/s.
    """
    if value is None or (isinstance(value, float) and math.isnan(value)):
        return "n/a"
    display_as = entry.get("display_as", "duration")
    comparator = entry.get("comparator", "lte")
    if display_as == "ratio" or comparator == "gte":
        return f"{float(value) / 1e9:.3f}"
    return _fmt_duration_ns(value)


def _fmt_duration_ns(ns: float | int | None) -> str:
    """Pretty-format a duration in ns as the most readable unit."""
    if ns is None or (isinstance(ns, float) and math.isnan(ns)):
        return "n/a"
    n = float(ns)
    if n < 1e3:
        return f"{n:.0f} ns"
    if n < 1e6:
        return f"{n / 1e3:.1f} µs"
    if n < 1e9:
        return f"{n / 1e6:.1f} ms"
    return f"{n / 1e9:.2f} s"


def status_emoji(entry: dict[str, Any]) -> str:
    if entry.get("skipped"):
        return "⏭"
    p = entry.get("passed")
    if p is True:
        return "✅"
    if p is False:
        return "❌"
    return "⚠️"


def render_compliance_summary(report: dict[str, Any]) -> str:
    """Counter table : PASS / WARN / FAIL / SKIPPED."""
    counts = {"PASS": 0, "WARN": 0, "FAIL": 0, "SKIPPED": 0}
    for entry in report.get("slo_compliance", {}).values():
        if entry.get("skipped"):
            counts["SKIPPED"] += 1
        elif entry.get("passed") is True:
            counts["PASS"] += 1
        elif entry.get("passed") is False:
            counts["FAIL"] += 1
        else:
            counts["WARN"] += 1
    rows = [
        f"| ✅ Pass | {counts['PASS']} |",
        f"| ⚠️ Warn | {counts['WARN']} |",
        f"| ❌ Fail | {counts['FAIL']} |",
        f"| ⏭ Skipped | {counts['SKIPPED']} |",
    ]
    return "| Status | Count |\n|--------|------:|\n" + "\n".join(rows)


def render_headline_metrics(report: dict[str, Any], top_n: int = 6) -> str:
    """Pick the `top_n` most informative SLO entries.

    Heuristic : highest target ⇒ highest-impact (and thus the keys
    we most want to see at-a-glance). Skipped entries are kept at
    the bottom (still useful to flag missing measurements).
    """
    rows: list[tuple[str, dict[str, Any]]] = list(
        report.get("slo_compliance", {}).items()
    )
    if not rows:
        return "_(no SLO entries to display)_"

    # Sort: failures first (most actionable), then by target descending.
    def sort_key(item: tuple[str, dict[str, Any]]) -> tuple[int, float]:
        slo_id, e = item
        if e.get("skipped"):
            rank = 3
        elif e.get("passed") is False:
            rank = 0
        elif e.get("passed") is True:
            rank = 2
        else:
            rank = 1
        target = -float(e.get("target_ns") or 0)
        return (rank, target)

    rows.sort(key=sort_key)
    rows = rows[:top_n]

    lines = [
        "| SLO | Name | Measured | Target | Status |",
        "|-----|------|---------:|-------:|:------:|",
    ]
    for slo_id, e in rows:
        lines.append(
            f"| `{slo_id}` | {e.get('name', '?')} "
            f"| {fmt_value(e, e.get('measured_ns'))} "
            f"| {fmt_value(e, e.get('target_ns'))} "
            f"| {status_emoji(e)} |"
        )
    return "\n".join(lines)


def render_regression_block(
    cand: dict[str, Any],
    baseline_path: str,
    threshold: float,
    warn_threshold: float,
    ci_coefficient: float,
) -> str:
    """Compare candidate to baseline, render a Markdown block."""
    if not baseline_path or not os.path.isfile(baseline_path):
        return "_(no baseline for comparison)_"
    try:
        with open(baseline_path, "r", encoding="utf-8") as fh:
            ref = json.load(fh)
    except json.JSONDecodeError as e:
        return f"_(baseline parse error : {e})_"

    ref_results = ref.get("results", {})
    cand_results = cand.get("results", {})
    if not ref_results:
        return "_(baseline has no results yet — placeholder)_"

    eff_threshold = threshold * ci_coefficient
    eff_warn = warn_threshold * ci_coefficient

    regressions: list[tuple[str, float, float, float]] = []
    warnings: list[tuple[str, float, float, float]] = []
    for key, ref_val in ref_results.items():
        cand_val = cand_results.get(key)
        if cand_val is None:
            continue
        ref_p95 = ref_val.get("approx_p95_ns")
        cand_p95 = cand_val.get("approx_p95_ns")
        if ref_p95 in (None, 0) or cand_p95 in (None, 0):
            continue
        delta = (cand_p95 - ref_p95) / ref_p95
        if delta > eff_threshold:
            regressions.append((key, ref_p95, cand_p95, delta))
        elif delta > eff_warn:
            warnings.append((key, ref_p95, cand_p95, delta))

    if not regressions and not warnings:
        return f"🟢 No regression > {eff_threshold * 100:.1f} % vs baseline ({os.path.basename(os.path.dirname(baseline_path))})."

    out: list[str] = []
    if regressions:
        out.append(f"❌ **{len(regressions)} regression(s) > {eff_threshold * 100:.1f} %** :")
        out.append("")
        out.append("| Bench key | Baseline (p95) | Candidate (p95) | Δ |")
        out.append("|-----------|---------------:|----------------:|---:|")
        for key, r, c, d in regressions:
            out.append(
                f"| `{key}` | {_fmt_duration_ns(r)} | {_fmt_duration_ns(c)} | **+{d * 100:.1f} %** |"
            )
        out.append("")
    if warnings:
        out.append(f"⚠️ **{len(warnings)} drift > {eff_warn * 100:.1f} %** (review) :")
        out.append("")
        out.append("| Bench key | Baseline (p95) | Candidate (p95) | Δ |")
        out.append("|-----------|---------------:|----------------:|---:|")
        for key, r, c, d in warnings:
            out.append(
                f"| `{key}` | {_fmt_duration_ns(r)} | {_fmt_duration_ns(c)} | +{d * 100:.1f} % |"
            )
    return "\n".join(out)


def render_comment(
    report: dict[str, Any],
    baseline_path: str | None,
    threshold: float,
    warn_threshold: float,
    ci_coefficient: float,
    workflow_url: str | None,
) -> str:
    git = report.get("git", {})
    host = report.get("host", {})
    tool = report.get("toolchain", {})

    parts: list[str] = []
    parts.append(f"## 📊 EFE Bench Report — `{git.get('sha', 'unknown')}`")
    parts.append("")
    parts.append(
        f"**Host** : {host.get('os', '?')} "
        f"({host.get('cpu', '?')}, {host.get('ncpu', '?')} cores, "
        f"{host.get('ram_gb', '?')} GB)  "
    )
    parts.append(
        f"**Toolchain** : `{tool.get('rustc', '?')}`  "
    )
    parts.append(
        f"**BLAS** : `{tool.get('blas_backend', 'none')}`  "
    )
    parts.append(f"**Branch** : `{git.get('branch', 'unknown')}`")
    parts.append("")
    parts.append("### SLO compliance summary")
    parts.append("")
    parts.append(render_compliance_summary(report))
    parts.append("")
    parts.append("### Headline metrics")
    parts.append("")
    parts.append(render_headline_metrics(report))
    parts.append("")
    if baseline_path:
        baseline_label = os.path.basename(os.path.dirname(baseline_path))
        parts.append(f"### Regression vs baseline `{baseline_label}`")
        parts.append("")
        parts.append(
            render_regression_block(
                report,
                baseline_path,
                threshold,
                warn_threshold,
                ci_coefficient,
            )
        )
        parts.append("")
    if workflow_url:
        parts.append(f"[Full report (artefact)]({workflow_url})")
        parts.append("")
    parts.append(
        "_Generated by `scripts/post_bench_pr_comment.py` "
        "(Sprint D5 unified bench orchestration)._"
    )
    return "\n".join(parts)


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--report", required=True)
    p.add_argument("--pr",
                   help="PR number to post the comment on. If absent, "
                        "renders to stdout only (preview mode).")
    p.add_argument("--baseline",
                   help="Baseline report for regression comparison.")
    p.add_argument("--threshold", type=float, default=0.10)
    p.add_argument("--warn-threshold", type=float, default=0.05)
    p.add_argument("--ci-coefficient", type=float, default=1.0)
    p.add_argument("--workflow-url",
                   help="URL to the GitHub Actions workflow run.")
    args = p.parse_args()

    try:
        with open(args.report, "r", encoding="utf-8") as fh:
            report = json.load(fh)
    except FileNotFoundError:
        print(f"ERROR: report not found: {args.report}", file=sys.stderr)
        return 2
    except json.JSONDecodeError as e:
        print(f"ERROR: report is not valid JSON ({args.report}): {e}", file=sys.stderr)
        return 2

    body = render_comment(
        report,
        args.baseline,
        args.threshold,
        args.warn_threshold,
        args.ci_coefficient,
        args.workflow_url,
    )

    if not args.pr:
        # Preview mode : print to stdout.
        print(body)
        return 0

    # Post via gh CLI. The runner has gh installed and authenticated
    # via GITHUB_TOKEN. We pass the body via stdin to avoid argv
    # length limits.
    cmd = ["gh", "pr", "comment", args.pr, "--body-file", "-"]
    proc = subprocess.run(
        cmd,
        input=body,
        text=True,
        capture_output=True,
    )
    if proc.returncode != 0:
        print(
            f"ERROR: gh pr comment failed (rc={proc.returncode})",
            file=sys.stderr,
        )
        print(proc.stderr, file=sys.stderr)
        return proc.returncode
    print(f"==> Posted comment to PR #{args.pr}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
