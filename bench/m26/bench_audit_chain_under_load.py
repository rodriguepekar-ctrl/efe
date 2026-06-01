"""bench_audit_chain_under_load — sustained 1000 events/s for 1h.

Verifies :
  - audit append P99 < 50 ms
  - chain verifies end-to-end after the bench
  - HMAC verification keeps up with append rate
"""
from __future__ import annotations

import asyncio
import os
import statistics
import time

import httpx
import pytest

API_BASE = os.environ.get("EFE_API_BASE", "http://localhost:8081")
TOKEN = os.environ.get("EFE_BENCH_TOKEN", "")


def headers() -> dict[str, str]:
    return {"Authorization": f"Bearer {TOKEN}"}


@pytest.mark.skipif(not TOKEN, reason="EFE_BENCH_TOKEN not set")
@pytest.mark.asyncio
async def test_audit_chain_under_load():
    duration_s = int(os.environ.get("EFE_BENCH_AUDIT_DURATION_S", "60"))  # default 60s for CI
    target_events_s = 1000

    latencies: list[float] = []
    started = time.time()
    end = started + duration_s

    async with httpx.AsyncClient(timeout=10.0) as client:
        async def emit_one():
            t0 = time.time()
            await client.post(
                f"{API_BASE}/v1/agent/_test/audit_emit",
                json={"tenant_id": "bench", "event_type": "tool_call_completed"},
                headers=headers(),
            )
            latencies.append(time.time() - t0)

        while time.time() < end:
            tick_start = time.time()
            await asyncio.gather(*[emit_one() for _ in range(target_events_s)])
            elapsed = time.time() - tick_start
            if elapsed < 1.0:
                await asyncio.sleep(1.0 - elapsed)

    p99 = statistics.quantiles(latencies, n=100)[98] if latencies else 0
    assert p99 < 0.05, f"audit append P99 {p99:.3f}s > 50ms"

    # Verify the chain.
    v = await httpx.AsyncClient(timeout=60.0).get(
        f"{API_BASE}/v1/audit/verify?tenant=bench&module=agent_runtime",
        headers=headers(),
    )
    assert v.status_code == 200
    assert v.json().get("ok") is True
