#!/usr/bin/env python3
"""A/B test rollout decision checker for Mode-2 prompts.

Sprint **Φ-D-MODE2 Completion (D68)** — artefact #26.
Cf. ``docs/EFE_MODE2_ANCHORS_COMPLETION_ARCHITECTURE.md`` § 11.

## Mission

For a given prompt_id with a candidate version rolling out, evaluate
24h-window metrics and decide :
* Continue rollout (= proceed to next phase).
* Hold rollout (= more time needed).
* Auto-rollback (= regression detected).

## Triggers for auto-rollback

| Trigger | Threshold |
|---|---|
| Recall@1 regression | candidate - baseline < -0.01 (= -1%) |
| Cost increase | candidate / baseline > 1.20 (= +20%) |
| Latency p99 increase | candidate - baseline > 5 seconds |
| Sample size insufficient | < 100 samples in window |

## Stub mode

When no metrics input is given, emit a SKIPPED marker.

## Output

```json
{
  "decision": "continue|hold|rollback",
  "reason": "...",
  "metrics_24h": {
    "baseline_recall": 0.88,
    "candidate_recall": 0.91,
    "baseline_cost_usd": 0.015,
    "candidate_cost_usd": 0.016,
    "baseline_latency_p99_s": 24.0,
    "candidate_latency_p99_s": 25.0
  }
}
```
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

RECALL_REGRESSION_THRESHOLD = -0.01
COST_INCREASE_RATIO = 1.20
LATENCY_INCREASE_SECONDS = 5.0
MIN_SAMPLE_SIZE = 100


def evaluate_rollout(metrics: dict) -> tuple[str, str]:
    """Returns (decision, reason)."""
    n = metrics.get("sample_size", 0)
    if n < MIN_SAMPLE_SIZE:
        return ("hold", f"sample size {n} < {MIN_SAMPLE_SIZE}, more time needed")

    baseline_recall = metrics.get("baseline_recall", 0.0)
    candidate_recall = metrics.get("candidate_recall", 0.0)
    recall_delta = candidate_recall - baseline_recall
    if recall_delta < RECALL_REGRESSION_THRESHOLD:
        return ("rollback", f"recall regression {recall_delta:+.4f} < {RECALL_REGRESSION_THRESHOLD}")

    baseline_cost = metrics.get("baseline_cost_usd", 0.0)
    candidate_cost = metrics.get("candidate_cost_usd", 0.0)
    # **Audit fix Φ-D-MODE2 B-extra2** — pre-fix the check `if baseline_cost > 0`
    # silently bypassed cost regression detection when baseline=0 but
    # candidate>0 (= infinite cost increase). Decision returned "continue"
    # instead of "rollback". Bug visible in the reason string :
    # "cost 100000000.00x" (= 1e8 ratio observed) but decision = continue.
    # Post-fix : non-zero candidate with zero baseline → rollback if above
    # a minimum threshold ($0.001 absolute) to avoid spurious rollback on
    # rounding noise.
    ABSOLUTE_COST_FLOOR_USD = 0.001
    if baseline_cost > 0:
        cost_ratio = candidate_cost / baseline_cost
        if cost_ratio > COST_INCREASE_RATIO:
            return ("rollback", f"cost increase {cost_ratio:.2f}x > {COST_INCREASE_RATIO}x")
    elif candidate_cost > ABSOLUTE_COST_FLOOR_USD:
        return (
            "rollback",
            f"baseline cost was 0 but candidate is ${candidate_cost:.4f} "
            f"(= infinite ratio, above ${ABSOLUTE_COST_FLOOR_USD} floor)"
        )

    baseline_lat = metrics.get("baseline_latency_p99_s", 0.0)
    candidate_lat = metrics.get("candidate_latency_p99_s", 0.0)
    lat_delta = candidate_lat - baseline_lat
    if lat_delta > LATENCY_INCREASE_SECONDS:
        return ("rollback", f"latency p99 increase {lat_delta:+.2f}s > {LATENCY_INCREASE_SECONDS}s")

    return ("continue", f"all triggers within tolerance (recall {recall_delta:+.4f}, cost {candidate_cost/max(baseline_cost,1e-9):.2f}x, lat {lat_delta:+.2f}s)")


def main() -> int:
    ap = argparse.ArgumentParser(description="Mode-2 A/B rollout decision")
    ap.add_argument("--prompt-id", required=True)
    ap.add_argument("--window-hours", type=int, default=24)
    ap.add_argument(
        "--metrics-file", type=Path,
        help="JSON file with 24h metrics (= live mode requires Prometheus query)",
    )
    ap.add_argument(
        "--out", type=Path,
        default=Path("target/mode2/ab_decision.json"),
    )
    args = ap.parse_args()

    args.out.parent.mkdir(parents=True, exist_ok=True)

    if not args.metrics_file:
        sys.stderr.write(
            "WARN : --metrics-file not provided ; emitting hold decision.\n"
            "       For live mode, query Prometheus and write metrics JSON.\n"
        )
        decision = "hold"
        reason = "metrics file not provided (= manual review required)"
        metrics = {}
    else:
        if not args.metrics_file.exists():
            sys.stderr.write(f"FATAL : metrics file missing : {args.metrics_file}\n")
            return 2
        metrics = json.loads(args.metrics_file.read_text())
        decision, reason = evaluate_rollout(metrics)

    print(f"\nA/B decision for prompt_id={args.prompt_id} : {decision}", file=sys.stderr)
    print(f"reason : {reason}", file=sys.stderr)

    args.out.write_text(
        json.dumps(
            {
                "prompt_id": args.prompt_id,
                "window_hours": args.window_hours,
                "decision": decision,
                "reason": reason,
                "metrics_24h": metrics,
                "thresholds": {
                    "recall_regression": RECALL_REGRESSION_THRESHOLD,
                    "cost_increase_ratio": COST_INCREASE_RATIO,
                    "latency_increase_seconds": LATENCY_INCREASE_SECONDS,
                    "min_sample_size": MIN_SAMPLE_SIZE,
                },
            },
            indent=2,
        )
        + "\n"
    )

    # Exit non-zero only on rollback (= triggers CI alert).
    return 1 if decision == "rollback" else 0


if __name__ == "__main__":
    sys.exit(main())
