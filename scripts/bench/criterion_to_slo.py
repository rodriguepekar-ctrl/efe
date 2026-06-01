#!/usr/bin/env python3
# SPDX-License-Identifier: UNLICENSED
"""Sprint **Φ-D D30 (Bundle completion)** — Criterion → SLO report.

Cf. `docs/EFE_CUDA_KERNELS_COMPLETION_ARCHITECTURE.md` § 13.2.

## Mission

Convertir les rapports JSON de Criterion (= `target/criterion/<bench>/
<id>/new/estimates.json`) en un format consommable par
`eval/scripts/check_slo.py`.

Criterion produit pour chaque bench :
```json
{
  "mean":   {"point_estimate": 12000000, "confidence_interval": ...},
  "median": {...},
  "slope":  {...},
  ...
}
```

`check_slo.py` attend :
```json
{
  "slo_compliance": {
    "SLO-CUDA-FUSION-ROUND": {"measured_ns": 12000000},
    "SLO-CUDA-FUSION-TOTAL": {"measured_ns": 120000000}
  }
}
```

Ce script fait la conversion + agrège les ratios speedup.

## Usage

```bash
python scripts/bench/criterion_to_slo.py \
    --criterion target/criterion \
    --output target/bench/cuda_kernels_report.json
```

## Pin tests

`scripts/tests/test_criterion_to_slo.py` (= 6 tests pure-function).
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


# Mapping Criterion bench name → SLO ID
# Source de vérité : `crates/efe-kernels/benches/*_bench.rs`
BENCH_TO_SLO: dict[str, str] = {
    "fusion_round":     "SLO-CUDA-FUSION-ROUND",
    "fusion_10rounds":  "SLO-CUDA-FUSION-TOTAL",
    "fusion_cpu":       "_CPU_BASELINE",  # = computed below for speedup ratio
    "fusion_gpu":       "_GPU_BASELINE",
}


def find_estimate_files(criterion_dir: Path) -> list[Path]:
    """Walk Criterion output dir, find `estimates.json` files."""
    return sorted(criterion_dir.rglob("new/estimates.json"))


def parse_estimate(path: Path) -> dict[str, Any] | None:
    """Parse 1 estimates.json file ; return median_ns + bench_id."""
    try:
        data = json.loads(path.read_text())
    except (json.JSONDecodeError, OSError) as e:
        sys.stderr.write(f"skip {path}: {e}\n")
        return None
    median = data.get("median", {}).get("point_estimate")
    if median is None:
        sys.stderr.write(f"skip {path}: no median.point_estimate\n")
        return None
    # bench_id = parent dir name (= e.g. "fusion_round")
    bench_id = path.parent.parent.name
    return {
        "bench_id": bench_id,
        "median_ns": int(median),
    }


def build_slo_compliance(
    parsed_benches: list[dict[str, Any]],
) -> dict[str, dict[str, Any]]:
    """Transform parsed benches into slo_compliance format."""
    compliance: dict[str, dict[str, Any]] = {}
    cpu_baseline_ns: int | None = None
    gpu_baseline_ns: int | None = None

    for b in parsed_benches:
        bench_id = b["bench_id"]
        median_ns = b["median_ns"]
        slo_id = BENCH_TO_SLO.get(bench_id)
        if slo_id is None:
            continue   # bench non-pertinent pour SLO
        if slo_id == "_CPU_BASELINE":
            cpu_baseline_ns = median_ns
            continue
        if slo_id == "_GPU_BASELINE":
            gpu_baseline_ns = median_ns
            continue
        compliance[slo_id] = {"measured_ns": median_ns}

    # Speedup ratio (= SLO-CUDA-SPEEDUP-CPU) si on a les 2 baselines
    if cpu_baseline_ns is not None and gpu_baseline_ns is not None and gpu_baseline_ns > 0:
        speedup = cpu_baseline_ns / gpu_baseline_ns
        compliance["SLO-CUDA-SPEEDUP-CPU"] = {
            "measured_ratio": speedup,
            "cpu_baseline_ns": cpu_baseline_ns,
            "gpu_baseline_ns": gpu_baseline_ns,
        }

    return compliance


def run(criterion_dir: Path, output: Path) -> int:
    """Pipeline complet criterion → slo_compliance JSON."""
    if not criterion_dir.exists():
        sys.stderr.write(f"ERROR: {criterion_dir} not found ; run benches first\n")
        return 2

    estimate_files = find_estimate_files(criterion_dir)
    if not estimate_files:
        sys.stderr.write(f"WARN: no estimates.json found in {criterion_dir}\n")
        # Empty compliance — check_slo will skip all SLOs gracefully
        compliance = {}
    else:
        parsed = [p for p in (parse_estimate(f) for f in estimate_files) if p is not None]
        compliance = build_slo_compliance(parsed)

    report = {
        "schema_version": "1.0",
        "source": "criterion",
        "criterion_dir": str(criterion_dir),
        "n_benches_parsed": len(estimate_files),
        "slo_compliance": compliance,
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2, sort_keys=True))
    sys.stderr.write(f"✓ Report written: {output}\n")
    sys.stderr.write(f"  benches parsed: {len(estimate_files)}\n")
    sys.stderr.write(f"  SLOs populated: {len(compliance)}\n")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--criterion", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    return run(args.criterion, args.output)


if __name__ == "__main__":
    sys.exit(main())
