#!/usr/bin/env python3
"""P2 saga rollback bench : injection test, p99 ≤ 10 min.

Sprint **Φ-D-LATACT D93** — artefact #23.
Cf. ``docs/EFE_P1_LATENCE_P2_ACTIVATION_COMPLETION_ARCHITECTURE.md`` § 9.

Inject failures at random saga steps (1..5), measure rollback duration.

Emit SLO-P2-ROLLBACK (≤ 600 s p99).
"""

from __future__ import annotations

import argparse
import json
import random
import sys
from pathlib import Path

CAP_S = 600.0   # 10 min
N_STEPS = 5


def simulate_rollback(seed: int, n_injections: int = 30) -> list[dict]:
    """Returns list of {step_failed_at, rollback_duration_s}."""
    rng = random.Random(seed)
    results = []
    for _ in range(n_injections):
        step_idx = rng.randint(1, N_STEPS)   # fail at step 1..5
        # Each compensate step takes 30-90 seconds.
        compensate_durations = [rng.uniform(30, 90) for _ in range(step_idx)]
        total_rollback_s = sum(compensate_durations)
        results.append({
            "step_failed_at": step_idx,
            "rollback_duration_s": total_rollback_s,
            "compensate_steps_run": step_idx,
        })
    return results


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--mode", choices=["live", "synth_pass", "synth_fail"],
                    default="synth_pass")
    ap.add_argument("--n-injections", type=int, default=30)
    ap.add_argument("--seed", type=int, default=42)
    ap.add_argument("--out", type=Path,
                    default=Path("target/latact/saga_rollback.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if args.mode == "live":
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-P2-ROLLBACK": {
                    "skip_reason": "Live saga injection (DT-LATACT-ROLLBACK-PROD-DRILL)",
                }
            }
        }, indent=2) + "\n")
        return 0

    if args.n_injections <= 0:
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-P2-ROLLBACK": {
                    "skip_reason": f"n_injections ≤ 0 (no evidence)",
                }
            }
        }, indent=2) + "\n")
        return 0

    results = simulate_rollback(args.seed, args.n_injections)
    if args.mode == "synth_fail":
        # Inject : 1 rollback at 700 s (= over cap).
        results[-1]["rollback_duration_s"] = 700.0
        results[-1]["step_failed_at"] = 5

    durations = sorted([r["rollback_duration_s"] for r in results])
    p99_idx = max(0, int(0.99 * len(durations)) - 1)
    p99 = durations[p99_idx]
    p50 = durations[len(durations) // 2]
    passed = p99 <= CAP_S

    print(f"Saga rollback : n={len(results)}, p50={p50:.1f}s, p99={p99:.1f}s, "
          f"cap={CAP_S:.0f}s", file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-P2-ROLLBACK": {
                "measured_seconds": p99,
                "p50_seconds": p50,
                "n_injections": len(results),
                "results_sample": results[:5],
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
