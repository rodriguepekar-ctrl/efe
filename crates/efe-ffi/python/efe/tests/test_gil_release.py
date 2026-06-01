"""SLO-FFI-GIL-RELEASED — 4-thread speedup proxy for GIL release ratio.

Sprint **Φ-C-FFI Completion (D27)** — artefact #17.
Cf. ``docs/EFE_PYO3_FFI_COMPLETION_ARCHITECTURE.md`` § 13.3 + SLO F-8.

## Mission

Mesurer indirectement le GIL release ratio R via le **speedup
multi-thread** :

```text
speedup_K_threads = T_serial / T_K_threads
≈ 1 / (1/K + (1-R) * (K-1)/K)

Pour K=4, R=0.95 → speedup ≈ 3.6×
Pour K=4, R=0.50 → speedup ≈ 1.45×
Pour K=4, R=0.00 → speedup = 1.00×
```

Le test exige ``speedup ≥ 3.0×`` (= R ≥ ~0.87, marge sous le R ≥ 0.95 strict).

## Output

```json
{
  "slo_compliance": {
    "SLO-FFI-GIL-RELEASED": {
      "measured_ratio": 3.42,
      "serial_seconds": 1.234,
      "parallel_seconds_4t": 0.361,
      "n_threads": 4,
      "n_iterations": 100
    }
  }
}
```

Note : le pre-Φ-C-FFI ``test_gil_release.py`` était marked
``pytest.mark.skip`` parce que le D25 stub était jugé trop rapide
pour observer le GIL release. Φ-C-FFI résout le problème en passant
de "background thread counts ticks" à "4-thread speedup" qui est
robuste à la vitesse du stub (= si le stub est trop rapide, le
parallèle reste proche du serial à 1.0× → fail).
"""

from __future__ import annotations

import json
import threading
import time
from pathlib import Path

import pytest

import efe

N_ITER_TOTAL = 100
N_THREADS = 4
# Threshold : 3.0× (= R ≈ 0.87 sur 4 threads ; marge sous R ≥ 0.95 strict).
SPEEDUP_THRESHOLD = 3.0


def _measure_serial(client: efe.EfeNativeClient, n: int) -> float:
    t0 = time.perf_counter()
    for i in range(n):
        client.classify(f"text {i % 10}")
    return time.perf_counter() - t0


def _measure_parallel(
    client: efe.EfeNativeClient, n_total: int, n_threads: int
) -> float:
    per_thread = n_total // n_threads

    def worker():
        for i in range(per_thread):
            client.classify(f"text {i % 10}")

    threads = [threading.Thread(target=worker, daemon=True) for _ in range(n_threads)]
    t0 = time.perf_counter()
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    return time.perf_counter() - t0


def test_gil_released_4thread_speedup_proxy(
    bundle: efe.Bundle,
    catalog_path: Path,
) -> None:
    """SLO-FFI-GIL-RELEASED — 4-thread speedup ≥ 3.0× (R ≈ 0.87)."""

    client = efe.EfeNativeClient(bundle, str(catalog_path), seed=42)
    try:
        # Warmup so JIT / allocator caches don't pollute serial.
        _measure_serial(client, 20)

        serial = _measure_serial(client, N_ITER_TOTAL)
        parallel = _measure_parallel(client, N_ITER_TOTAL, N_THREADS)

        speedup = serial / max(parallel, 1e-9)
        # Inverse the speedup→R approximation : R = 1 - (K - K/speedup) / (K-1)
        ratio_estimate = 1.0 - (N_THREADS - N_THREADS / speedup) / (N_THREADS - 1)
        ratio_estimate = max(0.0, min(1.0, ratio_estimate))

        print(
            f"\n[gil_release] serial={serial*1000:.2f} ms ; "
            f"parallel({N_THREADS} threads)={parallel*1000:.2f} ms ; "
            f"speedup={speedup:.2f}× ; estimated R≈{ratio_estimate:.3f}"
        )

        # Emit SLO compliance JSON.
        out_dir = Path("target") / "ffi"
        out_dir.mkdir(parents=True, exist_ok=True)
        (out_dir / "gil_release.json").write_text(
            json.dumps(
                {
                    "slo_compliance": {
                        "SLO-FFI-GIL-RELEASED": {
                            # check_slo.py reads `measured_ratio` for
                            # threshold_ratio comparator (gte 3.0).
                            "measured_ratio": speedup,
                            "serial_seconds": serial,
                            "parallel_seconds_4t": parallel,
                            "estimated_gil_release_fraction": ratio_estimate,
                            "n_threads": N_THREADS,
                            "n_iterations": N_ITER_TOTAL,
                        }
                    }
                },
                indent=2,
            )
            + "\n"
        )

        assert speedup >= SPEEDUP_THRESHOLD, (
            f"SLO-FFI-GIL-RELEASED violated : 4-thread speedup {speedup:.2f}× "
            f"< {SPEEDUP_THRESHOLD}× (= R ≈ {ratio_estimate:.3f} < ~0.87)"
        )
    finally:
        client.close()


def test_background_thread_progresses_during_classify(
    bundle: efe.Bundle,
    catalog_path: Path,
) -> None:
    """Complementary test : a background thread must accumulate ticks
    during a serial classify loop. Catches the "GIL never released"
    extreme regression that the 4-thread speedup test would also catch
    but with more diagnostic clarity.
    """
    client = efe.EfeNativeClient(bundle, str(catalog_path), seed=42)
    try:
        counter = 0
        stop = threading.Event()

        def background():
            nonlocal counter
            while not stop.is_set():
                counter += 1
                time.sleep(0.0001)

        t = threading.Thread(target=background, daemon=True)
        t.start()
        try:
            start_counter = counter
            for _ in range(50):
                client.classify("hello")
            end_counter = counter
        finally:
            stop.set()
            t.join(timeout=1.0)

        progressed = end_counter - start_counter
        # Loose floor : if GIL was held the whole time, counter ≈ 0.
        # 10 ticks across 50 classifies = robust.
        assert progressed >= 10, (
            f"background thread only progressed {progressed} times — "
            "GIL not released during classify"
        )
    finally:
        client.close()
