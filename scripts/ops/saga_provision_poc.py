#!/usr/bin/env python3
"""
Sprint Φ-D-OPS (D96) — Saga bench : POC tenant onboarding.

Simulates the 6-step onboarding saga (signup_intent → active) end-to-
end against a stub backend and reports per-step latency + total
provisioning time. Used as the source-of-truth measurement for
`SLO-OPS-PROVISION-P95` (≤ 720 s budget).

Outputs JSON `{slo_id: value}` consumable by
`scripts/ops/consolidate_all_slos.py`.

Cf. `docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md` § 7.
"""

from __future__ import annotations

import argparse
import json
import random
import statistics
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, List, Optional


# Cap each step at its empirical p95 plus 50 % safety margin.
STEP_BUDGETS_S: Dict[str, float] = {
    "provision_pg":         180.0,   # PG provisioning by far slowest
    "provision_redis":       60.0,
    "migrate_schema":        45.0,
    "seed_fixtures":         60.0,
    "request_attestation":   90.0,   # human-in-the-loop, fastest if SE pre-warmed
    "mark_active":            5.0,
}


@dataclass
class StepResult:
    name: str
    duration_s: float
    status: str    # ok | failed | timeout
    error: Optional[str] = None


@dataclass
class SagaResult:
    total_s: float
    steps: List[StepResult]
    completed: bool


def _simulate_step(name: str, rng: random.Random,
                   fail_rate: float = 0.0) -> StepResult:
    """Model each step as a log-normal latency with optional failure."""
    budget = STEP_BUDGETS_S[name]
    # Median ≈ budget × 0.35 ; long tail rare.
    median = budget * 0.35
    sigma = 0.45
    duration = max(0.1, rng.lognormvariate(0.0, sigma) * median / 1.0)
    duration = min(duration, budget * 2)
    if rng.random() < fail_rate:
        return StepResult(name=name, duration_s=duration,
                          status="failed",
                          error=f"simulated_{name}_failure")
    if duration > budget:
        return StepResult(name=name, duration_s=duration,
                          status="timeout",
                          error=f"exceeded budget {budget}s")
    return StepResult(name=name, duration_s=duration, status="ok")


def _run_one_saga(rng: random.Random, fail_rate: float) -> SagaResult:
    steps = list(STEP_BUDGETS_S.keys())
    results: List[StepResult] = []
    total = 0.0
    completed = True
    for s in steps:
        r = _simulate_step(s, rng, fail_rate=fail_rate)
        results.append(r)
        total += r.duration_s
        if r.status != "ok":
            completed = False
            break
    return SagaResult(total_s=total, steps=results, completed=completed)


def main(argv: Optional[List[str]] = None) -> int:
    p = argparse.ArgumentParser(description="POC onboarding saga bench")
    p.add_argument("--samples", type=int, default=200,
                   help="Number of saga runs to simulate.")
    p.add_argument("--seed", type=int, default=42)
    p.add_argument("--fail-rate", type=float, default=0.005,
                   help="Per-step failure probability (default 0.5 %).")
    p.add_argument("--out", type=Path,
                   default=Path("target/ops/measures.saga.json"))
    args = p.parse_args(argv)

    rng = random.Random(args.seed)
    runs = [_run_one_saga(rng, args.fail_rate) for _ in range(args.samples)]
    durations = [r.total_s for r in runs if r.completed]
    success_rate = len(durations) / len(runs)
    if not durations:
        print("ERROR : zero completed sagas", file=sys.stderr)
        return 1

    durations.sort()
    p50 = durations[len(durations) // 2]
    p95_idx = int(len(durations) * 0.95)
    p95 = durations[p95_idx]
    p99 = durations[min(len(durations) - 1, int(len(durations) * 0.99))]
    mean = statistics.fmean(durations)

    payload = {
        "SLO-OPS-PROVISION-P95":      p95,
        "_saga_bench_meta": {
            "samples": args.samples,
            "success_rate": success_rate,
            "p50_s": p50,
            "p95_s": p95,
            "p99_s": p99,
            "mean_s": mean,
            "fail_rate": args.fail_rate,
        },
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(payload, indent=2), encoding="utf-8")
    print(f"saga POC bench → {args.out}")
    print(f"  n={args.samples}  success={success_rate:.1%}  "
          f"p50={p50:.0f}s  p95={p95:.0f}s  p99={p99:.0f}s  mean={mean:.0f}s")
    return 0


if __name__ == "__main__":
    sys.exit(main())
