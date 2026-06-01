#!/usr/bin/env python3
"""
Sprint Φ-D-OPS (D98) — Cost-per-tenant bench.

Computes whether per-tenant cost stays under the platform's unit-
economics envelope :

   SLO-OPS-COST-PER-TENANT  (= USD 600 / tenant / month, P95)

The bench feeds back into the master SLO consolidator. Failure flags
a Sev2 incident — likely cause is runaway LLM premium tokens, missing
catalog hit-rate cache, or saga retry storm.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import List, Optional


def _quantile(values: List[float], q: float) -> float:
    if not values:
        return 0.0
    values = sorted(values)
    idx = max(0, min(len(values) - 1, int(round(len(values) * q)) - 1))
    return values[idx]


def main(argv: Optional[List[str]] = None) -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--chargeback", type=Path,
                   default=Path("target/ops/chargeback.json"))
    p.add_argument("--p95-cap-usd", type=float, default=600.0)
    p.add_argument("--out", type=Path,
                   default=Path("target/ops/measures.cost_per_tenant.json"))
    args = p.parse_args(argv)

    cb = json.loads(args.chargeback.read_text(encoding="utf-8"))
    totals = [row["total_usd"] for row in cb.get("rows", [])]
    if not totals:
        sys.stderr.write("no chargeback rows ; skipping bench\n")
        return 0

    p50 = _quantile(totals, 0.50)
    p95 = _quantile(totals, 0.95)
    p99 = _quantile(totals, 0.99)
    median_payload = {
        "SLO-OPS-COST-PER-TENANT": p95,
        "_meta": {
            "tenant_count": len(totals),
            "p50_usd":      p50,
            "p95_usd":      p95,
            "p99_usd":      p99,
            "cap_usd":      args.p95_cap_usd,
            "pass":         p95 <= args.p95_cap_usd,
        },
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(median_payload, indent=2), encoding="utf-8")
    print(f"cost-per-tenant → {args.out}")
    print(f"  p50={p50:.2f}  p95={p95:.2f}  p99={p99:.2f}  "
          f"cap={args.p95_cap_usd:.2f}  "
          f"{'PASS' if p95 <= args.p95_cap_usd else 'FAIL'}")
    return 0 if p95 <= args.p95_cap_usd else 1


if __name__ == "__main__":
    sys.exit(main())
