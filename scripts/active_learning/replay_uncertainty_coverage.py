#!/usr/bin/env python3
"""Verify 100 % of production calls have uncertainty_score.

Sprint **Φ-D-AL Completion (D75)** — artefact #07.
"""

from __future__ import annotations
import argparse, json, sys
from pathlib import Path


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--calls-dump", type=Path,
                    help="JSON dump of production calls (= one per row)")
    ap.add_argument("--out", type=Path,
                    default=Path("target/active_learning/uncertainty_coverage.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if not args.calls_dump:
        sys.stderr.write("WARN : --calls-dump required ; emitting SKIPPED.\n")
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-AL-UNCERTAINTY-COVERAGE": {
                    "skip_reason": "Live call dump required (DT-AL-CALLS-DUMP)"
                }
            }
        }, indent=2) + "\n")
        return 0

    calls = json.loads(args.calls_dump.read_text())
    if isinstance(calls, dict):
        calls = calls.get("calls", [])
    n_total = len(calls)
    n_with_score = sum(1 for c in calls if c.get("uncertainty_score") is not None)
    coverage = n_with_score / max(n_total, 1)
    passed = coverage == 1.0

    print(f"Uncertainty coverage : {n_with_score}/{n_total} = {coverage:.2%}",
          file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-AL-UNCERTAINTY-COVERAGE": {
                "measured_pass_count": 1 if passed else 0,
                "measured_expected_count": 1,
                "n_calls": n_total,
                "n_with_score": n_with_score,
                "coverage": coverage,
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
