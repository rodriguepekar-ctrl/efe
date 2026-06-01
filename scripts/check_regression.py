#!/usr/bin/env python3
"""scripts/check_regression.py — bench regression gate (Sprint D5).

Compares a candidate bench report against a frozen baseline and
fails (exit 1) if any common bench key has p95 latency > baseline ×
(1 + threshold × ci_coefficient). Drift between (warn_threshold,
threshold) is reported as a CI warning but does not fail.

Extracted from the inline `python3 - <<EOF` block in `bench.yml` v1
so it can be tested locally and reused by the D5 aggregate job.

Usage:
    python3 scripts/check_regression.py \\
        --baseline bench_baselines/v0.1.4/baseline.json \\
        --candidate target/bench-report-aggregated.json \\
        --threshold 0.10 \\
        --warn-threshold 0.05 \\
        --ci-coefficient 1.5

Exit codes:
    0  — no regression beyond the gated threshold (warnings allowed).
    1  — at least one regression beyond the gated threshold.
    2  — invocation error (missing files, malformed JSON, …).

Note on `ci_coefficient`:
    The reference baseline is captured on a fast host (Apple M3 Pro).
    GitHub-hosted ubuntu-latest runners are ~1.5× slower, so the
    effective threshold becomes `threshold × 1.5 = 15 %` rather than
    `10 %`. This avoids flaky failures caused purely by runner speed
    rather than real regressions.
"""

from __future__ import annotations
import argparse
import json
import sys
from typing import Any


def load_report(path: str, label: str) -> dict[str, Any]:
    try:
        with open(path, "r", encoding="utf-8") as fh:
            return json.load(fh)
    except FileNotFoundError:
        print(f"ERROR: {label} not found: {path}", file=sys.stderr)
        sys.exit(2)
    except json.JSONDecodeError as e:
        print(f"ERROR: {label} is not valid JSON ({path}): {e}", file=sys.stderr)
        sys.exit(2)


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--baseline", required=True)
    p.add_argument("--candidate", required=True)
    p.add_argument(
        "--threshold", type=float, default=0.10,
        help="Regression ratio above which the gate fails (default 0.10 = 10%%).",
    )
    p.add_argument(
        "--warn-threshold", type=float, default=0.05,
        help="Drift ratio above which we emit a warning (default 0.05 = 5%%).",
    )
    p.add_argument(
        "--ci-coefficient", type=float, default=1.0,
        help=(
            "Multiplier applied to thresholds to absorb host speed drift "
            "(default 1.0 ; CI ubuntu-latest typically 1.5)."
        ),
    )
    args = p.parse_args()

    ref = load_report(args.baseline, "baseline")
    cand = load_report(args.candidate, "candidate")

    ref_results = ref.get("results", {})
    cand_results = cand.get("results", {})

    if not ref_results:
        # Placeholder baselines (post-merge first runs) have empty
        # `results: {}`. We print INFO and exit clean — there's
        # nothing to compare against, but this is expected.
        print("INFO: baseline has no results yet (placeholder?). Skipping gate.")
        return 0

    eff_threshold = args.threshold * args.ci_coefficient
    eff_warn = args.warn_threshold * args.ci_coefficient

    # Sprint D6 audit fix : the regression check must distinguish
    # "lower is better" (latency / memory : `lte`) from
    # "higher is better" (accuracy / F1 / confidence : `gte`).
    # For `gte` metrics, a regression means `cand < ref` — the
    # opposite sign convention.
    #
    # We detect `gte` semantics by bench-key prefix : everything
    # starting with `eval_accuracy::` is a ratio where higher is
    # better. Other ratios produced by future SLO families would
    # need to be added to this list. The prefix check is preferred
    # over re-importing SLO_DEFINITIONS to keep this script
    # self-contained.
    GTE_PREFIXES = ("eval_accuracy::top1", "eval_accuracy::macro_f1",
                    "eval_accuracy::mean_conf_correct")

    regressions: list[tuple[str, float, float, float]] = []
    warnings: list[tuple[str, float, float, float]] = []
    skipped_keys = 0

    for key, ref_val in ref_results.items():
        cand_val = cand_results.get(key)
        if cand_val is None:
            skipped_keys += 1
            continue
        ref_p95 = ref_val.get("approx_p95_ns")
        cand_p95 = cand_val.get("approx_p95_ns")
        if ref_p95 in (None, 0) or cand_p95 in (None, 0):
            skipped_keys += 1
            continue

        # Compute the regression delta with the right sign convention.
        if any(key.startswith(p) for p in GTE_PREFIXES):
            # gte metric : regression means cand DROPPED below ref.
            # We invert the sign so the threshold check works the same.
            delta = (ref_p95 - cand_p95) / ref_p95
        else:
            # lte metric (default) : regression means cand exceeded ref.
            delta = (cand_p95 - ref_p95) / ref_p95

        if delta > eff_threshold:
            regressions.append((key, ref_p95, cand_p95, delta))
        elif delta > eff_warn:
            warnings.append((key, ref_p95, cand_p95, delta))

    print(
        f"==> Compared {len(ref_results)} baseline keys against candidate ; "
        f"skipped {skipped_keys} (not in candidate)."
    )
    print(
        f"    eff_threshold = {eff_threshold * 100:.1f}%  "
        f"eff_warn = {eff_warn * 100:.1f}%"
    )

    if regressions:
        print("::error::Bench regressions detected:")
        for key, r, c, d in regressions:
            # Use GitHub Actions error annotation ; the colon-format
            # `::error::` causes the runner to highlight the line
            # in the Checks UI.
            print(f"  {key}: ref={r:.0f}ns cand={c:.0f}ns delta=+{d * 100:.1f}%")

    if warnings:
        print(f"::warning::Bench drift > {eff_warn * 100:.1f}% (review):")
        for key, r, c, d in warnings:
            print(f"  {key}: ref={r:.0f}ns cand={c:.0f}ns delta=+{d * 100:.1f}%")

    if regressions:
        return 1

    print(f"OK — no regression > {eff_threshold * 100:.1f}% detected.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
