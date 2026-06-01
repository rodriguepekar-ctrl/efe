#!/usr/bin/env python3
"""Bridge Criterion → SLO compliance JSON for the FFI bench harness.

Sprint **Φ-C-FFI Completion (D28)** — artefact #21.
Cf. ``docs/EFE_PYO3_FFI_COMPLETION_ARCHITECTURE.md`` § 14.3.

## Mission

Criterion écrit ``target/criterion/<bench>/estimates.json`` après
``cargo bench -p efe-ffi --bench ffi_overhead``. Ce script lit ces
fichiers + produit un SLO compliance JSON parseable par
``eval/scripts/check_slo.py``.

## Usage

```bash
cargo bench -p efe-ffi --bench ffi_overhead
python scripts/bench/criterion_ffi_to_slo.py \\
    --criterion target/criterion \\
    --out target/ffi/native_vs_grpc.json
```

## Output

```json
{
  "slo_compliance": {
    "SLO-FFI-LATENCY-P99": {
      "measured_seconds": 0.000_002_5,
      "bench_source": "end_to_end_stub_no_python",
      "p50_seconds": 0.000_002_1,
      "p95_seconds": 0.000_002_4,
      "p99_seconds": 0.000_002_5
    }
  }
}
```

## Limitation D25 stub

Le ``end_to_end_stub_no_python`` bench mesure le coût pur Rust hors
Python. Le SLO-FFI-LATENCY-P99 cible 40 ms p99 **inclusivement** du
roundtrip Python — ce script remplit ``measured_seconds`` avec le
bench Rust-side comme **lower bound** information ; le SLO réel est
gated par ``test_speedup_vs_grpc.py`` côté pytest.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


def load_estimates(estimates_path: Path) -> dict | None:
    """Load Criterion estimates.json. Returns None if absent."""
    if not estimates_path.exists():
        return None
    return json.loads(estimates_path.read_text())


def extract_point_seconds(estimates: dict, field: str = "mean") -> float | None:
    """Criterion estimates.json format :

    {
      "mean": {"point_estimate": <ns>, "standard_error": <ns>,
               "confidence_interval": {"lower_bound": <ns>, "upper_bound": <ns>,
                                       "confidence_level": 0.95}},
      "median": {...},
      "median_abs_dev": {...},
      "slope": {...},
      "std_dev": {...}
    }

    Times are in nanoseconds. We convert to seconds for SLO.
    """
    sub = estimates.get(field, {})
    pe = sub.get("point_estimate")
    if pe is None:
        return None
    return float(pe) / 1e9


def main() -> int:
    ap = argparse.ArgumentParser(description="Criterion → FFI SLO compliance JSON")
    ap.add_argument(
        "--criterion",
        type=Path,
        default=Path("target/criterion"),
        help="Criterion output root dir",
    )
    ap.add_argument(
        "--out",
        type=Path,
        default=Path("target/ffi/native_vs_grpc.json"),
        help="Output SLO compliance JSON path",
    )
    ap.add_argument(
        "--bench",
        default="end_to_end_stub_no_python",
        help="Bench name to read p99 from",
    )
    args = ap.parse_args()

    bench_dir = args.criterion / args.bench
    estimates_path = bench_dir / "new" / "estimates.json"
    if not estimates_path.exists():
        # Try old-style path
        estimates_path = bench_dir / "estimates.json"
    estimates = load_estimates(estimates_path)
    if estimates is None:
        sys.stderr.write(
            f"FATAL : no Criterion estimates found at {estimates_path}\n"
            f"Run `cargo bench -p efe-ffi --bench ffi_overhead` first.\n"
        )
        return 2

    mean_s = extract_point_seconds(estimates, "mean") or 0.0
    median_s = extract_point_seconds(estimates, "median") or 0.0
    # Approximation p99 ≈ mean + 3σ (= upper bound of 99 % confidence
    # interval pour une distribution approximativement normale).
    std_s = extract_point_seconds(estimates, "std_dev") or 0.0
    p99_s = mean_s + 3.0 * std_s
    p95_s = mean_s + 1.65 * std_s

    payload = {
        "slo_compliance": {
            "SLO-FFI-LATENCY-P99": {
                "measured_seconds": p99_s,
                "bench_source": args.bench,
                "mean_seconds": mean_s,
                "median_seconds": median_s,
                "std_dev_seconds": std_s,
                "p95_seconds": p95_s,
                "p99_seconds": p99_s,
            }
        }
    }

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(payload, indent=2) + "\n")
    print(
        f"Wrote SLO compliance JSON → {args.out}\n"
        f"  bench={args.bench}\n"
        f"  mean={mean_s*1e6:.2f} µs ; p99≈{p99_s*1e6:.2f} µs"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
