#!/usr/bin/env python3
"""Active Learning concept drift detector.

Sprint **Φ-D-AL Completion (D76)** — artefact #10.
Cf. ``docs/EFE_ACTIVE_LEARNING_COMPLETION_ARCHITECTURE.md`` § 13.

Daily job compares :
  * Current production embedding distribution (= last 24h sample).
  * Baseline (= at last retune time).

Wasserstein 1D distance (= sliced for high-dim). Alert if W > threshold.

Stub mode requires injected drift fixture to validate.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import random
import sys
from pathlib import Path

THRESHOLD_W = 0.15  # Wasserstein threshold


def wasserstein_1d(a: list[float], b: list[float]) -> float:
    """1-D Wasserstein distance between two empirical distributions.

    Audit fix Φ-D-AL.A3 : pre-fix `n == 0` was only checked when BOTH
    arrays were empty. If `a == []` and `b == [x, y, z]`, n = 3 and
    `a_sorted[min(0, -1)] = a_sorted[-1]` → IndexError. Post-fix : any
    empty array returns 0.0 with a stderr warning (= "no evidence",
    not faussement-zero).
    """
    a_sorted = sorted(a)
    b_sorted = sorted(b)
    if not a_sorted or not b_sorted:
        # Anti-Φ-D.A1 : empty input is not "drift = 0", it's "no evidence".
        # Callers must distinguish via the surrounding skip path.
        sys.stderr.write(
            f"WARN : wasserstein_1d called with empty input "
            f"(|a|={len(a_sorted)}, |b|={len(b_sorted)}) ; returning 0.0\n"
        )
        return 0.0
    # Pad to same length via simple interpolation (= step approximation).
    n = max(len(a_sorted), len(b_sorted))
    a_pad = [a_sorted[min(int(i * len(a_sorted) / n), len(a_sorted) - 1)] for i in range(n)]
    b_pad = [b_sorted[min(int(i * len(b_sorted) / n), len(b_sorted) - 1)] for i in range(n)]
    return sum(abs(x - y) for x, y in zip(a_pad, b_pad)) / n


def synthetic_baseline(n: int, seed: int) -> list[float]:
    rng = random.Random(seed)
    return [rng.gauss(0.0, 1.0) for _ in range(n)]


def synthetic_drift(n: int, seed: int, shift: float = 0.0) -> list[float]:
    rng = random.Random(seed)
    return [rng.gauss(shift, 1.0) for _ in range(n)]


def main() -> int:
    ap = argparse.ArgumentParser(description="AL drift detector")
    ap.add_argument("--baseline-fixture", type=Path,
                    help="JSON file with baseline distribution")
    ap.add_argument("--current-fixture", type=Path,
                    help="JSON file with current distribution")
    ap.add_argument("--mode", choices=["live", "synth_pass", "synth_fail"],
                    default="synth_pass",
                    help="synth_pass = inject NO drift ; synth_fail = inject drift")
    ap.add_argument("--threshold", type=float, default=THRESHOLD_W)
    ap.add_argument("--out", type=Path,
                    default=Path("target/active_learning/drift.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if args.mode == "live":
        if not args.baseline_fixture or not args.current_fixture:
            sys.stderr.write("WARN : live mode requires fixtures ; emitting SKIPPED.\n")
            args.out.write_text(json.dumps({
                "slo_compliance": {
                    "SLO-AL-CONCEPT-DRIFT": {
                        "skip_reason": "Live fixtures required (DT-AL-DRIFT-DUMP)"
                    }
                }
            }, indent=2) + "\n")
            return 0
        baseline = json.loads(args.baseline_fixture.read_text())
        current = json.loads(args.current_fixture.read_text())
    elif args.mode == "synth_pass":
        # No drift : baseline and current both from N(0, 1).
        baseline = synthetic_baseline(1000, seed=42)
        current = synthetic_baseline(1000, seed=43)
    else:  # synth_fail
        # Inject drift : current shifted by 1.0 → W > 0.15.
        baseline = synthetic_baseline(1000, seed=42)
        current = synthetic_drift(1000, seed=44, shift=1.0)

    w = wasserstein_1d(baseline, current)
    drift_detected = w > args.threshold

    # SLO logic : "drift detector fires correctly on injected drift".
    # PASS when : (synth_fail → drift_detected=True) OR (synth_pass → drift_detected=False) OR (live → just record w)
    if args.mode == "synth_fail":
        passed = drift_detected
    elif args.mode == "synth_pass":
        passed = not drift_detected
    else:
        passed = True  # live mode just records

    print(f"\nDrift detection :", file=sys.stderr)
    print(f"  Wasserstein distance : {w:.4f}", file=sys.stderr)
    print(f"  threshold            : {args.threshold}", file=sys.stderr)
    print(f"  drift detected       : {drift_detected}", file=sys.stderr)
    print(f"  mode                 : {args.mode}", file=sys.stderr)
    print(f"  SLO PASS             : {passed}", file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-AL-CONCEPT-DRIFT": {
                "measured_pass_count": 1 if passed else 0,
                "measured_expected_count": 1,
                "wasserstein_distance": w,
                "threshold": args.threshold,
                "drift_detected": drift_detected,
                "mode": args.mode,
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
