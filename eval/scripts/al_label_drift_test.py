#!/usr/bin/env python3
"""Per-label proportion drift detector for Active Learning.

Sprint **Φ-D-AL Completion (D78)** — artefact #22.
Cf. ``docs/EFE_ACTIVE_LEARNING_COMPLETION_ARCHITECTURE.md`` § 13.

Complement to the embedding-distribution Wasserstein drift detector :
this script tracks **per-label proportion drift** (= % of labels in
each class changes over time, signaling intent shift in production).

## Detection

Total Variation Distance (TVD) between two categorical distributions :
    TVD(p, q) = 0.5 * Σ |p_i - q_i|
Alert if TVD > 0.10 (= 10 pp shift).

## Modes

* `--baseline-counts FILE` + `--current-counts FILE` : live mode.
* `--mode synth_pass` : both distributions equal → TVD ≈ 0 → no drift.
* `--mode synth_fail` : current shifted by 30 pp → TVD ≈ 0.30 → drift.
"""

from __future__ import annotations

import argparse
import json
import random
import sys
from pathlib import Path

THRESHOLD_TVD = 0.10


def tvd(p: dict[str, float], q: dict[str, float]) -> float:
    """Total Variation Distance between two label proportions."""
    keys = set(p) | set(q)
    return 0.5 * sum(abs(p.get(k, 0.0) - q.get(k, 0.0)) for k in keys)


def _normalize_counts(counts: dict[str, int]) -> dict[str, float]:
    total = sum(counts.values())
    if total == 0:
        return {}
    return {k: v / total for k, v in counts.items()}


def _synth_baseline() -> dict[str, int]:
    """Synthetic baseline : 5 classes, ~ uniform."""
    rng = random.Random(42)
    classes = ["billing", "shipping", "tech", "account", "refund"]
    counts = {c: rng.randint(180, 220) for c in classes}  # ~ 1000 total
    return counts


def _synth_current(fail: bool) -> dict[str, int]:
    """Synthetic current : either uniform (pass) or shifted toward 'billing'."""
    if not fail:
        rng = random.Random(43)  # different seed, similar distribution
        classes = ["billing", "shipping", "tech", "account", "refund"]
        return {c: rng.randint(180, 220) for c in classes}
    # Inject drift : 'billing' absorbs 30 pp of the other classes.
    return {
        "billing": 500,   # was ~200 ; +300 = +30 pp
        "shipping": 125,
        "tech": 125,
        "account": 125,
        "refund": 125,
    }


def main() -> int:
    ap = argparse.ArgumentParser(description="AL per-label drift detector")
    ap.add_argument("--baseline-counts", type=Path)
    ap.add_argument("--current-counts", type=Path)
    ap.add_argument("--mode", choices=["live", "synth_pass", "synth_fail"],
                    default="synth_pass")
    ap.add_argument("--threshold", type=float, default=THRESHOLD_TVD)
    ap.add_argument("--out", type=Path,
                    default=Path("target/active_learning/label_drift.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if args.mode == "live":
        if not args.baseline_counts or not args.current_counts:
            sys.stderr.write(
                "WARN : live mode requires --baseline-counts + --current-counts ; "
                "emitting SKIPPED.\n"
            )
            args.out.write_text(json.dumps({
                "slo_compliance": {
                    "SLO-AL-LABEL-DRIFT-DETECTION": {
                        "skip_reason": "Live count fixtures required (DT-AL-LABEL-DRIFT)"
                    }
                }
            }, indent=2) + "\n")
            return 0
        # L2 audit fix Φ-D-AL.A11 : clean FATAL message if file is missing.
        for f in (args.baseline_counts, args.current_counts):
            if not f.exists():
                sys.stderr.write(f"FATAL : counts file missing : {f}\n")
                return 2
        baseline = json.loads(args.baseline_counts.read_text())
        current = json.loads(args.current_counts.read_text())
    elif args.mode == "synth_pass":
        baseline = _synth_baseline()
        current = _synth_current(fail=False)
    else:  # synth_fail
        baseline = _synth_baseline()
        current = _synth_current(fail=True)

    p = _normalize_counts(baseline)
    q = _normalize_counts(current)
    distance = tvd(p, q)
    drift_detected = distance > args.threshold

    # SLO logic : "per-label drift detector fires correctly".
    if args.mode == "synth_fail":
        passed = drift_detected
    elif args.mode == "synth_pass":
        passed = not drift_detected
    else:
        passed = True  # live mode just records

    print(f"\nLabel drift : TVD={distance:.4f}, threshold={args.threshold}, "
          f"detected={drift_detected}, mode={args.mode}",
          file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-AL-LABEL-DRIFT-DETECTION": {
                "measured_pass_count": 1 if passed else 0,
                "measured_expected_count": 1,
                "tvd": distance,
                "threshold": args.threshold,
                "drift_detected": drift_detected,
                "mode": args.mode,
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
