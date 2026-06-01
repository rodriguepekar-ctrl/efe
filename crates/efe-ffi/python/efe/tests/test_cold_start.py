"""SLO-FFI-COLD-START — `import efe` p95 ≤ 100 ms.

Sprint **Φ-C-FFI Completion (D27)** — artefact #13.
Cf. ``docs/EFE_PYO3_FFI_COMPLETION_ARCHITECTURE.md`` § 9.

## Mission

Mesurer le coût d'un cold ``import efe`` (= first import in a fresh
Python process). Lance ``subprocess.run([sys.executable, '-c', ...])``
N fois et calcule p95 sur la distribution.

## Output

```json
{
  "slo_compliance": {
    "SLO-FFI-COLD-START": {
      "measured_seconds": 0.073,
      "p50_seconds": 0.052,
      "p99_seconds": 0.091,
      "n_iterations": 30
    }
  }
}
```

Note : ``measured_seconds`` est le p95 (= la métrique gated par SLO).
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import pytest

# Skipped quand explicitement non-marked (= seulement en `pytest -m bench`).
# Mais en CI nightly on l'inclut.
pytestmark = pytest.mark.bench

ITERATIONS = 30

# SLO threshold : 100 ms p95.
SLO_THRESHOLD_SECONDS = 0.100

SCRIPT = (
    "import time, sys; "
    "t0 = time.perf_counter(); "
    "import efe; "
    "v = efe.version(); "
    "t1 = time.perf_counter(); "
    "sys.stdout.write(f'{(t1-t0)*1000:.3f}\\n')"
)


def _percentile(latencies: list[float], q: float) -> float:
    if not latencies:
        return float("nan")
    sorted_lat = sorted(latencies)
    idx = max(0, min(len(sorted_lat) - 1, int(q * len(sorted_lat)) - 1))
    return sorted_lat[idx]


def test_cold_start_p95_under_100ms() -> None:
    """SLO-FFI-COLD-START — p95 ≤ 100 ms."""
    latencies_ms: list[float] = []
    for _ in range(ITERATIONS):
        result = subprocess.run(
            [sys.executable, "-c", SCRIPT],
            check=True,
            capture_output=True,
            text=True,
            timeout=10,
        )
        latencies_ms.append(float(result.stdout.strip()))

    p50_ms = _percentile(latencies_ms, 0.50)
    p95_ms = _percentile(latencies_ms, 0.95)
    p99_ms = _percentile(latencies_ms, 0.99)

    print(
        f"\n[cold_start] over {ITERATIONS} cold imports : "
        f"p50={p50_ms:.2f} ms, p95={p95_ms:.2f} ms, p99={p99_ms:.2f} ms"
    )

    # Emit SLO compliance JSON.
    out_dir = Path("target") / "ffi"
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "cold_start.json").write_text(
        json.dumps(
            {
                "slo_compliance": {
                    "SLO-FFI-COLD-START": {
                        # `check_slo.py` reads `measured_seconds` for
                        # threshold_seconds comparator. p95 is the
                        # canonical metric (= % users that experience
                        # this or better).
                        "measured_seconds": p95_ms / 1000.0,
                        "p50_seconds": p50_ms / 1000.0,
                        "p99_seconds": p99_ms / 1000.0,
                        "n_iterations": ITERATIONS,
                    }
                }
            },
            indent=2,
        )
        + "\n"
    )

    assert p95_ms < SLO_THRESHOLD_SECONDS * 1000.0, (
        f"SLO-FFI-COLD-START violated : p95 {p95_ms:.2f} ms > {SLO_THRESHOLD_SECONDS*1000:.0f} ms"
    )


def test_cold_start_is_consistent() -> None:
    """Sanity : 30 imports should not span more than 4× p50 → p99.

    Si p99 > 4 × p50, ça indique un outlier (GC pause, page fault, …)
    qui justifie d'investiger même si p95 sous budget.
    """
    latencies_ms: list[float] = []
    for _ in range(ITERATIONS):
        result = subprocess.run(
            [sys.executable, "-c", SCRIPT],
            check=True,
            capture_output=True,
            text=True,
            timeout=10,
        )
        latencies_ms.append(float(result.stdout.strip()))
    p50 = _percentile(latencies_ms, 0.50)
    p99 = _percentile(latencies_ms, 0.99)
    ratio = p99 / max(p50, 1e-6)
    print(f"\n[cold_start] dispersion ratio p99/p50 = {ratio:.2f}")
    # Soft check — info only, doesn't fail. Comments visible in CI logs.
    if ratio > 4.0:
        print(f"  WARNING : high dispersion (p99/p50 = {ratio:.2f}). Investigate.")
