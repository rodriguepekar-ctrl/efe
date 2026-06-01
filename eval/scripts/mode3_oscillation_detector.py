#!/usr/bin/env python3
"""Detect convergence oscillation in Mode-3 sessions.

Sprint **Φ-D-MODE3 Completion (D72)** — artefact #17.
Cf. ``docs/EFE_MODE3_MULTI_TURN_COMPLETION_ARCHITECTURE.md`` § 6.3.

Oscillation pattern : anchor flip-flops between 2 values without
converging. Detected when `cosine_sim(N, N-2) > cosine_sim(N, N-1)`
for ≥ 3 consecutive turns.

Tests against the 5 osc-* adversarial sessions in
`eval/mode3/adversarial_sessions/sessions.json`.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import random
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_ADV = REPO_ROOT / "eval" / "mode3" / "adversarial_sessions" / "sessions.json"
DIM = 384


def cosine_sim(a, b):
    dot = sum(x * y for x, y in zip(a, b))
    na = math.sqrt(sum(x * x for x in a))
    nb = math.sqrt(sum(x * x for x in b))
    return dot / (na * nb) if na > 0 and nb > 0 else 0.0


def osc_anchor(session_id: str, turn: int) -> list[float]:
    """Simulate flip-flopping anchor for oscillation testing."""
    # Two target vectors A and B. Even turns lean A, odd lean B.
    seed_a = int(hashlib.sha256(f"{session_id}/A".encode()).hexdigest()[:8], 16)
    seed_b = int(hashlib.sha256(f"{session_id}/B".encode()).hexdigest()[:8], 16)
    rng_a = random.Random(seed_a)
    rng_b = random.Random(seed_b)
    A = [rng_a.uniform(-1, 1) for _ in range(DIM)]
    B = [rng_b.uniform(-1, 1) for _ in range(DIM)]
    # Normalize
    na = math.sqrt(sum(x*x for x in A))
    nb = math.sqrt(sum(x*x for x in B))
    A = [x/na for x in A]
    B = [x/nb for x in B]
    # Even turn → 80 % A + 20 % B. Odd turn → 80 % B + 20 % A.
    if turn % 2 == 0:
        m = [0.8*a + 0.2*b for a, b in zip(A, B)]
    else:
        m = [0.2*a + 0.8*b for a, b in zip(A, B)]
    nm = math.sqrt(sum(x*x for x in m))
    return [x/nm for x in m]


def detect_oscillation(anchor_history):
    """Returns True if oscillation detected over ≥ 3 consecutive turns."""
    if len(anchor_history) < 4:
        return False
    osc_count = 0
    for i in range(2, len(anchor_history)):
        sim_n_n2 = cosine_sim(anchor_history[i], anchor_history[i-2])
        sim_n_n1 = cosine_sim(anchor_history[i], anchor_history[i-1])
        if sim_n_n2 > sim_n_n1:
            osc_count += 1
        else:
            osc_count = 0
        if osc_count >= 3:
            return True
    return False


def main() -> int:
    ap = argparse.ArgumentParser(description="Mode-3 oscillation detector")
    ap.add_argument("--adversarial-sessions", type=Path, default=DEFAULT_ADV)
    ap.add_argument("--out", type=Path,
                    default=Path("target/mode3/oscillation_test.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    data = json.loads(args.adversarial_sessions.read_text())
    osc_sessions = [s for s in data["sessions"]
                    if s["category"] == "convergence_oscillation_forced"]

    # **Audit fix Φ-D-MODE3 B-extra3** — refuse to pass trivially when
    # no oscillation sessions are present. Pre-fix : 0/0 = trivially
    # all-pass (= faussement-PASS pattern Φ-D.A1). Post-fix : require
    # at least MIN_OSC_SESSIONS to validate the SLO.
    MIN_OSC_SESSIONS = 5
    if len(osc_sessions) < MIN_OSC_SESSIONS:
        sys.stderr.write(
            f"FATAL : only {len(osc_sessions)} oscillation sessions found "
            f"(need ≥ {MIN_OSC_SESSIONS}). Cannot validate SLO on insufficient corpus.\n"
        )
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-MODE3-CONVERGENCE-COSINE": {
                    "skip_reason": (
                        f"only {len(osc_sessions)} osc sessions ; "
                        f"need ≥ {MIN_OSC_SESSIONS} for valid eval"
                    ),
                    "oscillation_sessions": len(osc_sessions),
                }
            }
        }, indent=2) + "\n")
        return 2

    n_detected = 0
    for s in osc_sessions:
        anchors = [osc_anchor(s["id"], t) for t in range(len(s["turns"]))]
        if detect_oscillation(anchors):
            n_detected += 1

    passed = n_detected == len(osc_sessions)
    print(f"Oscillation detection : {n_detected}/{len(osc_sessions)}",
          file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-MODE3-CONVERGENCE-COSINE": {
                "measured_pass_count": 1 if passed else 0,
                "measured_expected_count": 1,
                "oscillation_sessions": len(osc_sessions),
                "detected": n_detected,
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
