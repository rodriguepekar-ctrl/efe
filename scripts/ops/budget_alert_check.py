#!/usr/bin/env python3
"""
Sprint Φ-D-OPS (D98) — Budget alert bench.

Reads the chargeback JSON produced by `chargeback_compute.py` and the
per-tenant monthly budgets from `bench_baselines/v0.9.0/budgets.yaml`
(= optional file ; defaults to demo budgets), then asserts that no
tenant has spent more than `--threshold` × their monthly budget.

Emits `{slo_id: count}` consumable by the consolidator.

Cf. `docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md` § 12.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Dict, List, Optional

try:
    import yaml  # type: ignore
except ImportError:
    yaml = None  # type: ignore


DEFAULT_BUDGETS = {
    "acme":    1500.0,
    "globex":   300.0,
    "initech":  800.0,
}


def main(argv: Optional[List[str]] = None) -> int:
    p = argparse.ArgumentParser(description="Per-tenant budget bench")
    p.add_argument("--chargeback", type=Path,
                   default=Path("target/ops/chargeback.json"))
    p.add_argument("--budgets", type=Path,
                   default=Path("bench_baselines/v0.9.0/budgets.yaml"),
                   help="YAML map tenant → monthly USD budget.")
    p.add_argument("--threshold", type=float, default=1.10,
                   help="Budget overrun multiplier that flips a row to FAIL.")
    p.add_argument("--out", type=Path,
                   default=Path("target/ops/measures.budget.json"))
    args = p.parse_args(argv)

    if not args.chargeback.exists():
        sys.stderr.write(f"missing chargeback file: {args.chargeback}\n")
        return 2
    chargeback = json.loads(args.chargeback.read_text(encoding="utf-8"))

    if args.budgets.exists() and yaml is not None:
        budgets = yaml.safe_load(args.budgets.read_text(encoding="utf-8"))
        if not isinstance(budgets, dict):
            sys.stderr.write(f"budgets file malformed : {args.budgets}\n")
            return 2
    else:
        budgets = DEFAULT_BUDGETS

    over_count = 0
    details: List[Dict[str, object]] = []
    for row in chargeback.get("rows", []):
        tenant = row["tenant"]
        total = row["total_usd"]
        budget = float(budgets.get(tenant, 0.0))
        if budget <= 0:
            ratio = float("inf")
        else:
            ratio = total / budget
        flagged = ratio > args.threshold
        if flagged:
            over_count += 1
        details.append({
            "tenant":   tenant,
            "spent_usd": total,
            "budget_usd": budget,
            "ratio":    None if ratio == float("inf") else round(ratio, 3),
            "over":     flagged,
        })
        marker = "⚠" if flagged else "✓"
        print(f"  {marker} {tenant:<10} USD {total:>9.2f} / "
              f"{budget:>9.2f}  ratio={'∞' if ratio==float('inf') else f'{ratio:.2f}'}")

    payload = {
        "SLO-OPS-BUDGET-OVERRUN": over_count,
        "_budget_meta": {
            "threshold": args.threshold,
            "details":   details,
        },
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(payload, indent=2), encoding="utf-8")
    print(f"\nbudget bench → {args.out}  (over_count={over_count})")
    return 0 if over_count == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
