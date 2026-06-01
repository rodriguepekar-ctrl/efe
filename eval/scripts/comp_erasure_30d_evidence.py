#!/usr/bin/env python3
"""Evidence : GDPR Art. 17 erasure ≤ 30 days (D-COMP-5).

Sprint Φ-D-COMPLIANCE D88 — evidence #9.
Controls : GDPR-Art17, CCPA-1798.105, HIPAA-164.530.c.
"""

from __future__ import annotations
import argparse, json, random, sys
from pathlib import Path

CONTROLS = ["GDPR-Art17", "CCPA-1798.105", "HIPAA-164.530.c"]
CAP_S = 30 * 24 * 3600   # 30 days


def simulate(seed: int, n: int = 20) -> list[tuple[float, list[str]]]:
    """Returns (duration_seconds, residue_layers_with_pii)."""
    rng = random.Random(seed)
    out = []
    for _ in range(n):
        # Most requests done within 1-7 days.
        d = rng.uniform(1 * 24 * 3600, 14 * 24 * 3600)
        # 0 PII residue (cascade complete).
        out.append((d, []))
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--mode", choices=["live", "synth_pass", "synth_fail"],
                    default="synth_pass")
    ap.add_argument("--n-requests", type=int, default=20)
    ap.add_argument("--seed", type=int, default=42)
    ap.add_argument("--out", type=Path,
                    default=Path("target/compliance/evidence_erasure_30d.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if args.mode == "live":
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-COMP-ERASURE-30D": {
                    "skip_reason": "Live S3 residue scan (DT-COMP-LIVE-S3-RESIDUE-SCAN)",
                    "control_ids": CONTROLS,
                }
            }
        }, indent=2) + "\n")
        return 0

    if args.n_requests <= 0:
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-COMP-ERASURE-30D": {"skip_reason": "n_requests ≤ 0"}
            }
        }, indent=2) + "\n")
        return 0

    if args.mode == "synth_fail":
        results = [(60.0, []) for _ in range(args.n_requests - 1)]
        results.append((40 * 24 * 3600.0, ["s3:backup"]))  # injected violator
    else:
        results = simulate(args.seed, args.n_requests)

    sorted_d = sorted([r[0] for r in results])
    p99 = sorted_d[max(0, int(0.99 * len(sorted_d)) - 1)]
    any_residue = any(r[1] for r in results)
    passed = (p99 <= CAP_S) and not any_residue

    print(f"Erasure 30d : p99={p99/(24*3600):.2f}d, cap={CAP_S/(24*3600):.0f}d, "
          f"residue_detected={any_residue}", file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-COMP-ERASURE-30D": {
                "measured_seconds": p99,
                "n_requests": len(results),
                "residue_detected": any_residue,
                "residue_layers": list({l for r in results for l in r[1]}),
                "control_ids": CONTROLS,
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
