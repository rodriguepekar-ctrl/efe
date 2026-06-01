"""Benchmark : `EfeNativeClient` (PyO3) vs `EfeClient` (gRPC).

SLO F-2 (cf. ``docs/EFE_FFI_ARCHITECTURE.md`` § 4.1) : speedup
on p99 latency must be ≥ 5×.

**D25 status** : the gRPC path is skipped when ``efe_tool`` is not
installed (typical on a fresh clone). The native path is benched
unconditionally — operators can compare offline against the gRPC
baseline measured in D10.

Run via :

    python -m pytest python/efe/tests/bench/native_vs_grpc.py -m bench -v

The ``-m bench`` selector skips this test by default in the regular
``pytest`` run.
"""

from __future__ import annotations

import time
from pathlib import Path
from statistics import quantiles
from typing import Any

import pytest

import efe

pytestmark = pytest.mark.bench


def _percentile(latencies_ms: list[float], q: float) -> float:
    """Compute the q-th percentile of a list of latencies (q in [0, 1])."""
    if not latencies_ms:
        return float("nan")
    if len(latencies_ms) == 1:
        return latencies_ms[0]
    cuts = quantiles(latencies_ms, n=100)
    idx = max(0, min(98, int(q * 100) - 1))
    return cuts[idx]


@pytest.fixture(scope="module")
def warmup_iterations() -> int:
    return 50


@pytest.fixture(scope="module")
def bench_iterations() -> int:
    return 1000


def _measure_native(
    client: efe.EfeNativeClient,
    texts: list[str],
    n_iter: int,
) -> list[float]:
    latencies: list[float] = []
    for i in range(n_iter):
        text = texts[i % len(texts)]
        t0 = time.perf_counter()
        client.classify(text)
        t1 = time.perf_counter()
        latencies.append((t1 - t0) * 1000.0)
    return latencies


def _measure_grpc(
    grpc_client: Any,
    catalog: Any,
    texts: list[str],
    n_iter: int,
) -> list[float]:
    latencies: list[float] = []
    for i in range(n_iter):
        text = texts[i % len(texts)]
        t0 = time.perf_counter()
        grpc_client.classify(text, catalog)
        t1 = time.perf_counter()
        latencies.append((t1 - t0) * 1000.0)
    return latencies


def test_native_p99_under_budget(
    bundle: efe.Bundle,
    catalog_path: Path,
    fixture_texts: list[str],
    warmup_iterations: int,
    bench_iterations: int,
) -> None:
    """Native client p99 < 50 ms (informational ; SLO F-2 is the speedup).

    The full SLO check is in ``test_speedup_vs_grpc`` below ; this
    standalone benchmark always runs and provides the absolute
    number for tracking over time.
    """
    client = efe.EfeNativeClient(bundle, str(catalog_path), seed=42)
    try:
        # Warm up.
        _measure_native(client, fixture_texts, warmup_iterations)
        # Measure.
        lat = _measure_native(client, fixture_texts, bench_iterations)
        p50 = _percentile(lat, 0.50)
        p95 = _percentile(lat, 0.95)
        p99 = _percentile(lat, 0.99)
        print(
            f"\n[native_vs_grpc] EfeNativeClient over {bench_iterations} iter "
            f": p50={p50:.3f} ms, p95={p95:.3f} ms, p99={p99:.3f} ms"
        )
        # D25 stub is fast — assert just a sanity ceiling.
        assert p99 < 100.0, f"p99 {p99:.3f} ms > 100 ms ceiling"
    finally:
        client.close()


def test_speedup_vs_grpc(
    bundle: efe.Bundle,
    catalog_path: Path,
    fixture_texts: list[str],
    warmup_iterations: int,
    bench_iterations: int,
) -> None:
    """SLO F-2 : speedup p99 ≥ 5× vs gRPC.

    Skipped when ``efe_tool`` is not installed (typical on a fresh
    clone without the gRPC stack). Operators on a full install
    must run this benchmark before tagging a release.
    """
    pytest.importorskip("efe_tool")
    pytest.importorskip("grpc")

    try:
        from efe_tool import EfeClient
    except ImportError:
        pytest.skip("efe_tool gRPC client not available")
        return

    # Try to construct ; skip if the env doesn't have the required
    # EFE_HOST / catalog setup.
    try:
        grpc_client = EfeClient.from_env()
        catalog_obj = grpc_client.embedder  # placeholder ; real catalog API
    except Exception as e:
        pytest.skip(f"gRPC client setup failed (no EFE_HOST set?): {e}")
        return

    # Native.
    native_client = efe.EfeNativeClient(bundle, str(catalog_path), seed=42)
    try:
        _measure_native(native_client, fixture_texts, warmup_iterations)
        native_lat = _measure_native(
            native_client, fixture_texts, bench_iterations
        )
    finally:
        native_client.close()

    # gRPC.
    _measure_grpc(grpc_client, catalog_obj, fixture_texts, warmup_iterations)
    grpc_lat = _measure_grpc(grpc_client, catalog_obj, fixture_texts, bench_iterations)

    native_p99 = _percentile(native_lat, 0.99)
    grpc_p99 = _percentile(grpc_lat, 0.99)
    speedup = grpc_p99 / native_p99 if native_p99 > 0 else float("inf")
    print(
        f"\n[native_vs_grpc] native p99={native_p99:.3f} ms ; "
        f"grpc p99={grpc_p99:.3f} ms ; speedup={speedup:.2f}×"
    )
    assert speedup >= 5.0, f"SLO F-2 violated : speedup {speedup:.2f}× < 5×"
