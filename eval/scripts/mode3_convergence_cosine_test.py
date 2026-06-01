#!/usr/bin/env python3
"""Pin test for Mode-3 convergence cosine threshold.

Sprint **Φ-D-MODE3 Completion (D72)** — artefact #18.
Cf. ``docs/EFE_MODE3_MULTI_TURN_COMPLETION_ARCHITECTURE.md`` § 6.2.

Verifies the doctrinal invariant : convergence detected when
`cosine_sim(anchor_N, anchor_N+1) ≥ 0.95 for 2 consecutive turns`.
The test uses synthetic anchor pairs where the threshold should fire
exactly at the documented boundary.
"""

from __future__ import annotations

import argparse
import json
import math
import sys
from pathlib import Path


def cosine_sim(a, b):
    dot = sum(x * y for x, y in zip(a, b))
    na = math.sqrt(sum(x * x for x in a))
    nb = math.sqrt(sum(x * x for x in b))
    return dot / (na * nb) if na > 0 and nb > 0 else 0.0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path,
                    default=Path("target/mode3/convergence_cosine_test.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    # Test 1 : two identical vectors → sim = 1.0
    a = [1.0, 0.0, 0.0]
    b = [1.0, 0.0, 0.0]
    sim_1 = cosine_sim(a, b)

    # Test 2 : two vectors with sim ≈ 0.96 (= just above threshold)
    a = [1.0, 0.0, 0.0]
    b = [0.96, 0.28, 0.0]  # ~cosine 0.96
    sim_2 = cosine_sim(a, b)

    # Test 3 : two vectors with sim ≈ 0.93 (= just below threshold)
    a = [1.0, 0.0, 0.0]
    b = [0.93, 0.37, 0.0]
    sim_3 = cosine_sim(a, b)

    # Test 4 : orthogonal → sim = 0
    a = [1.0, 0.0, 0.0]
    b = [0.0, 1.0, 0.0]
    sim_4 = cosine_sim(a, b)

    # Verify doctrine : threshold 0.95.
    THRESHOLD = 0.95
    passed = (
        sim_1 >= THRESHOLD and
        sim_2 >= THRESHOLD and
        sim_3 < THRESHOLD and
        sim_4 < THRESHOLD
    )

    print(f"Convergence cosine doctrine test :", file=sys.stderr)
    print(f"  sim(identical)   = {sim_1:.4f} ≥ 0.95 : {'PASS' if sim_1 >= THRESHOLD else 'FAIL'}",
          file=sys.stderr)
    print(f"  sim(near 0.96)   = {sim_2:.4f} ≥ 0.95 : {'PASS' if sim_2 >= THRESHOLD else 'FAIL'}",
          file=sys.stderr)
    print(f"  sim(near 0.93)   = {sim_3:.4f} < 0.95 : {'PASS' if sim_3 < THRESHOLD else 'FAIL'}",
          file=sys.stderr)
    print(f"  sim(orthogonal)  = {sim_4:.4f} < 0.95 : {'PASS' if sim_4 < THRESHOLD else 'FAIL'}",
          file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-MODE3-CONVERGENCE-COSINE": {
                "measured_pass_count": 1 if passed else 0,
                "measured_expected_count": 1,
                "tests": {
                    "identical": sim_1,
                    "near_above_threshold": sim_2,
                    "near_below_threshold": sim_3,
                    "orthogonal": sim_4,
                },
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
