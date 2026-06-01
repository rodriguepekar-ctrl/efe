#!/usr/bin/env python3
# SPDX-License-Identifier: UNLICENSED
"""Sprint **Φ-D D31 (Bundle completion)** — 24h soak metrics analyzer.

Cf. `docs/EFE_CUDA_KERNELS_COMPLETION_ARCHITECTURE.md` § 12.4 + § 13.1.

## Mission

Analyser le CSV de métriques produit pendant un soak 24h
(= `timestamp_unix,vram_used_mib,rss_bytes`) et produire un
rapport JSON consommable par `check_slo.py` pour gater
**SLO-CUDA-SOAK-24H** (= RSS drift ≤ 10 MiB/h).

## Format d'entrée

CSV sans header :
```
1722499200,4523,1234567890
1722499260,4524,1234567920
...
```

## Format de sortie

```json
{
  "n_samples": 1440,
  "duration_hours": 24.0,
  "rss_start_bytes": 1234567890,
  "rss_end_bytes": 1244567890,
  "rss_drift_bytes_per_hour": 416666.7,
  "vram_start_mib": 4523,
  "vram_end_mib": 4530,
  "vram_drift_mib_per_hour": 0.29,
  "slo_compliance": {
    "SLO-CUDA-SOAK-24H": {
      "measured_bytes_per_hour": 416666,
      "threshold_bytes_per_hour": 10485760,
      "passed": true
    }
  }
}
```

## Usage

```bash
# Standalone analysis (= produce JSON report)
python scripts/bench/analyze_soak.py soak-metrics.csv \
    --output target/soak-report.json

# Direct exit code (= for CI)
python scripts/bench/analyze_soak.py soak-metrics.csv
# returns 0 if drift ≤ 10 MiB/h, 1 otherwise
```

## Méthode

Drift = pente OLS linéaire (least squares) sur (time_hours, bytes).
Robuste aux fluctuations transitoires ; détecte le **trend long terme**.

## Pin tests

`scripts/tests/test_analyze_soak.py` (= 5 tests pure-function).
"""

from __future__ import annotations

import argparse
import csv
import json
import sys
from pathlib import Path
from typing import Any


# Seuil SLO-CUDA-SOAK-24H (= aligné avec bundle_slos.yaml)
SLO_CUDA_SOAK_24H_THRESHOLD_BYTES_PER_HOUR = 10 * 1024 * 1024    # 10 MiB/h


def parse_csv(path: Path) -> list[tuple[int, int, int]]:
    """Parse le CSV ; retourne list de (timestamp, vram_mib, rss_bytes)."""
    out: list[tuple[int, int, int]] = []
    with path.open() as f:
        reader = csv.reader(f)
        for i, row in enumerate(reader):
            if not row or row[0].startswith("#"):
                continue
            try:
                ts = int(row[0])
                vram = int(row[1])
                rss = int(row[2])
                out.append((ts, vram, rss))
            except (ValueError, IndexError) as e:
                sys.stderr.write(f"skip line {i}: {e}\n")
    return out


def ols_slope(xs: list[float], ys: list[float]) -> float:
    """Calcule la pente OLS (least-squares) y = a*x + b ; retourne `a`.

    Pure function. Si n < 2 ou xs constant, retourne 0.
    """
    n = len(xs)
    if n < 2:
        return 0.0
    x_mean = sum(xs) / n
    y_mean = sum(ys) / n
    num = 0.0
    den = 0.0
    for x, y in zip(xs, ys):
        dx = x - x_mean
        num += dx * (y - y_mean)
        den += dx * dx
    if den == 0:
        return 0.0
    return num / den


def analyze(samples: list[tuple[int, int, int]]) -> dict[str, Any]:
    """Analyse les samples + compute drift.

    Retourne un dict JSON-sérialisable.
    """
    if not samples:
        return {
            "n_samples": 0,
            "duration_hours": 0,
            "rss_drift_bytes_per_hour": 0.0,
            "vram_drift_mib_per_hour": 0.0,
            "slo_compliance": {
                "SLO-CUDA-SOAK-24H": {
                    "measured_bytes_per_hour": 0,
                    "threshold_bytes_per_hour": SLO_CUDA_SOAK_24H_THRESHOLD_BYTES_PER_HOUR,
                    "passed": None,
                    "skipped": "no samples",
                }
            },
        }

    timestamps = [s[0] for s in samples]
    vrams = [s[1] for s in samples]
    rsses = [s[2] for s in samples]

    duration_secs = timestamps[-1] - timestamps[0]
    duration_hours = duration_secs / 3600.0

    # Convert timestamps to hours (= scale x-axis pour OLS stability)
    ts_hours = [(t - timestamps[0]) / 3600.0 for t in timestamps]

    rss_slope_per_hour = ols_slope(ts_hours, [float(r) for r in rsses])
    vram_slope_per_hour_mib = ols_slope(ts_hours, [float(v) for v in vrams])

    rss_drift_int = int(round(rss_slope_per_hour))
    passed = rss_drift_int <= SLO_CUDA_SOAK_24H_THRESHOLD_BYTES_PER_HOUR

    return {
        "n_samples": len(samples),
        "duration_hours": round(duration_hours, 3),
        "rss_start_bytes": rsses[0],
        "rss_end_bytes": rsses[-1],
        "rss_drift_bytes_per_hour": round(rss_slope_per_hour, 2),
        "vram_start_mib": vrams[0],
        "vram_end_mib": vrams[-1],
        "vram_drift_mib_per_hour": round(vram_slope_per_hour_mib, 4),
        "slo_compliance": {
            "SLO-CUDA-SOAK-24H": {
                "measured_bytes_per_hour": rss_drift_int,
                "threshold_bytes_per_hour": SLO_CUDA_SOAK_24H_THRESHOLD_BYTES_PER_HOUR,
                "passed": passed,
            }
        },
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("csv", type=Path, help="soak metrics CSV file")
    parser.add_argument("--output", type=Path, default=None, help="JSON report path")
    parser.add_argument("--quiet", action="store_true")
    args = parser.parse_args(argv)

    if not args.csv.exists():
        sys.stderr.write(f"ERROR: {args.csv} not found\n")
        return 2

    samples = parse_csv(args.csv)
    report = analyze(samples)

    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2, sort_keys=True))
        sys.stderr.write(f"✓ Report written: {args.output}\n")
    elif not args.quiet:
        json.dump(report, sys.stdout, indent=2, sort_keys=True)
        sys.stdout.write("\n")

    drift_mib_h = report["rss_drift_bytes_per_hour"] / 1024 / 1024
    sys.stderr.write(
        f"RSS drift = {drift_mib_h:.2f} MiB/h "
        f"(threshold 10 MiB/h)\n"
    )

    if not report["slo_compliance"]["SLO-CUDA-SOAK-24H"]["passed"]:
        sys.stderr.write("::error::SLO-CUDA-SOAK-24H FAILED\n")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
