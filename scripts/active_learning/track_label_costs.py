#!/usr/bin/env python3
"""Track Active Learning per-label cost.

Sprint **Φ-D-AL Completion (D76)** — artefact #09.

Computes p99 cost per gold label. SLO ≤ $0.01.
"""

from __future__ import annotations
import argparse, json, random, sys
from pathlib import Path


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--labels-cost-dump", type=Path)
    ap.add_argument("--out", type=Path,
                    default=Path("target/active_learning/label_costs.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if not args.labels_cost_dump:
        sys.stderr.write("WARN : --labels-cost-dump required ; emitting SKIPPED.\n")
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-AL-COST-PER-LABEL": {
                    "skip_reason": "Live label cost dump required (DT-AL-COST-DUMP)"
                }
            }
        }, indent=2) + "\n")
        return 0

    costs = json.loads(args.labels_cost_dump.read_text())
    if isinstance(costs, dict):
        costs = costs.get("costs", [])
    costs_sorted = sorted(costs)
    if not costs_sorted:
        print("No labels.", file=sys.stderr)
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-AL-COST-PER-LABEL": {"skip_reason": "Empty cost data"}
            }
        }, indent=2) + "\n")
        return 0
    p99_idx = max(0, int(0.99 * len(costs_sorted)) - 1)
    p99 = costs_sorted[p99_idx]
    passed = p99 <= 0.01
    print(f"Cost per label : n={len(costs)}, p99=${p99:.4f}", file=sys.stderr)
    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-AL-COST-PER-LABEL": {
                "measured_ratio": p99,
                "n_labels": len(costs),
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
