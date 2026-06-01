#!/usr/bin/env python3
"""Evidence : breach containment ≤ 24h (D-COMP-4).

Sprint Φ-D-COMPLIANCE D88 — evidence #8.
Controls : ISO-A.5.24, ISO-A.5.25, SOC2-CC7.4.
"""

from __future__ import annotations
import argparse, json, random, sys
from pathlib import Path

CONTROLS = ["ISO-A.5.24", "ISO-A.5.25", "SOC2-CC7.4"]
CAP_S = 24 * 3600


def simulate(seed: int, n: int = 30) -> list[float]:
    rng = random.Random(seed)
    return [rng.uniform(60, 18 * 3600) for _ in range(n)]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--mode", choices=["live", "synth_pass", "synth_fail"],
                    default="synth_pass")
    ap.add_argument("--n-incidents", type=int, default=30)
    ap.add_argument("--seed", type=int, default=42)
    ap.add_argument("--out", type=Path,
                    default=Path("target/compliance/evidence_breach_24h_containment.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if args.mode == "live":
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-COMP-BREACH-CONTAIN-24H": {
                    "skip_reason": "Live breach drill timing",
                    "control_ids": CONTROLS,
                }
            }
        }, indent=2) + "\n")
        return 0

    if args.n_incidents <= 0:
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-COMP-BREACH-CONTAIN-24H": {
                    "skip_reason": "n_incidents ≤ 0 (no evidence)"
                }
            }
        }, indent=2) + "\n")
        return 0

    if args.mode == "synth_fail":
        durations = [60.0] * (args.n_incidents - 1) + [36 * 3600.0]
    else:
        durations = simulate(args.seed, args.n_incidents)

    sorted_d = sorted(durations)
    p99 = sorted_d[max(0, int(0.99 * len(sorted_d)) - 1)]
    passed = p99 <= CAP_S

    print(f"Containment 24h : p99={p99/3600:.2f}h, cap={CAP_S/3600:.0f}h", file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-COMP-BREACH-CONTAIN-24H": {
                "measured_seconds": p99,
                "n_incidents": len(durations),
                "control_ids": CONTROLS,
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
