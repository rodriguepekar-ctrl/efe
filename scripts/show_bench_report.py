#!/usr/bin/env python3
"""scripts/show_bench_report.py — pretty-print SLO compliance.

Sprint D5 extraction. Replaces the inline `python3 -c` block in
`bench.yml` v1, so the formatting can be tested locally and tweaked
without touching the workflow YAML.

Reads a consolidated bench report (output of
`parse_criterion_report.py` or `aggregate_bench_reports.py`) and
prints a human-readable summary :
- Header with host metadata.
- One row per SLO entry : ID, name, target, measured, status.
- A footer counter (pass / warn / fail / skipped).

Status legend:
    PASS     measured ≤ target
    FAIL     measured >  target
    SKIPPED  bench not measured (e.g., BLAS-only key on a no-BLAS run)

Usage:
    python3 scripts/show_bench_report.py target/bench-report-*.json

Exit codes:
    0  — no FAIL.
    1  — at least one FAIL (gate signal for CI).
    2  — invocation / read error.
"""

from __future__ import annotations
import argparse
import json
import math
import sys
from typing import Any


def fmt_ns(ns: float | int | None) -> str:
    """Pretty-format a duration in ns as the most readable unit."""
    if ns is None or (isinstance(ns, float) and math.isnan(ns)):
        return "    n/a"
    n = float(ns)
    if n < 1e3:
        return f"{n:7.1f} ns"
    if n < 1e6:
        return f"{n / 1e3:7.2f} µs"
    if n < 1e9:
        return f"{n / 1e6:7.2f} ms"
    return f"{n / 1e9:7.2f}  s"


def fmt_acc(scaled_ns: float | int | None) -> str:
    """Sprint D6 — format an accuracy value back to its 0..1 ratio.

    Accuracy SLO are scaled by 1e9 in the bench keys. Reverse the
    scaling here so the operator reads `0.825` not `825.00 ms`.
    """
    if scaled_ns is None or (isinstance(scaled_ns, float) and math.isnan(scaled_ns)):
        return "    n/a"
    return f"{float(scaled_ns) / 1e9:7.3f}  "


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("report", help="Path to the bench-report-*.json file")
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

    # Header
    host = report.get("host", {})
    tool = report.get("toolchain", {})
    git = report.get("git", {})
    print(f"==> Bench report: {args.report}")
    print(
        f"    host = {host.get('os', '?')} "
        f"({host.get('cpu', '?')}, {host.get('ncpu', '?')} cores, "
        f"{host.get('ram_gb', '?')} GB)"
    )
    print(
        f"    rustc = {tool.get('rustc', '?')} ; "
        f"BLAS = {tool.get('blas_backend', 'none')} ; "
        f"sha = {git.get('sha', '?')}"
    )
    print()

    slo_compliance: dict[str, dict[str, Any]] = report.get("slo_compliance", {})
    if not slo_compliance:
        print("    (no SLO compliance data in report)")
        return 0

    # Compute column widths
    name_width = max(
        (len(e.get("name", "")) for e in slo_compliance.values()),
        default=20,
    )
    name_width = max(name_width, 24)

    print(
        f"    {'ID':<11} {'Name':<{name_width}} "
        f"{'Target':>11} {'Measured':>11} {'Status':>8}"
    )
    print(f"    {'-' * (11 + name_width + 36)}")

    counts = {"PASS": 0, "FAIL": 0, "SKIPPED": 0, "WARN": 0}

    # Stable order : sort by SLO ID lexicographically. The naming
    # convention (SLO-1, SLO-D2-1, SLO-D3-1B, SLO-D4-T1, SLO-RD1-3,
    # …) gives a sensible group ordering by sprint.
    for slo_id in sorted(slo_compliance.keys()):
        entry = slo_compliance[slo_id]
        name = entry.get("name", "?")
        target = entry.get("target_ns")
        measured = entry.get("measured_ns")
        skipped = entry.get("skipped", False)
        passed = entry.get("passed")
        comparator = entry.get("comparator", "lte")

        if skipped or measured is None:
            status = "SKIPPED"
            counts["SKIPPED"] += 1
        elif passed is True:
            status = "PASS"
            counts["PASS"] += 1
        elif passed is False:
            status = "FAIL"
            counts["FAIL"] += 1
        else:
            status = "WARN"
            counts["WARN"] += 1

        # D6 : SLO with `display_as: "ratio"` carry scaled (×1e9)
        # values in [0, 1] ratio space — display as fraction. Fallback
        # to `comparator == "gte"` for SLO authored before the
        # `display_as` field was introduced (parser audit fix).
        display_as = entry.get("display_as", "duration")
        if display_as == "ratio" or comparator == "gte":
            target_str = fmt_acc(target)
            measured_str = fmt_acc(measured)
        else:
            target_str = fmt_ns(target)
            measured_str = fmt_ns(measured)

        print(
            f"    {slo_id:<11} {name:<{name_width}} "
            f"{target_str:>11} {measured_str:>11} "
            f"{status:>8}"
        )

    print()
    print(
        f"    Summary : "
        f"PASS={counts['PASS']}  FAIL={counts['FAIL']}  "
        f"SKIPPED={counts['SKIPPED']}  WARN={counts['WARN']}"
    )

    return 1 if counts["FAIL"] > 0 else 0


if __name__ == "__main__":
    sys.exit(main())
