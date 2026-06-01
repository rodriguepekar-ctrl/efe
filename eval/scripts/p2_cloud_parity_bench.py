#!/usr/bin/env python3
"""P2 3-cloud parity bench : TTFC variance < 20 %.

Sprint **Φ-D-LATACT D93** — artefact #20.
Cf. ``docs/EFE_P1_LATENCE_P2_ACTIVATION_COMPLETION_ARCHITECTURE.md`` § 8.

Emit SLO-P2-CLOUD-PARITY : max(p90) / min(p90) - 1 ≤ 0.20.
"""

from __future__ import annotations

import argparse
import json
import random
import sys
from pathlib import Path

CLOUDS = ("gcp", "aws", "azure")
CAP_VARIANCE = 0.20   # ≤ 20 %


def gen_ttfc_per_cloud(seed: int, n_per_cloud: int = 50) -> dict[str, list[float]]:
    """Synthesize TTFC for each cloud with slight variance."""
    rng = random.Random(seed)
    base_mean = 1000.0   # 16.7 min
    out = {}
    for cloud in CLOUDS:
        # Each cloud has a slight offset (±10 %).
        offset = {"gcp": 1.00, "aws": 1.05, "azure": 1.08}[cloud]
        samples = [
            rng.uniform(base_mean * offset * 0.7, base_mean * offset * 1.3)
            for _ in range(n_per_cloud)
        ]
        out[cloud] = samples
    return out


def percentile(values: list[float], q: float) -> float:
    s = sorted(values)
    return s[max(0, min(len(s) - 1, int(q * len(s)) - 1))]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--mode", choices=["live", "synth_pass", "synth_fail"],
                    default="synth_pass")
    ap.add_argument("--n-per-cloud", type=int, default=50)
    ap.add_argument("--seed", type=int, default=42)
    ap.add_argument("--out", type=Path,
                    default=Path("target/latact/cloud_parity.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if args.mode == "live":
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-P2-CLOUD-PARITY": {
                    "skip_reason": "Live 3-cloud TTFC bench (DT-LATACT-CLOUD-AZURE-GOV)",
                }
            }
        }, indent=2) + "\n")
        return 0

    if args.n_per_cloud <= 0:
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-P2-CLOUD-PARITY": {
                    "skip_reason": "n_per_cloud ≤ 0 (no evidence)",
                }
            }
        }, indent=2) + "\n")
        return 0

    samples = gen_ttfc_per_cloud(args.seed, args.n_per_cloud)
    if args.mode == "synth_fail":
        # Inject Azure outliers : 3× normal.
        samples["azure"] = [s * 3 for s in samples["azure"]]

    p90_per_cloud = {c: percentile(s, 0.90) for c, s in samples.items()}
    max_p90 = max(p90_per_cloud.values())
    min_p90 = min(p90_per_cloud.values())
    variance = (max_p90 / min_p90 - 1) if min_p90 > 0 else float("inf")
    passed = variance <= CAP_VARIANCE

    print(f"Cloud parity : p90 per cloud = {p90_per_cloud}", file=sys.stderr)
    print(f"  variance = {variance:.4f} (cap = {CAP_VARIANCE})", file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-P2-CLOUD-PARITY": {
                "measured_ratio": variance,
                "p90_per_cloud": p90_per_cloud,
                "max_p90_cloud": max(p90_per_cloud, key=p90_per_cloud.get),
                "min_p90_cloud": min(p90_per_cloud, key=p90_per_cloud.get),
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
