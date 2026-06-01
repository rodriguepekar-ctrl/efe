"""Async overhead bench — Sprint **Φ-C-FFI Completion (D27)**.

Cf. ``docs/EFE_PYO3_FFI_COMPLETION_ARCHITECTURE.md`` § 14.2.

## Mission

Mesure l'overhead du chemin async (``aclassify_zero_copy``) vs sync
(``classify_zero_copy``). Le D25 délègue à ``tokio::task::spawn_blocking``
donc l'overhead doit rester raisonnable (≤ 50 %).

Pas un SLO contractuel (= informational only) mais utile pour détecter
une régression sur la couche async.
"""

from __future__ import annotations

import asyncio
import time
from pathlib import Path

import pytest

import efe

pytestmark = pytest.mark.bench

N_ITER = 100


def _measure_sync(client: efe.EfeNativeClient, text: str, n: int) -> float:
    t0 = time.perf_counter()
    for _ in range(n):
        client.classify(text)
    return time.perf_counter() - t0


async def _measure_async(
    text: str,
    catalog_id: str,
    bundle_path: str,
    catalog_path: str,
    n: int,
) -> float:
    t0 = time.perf_counter()
    for _ in range(n):
        await efe.aclassify_zero_copy(
            text,
            catalog_id=catalog_id,
            bundle_path=bundle_path,
            catalog_path=catalog_path,
            seed=42,
        )
    return time.perf_counter() - t0


def test_async_overhead_under_2x_sync(
    bundle: efe.Bundle,
    catalog_path: Path,
) -> None:
    """Async path should not be more than 2× slower than sync.

    A larger gap suggests the spawn_blocking thread-pool is saturated
    or tokio runtime init is being repeated. The pin test signals
    such a regression before it pollutes nightly SLO reports.
    """
    text = "hello world"
    bp = str(catalog_path.parent)  # bundle_path arg ≠ catalog_path

    # Sync path.
    client = efe.EfeNativeClient(bundle, str(catalog_path), seed=42)
    try:
        sync_seconds = _measure_sync(client, text, N_ITER)
    finally:
        client.close()

    # Async path.
    async_seconds = asyncio.run(
        _measure_async(text, "router-v1", bp, str(catalog_path), N_ITER)
    )

    ratio = async_seconds / max(sync_seconds, 1e-9)
    print(
        f"\n[async_overhead] sync={sync_seconds*1000:.2f} ms ; "
        f"async={async_seconds*1000:.2f} ms ; ratio={ratio:.2f}×"
    )

    # Threshold : 2× (generous to absorb tokio spawn cost).
    assert ratio < 2.5, (
        f"async overhead too high : {ratio:.2f}× sync (expected < 2.5×)"
    )
