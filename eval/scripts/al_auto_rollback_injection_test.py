#!/usr/bin/env python3
"""Auto-rollback injection test for AL canary promotion.

Sprint **Φ-D-AL Completion (D77)** — artefact #14.
Cf. ``docs/EFE_ACTIVE_LEARNING_COMPLETION_ARCHITECTURE.md`` § 11.3.

Injects 3 regression scenarios and verifies that the A/B decision
engine fires `decision == "rollback"` for each. Reuses the Mode-2
A/B decision logic from `scripts/mode2/check_ab_test.py`.

## Regressions injected (= negative-direction triggers)

| Scenario | Trigger | Expected |
|---|---|---|
| recall_regression | recall -2 % | rollback |
| cost_blowup | cost +150 % | rollback |
| latency_blowup | p99 +12s | rollback |

## Positive control (= L2 audit fix Φ-D-AL.A8)

Audit Level 2 finding : a one-sided test of "rollback fires" cannot
distinguish a correct decision engine from one that always rollbacks.
A 4th `healthy_candidate` scenario is added : recall +1 %, cost flat,
latency flat. Expected `decision == "continue"`. Detects regression
where auto-rollback becomes pathologically conservative.

SLO-AL-PROMOTION-AUTO-ROLLBACK : all 4/4 scenarios must produce the
expected decision (3 rollbacks + 1 continue).
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT / "scripts" / "mode2"))
from check_ab_test import evaluate_rollout  # noqa: E402


SCENARIOS = [
    {
        "name": "recall_regression",
        "metrics": {
            "sample_size": 500,
            "baseline_recall": 0.88,
            "candidate_recall": 0.86,   # -0.02 = -2 %, < -0.01 threshold
            "baseline_cost_usd": 0.015,
            "candidate_cost_usd": 0.015,
            "baseline_latency_p99_s": 24.0,
            "candidate_latency_p99_s": 24.0,
        },
        "expected": "rollback",
    },
    {
        "name": "cost_blowup",
        "metrics": {
            "sample_size": 500,
            "baseline_recall": 0.88,
            "candidate_recall": 0.89,
            "baseline_cost_usd": 0.010,
            "candidate_cost_usd": 0.025,  # ×2.5 > 1.20 threshold
            "baseline_latency_p99_s": 24.0,
            "candidate_latency_p99_s": 24.0,
        },
        "expected": "rollback",
    },
    {
        "name": "latency_blowup",
        "metrics": {
            "sample_size": 500,
            "baseline_recall": 0.88,
            "candidate_recall": 0.89,
            "baseline_cost_usd": 0.015,
            "candidate_cost_usd": 0.015,
            "baseline_latency_p99_s": 20.0,
            "candidate_latency_p99_s": 32.0,  # +12s > 5s threshold
        },
        "expected": "rollback",
    },
    # ── L2 audit fix Φ-D-AL.A8 : positive control (= continue, not rollback) ──
    {
        "name": "healthy_candidate",
        "metrics": {
            "sample_size": 500,
            "baseline_recall": 0.88,
            "candidate_recall": 0.89,         # +1 % ≥ threshold (no regression)
            "baseline_cost_usd": 0.015,
            "candidate_cost_usd": 0.016,      # +6.7 % ratio = 1.067 < 1.20
            "baseline_latency_p99_s": 24.0,
            "candidate_latency_p99_s": 25.0,  # +1s < 5s threshold
        },
        "expected": "continue",
    },
]


def main() -> int:
    ap = argparse.ArgumentParser(description="AL auto-rollback injection test")
    ap.add_argument("--out", type=Path,
                    default=Path("target/active_learning/auto_rollback.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    results = []
    n_pass = 0
    for sc in SCENARIOS:
        decision, reason = evaluate_rollout(sc["metrics"])
        ok = decision == sc["expected"]
        if ok:
            n_pass += 1
        results.append({
            "scenario": sc["name"],
            "decision": decision,
            "expected": sc["expected"],
            "reason": reason,
            "pass": ok,
        })
        sym = "✓" if ok else "✗"
        print(f"  {sym} {sc['name']:<20} : got={decision} expected={sc['expected']}",
              file=sys.stderr)

    passed = n_pass == len(SCENARIOS)
    print(f"\nAuto-rollback injection : {n_pass}/{len(SCENARIOS)} PASS",
          file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-AL-PROMOTION-AUTO-ROLLBACK": {
                "measured_pass_count": 1 if passed else 0,
                "measured_expected_count": 1,
                "n_scenarios": len(SCENARIOS),
                "n_passed": n_pass,
                "results": results,
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
