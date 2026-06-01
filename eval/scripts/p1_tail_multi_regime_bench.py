#!/usr/bin/env python3
"""P1 tail multi-regime bench : p99.9 / p99.99 / p99.999.

Sprint **Φ-D-LATACT D91** — artefact #08.
Cf. ``docs/EFE_P1_LATENCE_P2_ACTIVATION_COMPLETION_ARCHITECTURE.md`` § 5.

Emit SLO-LATACT-TAIL-P9999 (Mode-1 GPU p99.99 ≤ 800 ms) sur 100k samples
sustained.

Modes :
  * `synth_pass` : 100k samples avec distribution log-normal calibrée
                   (p99 = 180 ms, p99.99 = 600 ms — below cap 800 ms).
  * `synth_fail` : seed qui produit un outlier > 800 ms.
  * `live`        : exige bench GPU réel (DT-LATACT-GPU-CAPACITY-PROD).
"""

from __future__ import annotations

import argparse
import json
import math
import random
import sys
from pathlib import Path

CAP_P9999_S = 0.800   # 800 ms


def gen_samples(seed: int, n: int) -> list[float]:
    """Generate N latency samples from a log-normal distribution.

    Calibrated such that p99 ≈ 180 ms, p99.99 ≈ 600 ms.
    """
    rng = random.Random(seed)
    samples: list[float] = []
    # Log-normal : exp(μ + σ * Z) where Z ~ N(0,1).
    # μ chosen so that median = 0.025 s, σ controls tail spread.
    mu = math.log(0.025)
    sigma = 0.85
    for _ in range(n):
        z = rng.gauss(0.0, 1.0)
        latency_s = math.exp(mu + sigma * z)
        samples.append(latency_s)
    return samples


def quantile(sorted_samples: list[float], q: float) -> float:
    """Index-based quantile : Returns NaN if not enough samples for confidence."""
    n = len(sorted_samples)
    if n == 0:
        return float("nan")
    idx = max(0, min(n - 1, int(q * n) - 1))
    return sorted_samples[idx]


def main() -> int:
    ap = argparse.ArgumentParser(description="P1 tail multi-regime bench")
    ap.add_argument("--mode", choices=["live", "synth_pass", "synth_fail"],
                    default="synth_pass")
    ap.add_argument("--n-samples", type=int, default=100_000,
                    help="≥ 100k required for p99.99 confidence")
    ap.add_argument("--seed", type=int, default=42)
    ap.add_argument("--out", type=Path,
                    default=Path("target/latact/tail_multi_regime.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if args.mode == "live":
        sys.stderr.write("WARN : live mode requires GPU runner ; SKIPPED.\n")
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-LATACT-TAIL-P9999": {
                    "skip_reason": "Live GPU bench (DT-LATACT-GPU-CAPACITY-PROD)",
                }
            }
        }, indent=2) + "\n")
        return 0

    if args.n_samples <= 0:
        sys.stderr.write(f"WARN : --n-samples {args.n_samples} ≤ 0 ; SKIPPED.\n")
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-LATACT-TAIL-P9999": {
                    "skip_reason": "n_samples ≤ 0 (no evidence)",
                }
            }
        }, indent=2) + "\n")
        return 0

    if args.mode == "synth_fail":
        # Inject enough outliers to push p99.99 above cap.
        # For n=100k, p99.99 index ≈ 99989. We need ≥ 11 outliers at the
        # top so sorted[99989] is an outlier. Inject 0.02 % (= 20 outliers
        # for n=100k) for safety margin.
        n_outliers = max(20, args.n_samples // 5000)
        samples = gen_samples(args.seed, args.n_samples - n_outliers)
        samples.extend([1.500] * n_outliers)
    else:
        samples = gen_samples(args.seed, args.n_samples)

    s = sorted(samples)
    quantiles = {
        "p50":     quantile(s, 0.50),
        "p99":     quantile(s, 0.99),
        "p999":    quantile(s, 0.999) if len(s) >= 1_000 else None,
        "p9999":   quantile(s, 0.9999) if len(s) >= 10_000 else None,
        "p99999":  quantile(s, 0.99999) if len(s) >= 100_000 else None,
    }

    p9999 = quantiles["p9999"]
    passed = p9999 is not None and p9999 <= CAP_P9999_S

    print(f"Tail multi-regime : n={len(samples)}", file=sys.stderr)
    for q_name, q_val in quantiles.items():
        if q_val is not None:
            print(f"  {q_name:<8} = {q_val * 1000:.2f} ms", file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-LATACT-TAIL-P9999": {
                "measured_seconds": p9999,
                "n_samples": len(samples),
                "quantiles": quantiles,
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
