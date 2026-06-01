"""bench_50tenants_5concurrent — 50 tenants × 5 concurrent runs sustained 4h."""
from __future__ import annotations

import asyncio
import os
import time

import httpx
import pytest


API_BASE = os.environ.get("EFE_API_BASE", "http://localhost:8081")
TOKEN = os.environ.get("EFE_BENCH_TOKEN", "")
DURATION_S = int(os.environ.get("EFE_BENCH_DURATION_S", str(4 * 3600)))


def headers() -> dict[str, str]:
    return {"Authorization": f"Bearer {TOKEN}"}


@pytest.mark.skipif(not TOKEN, reason="EFE_BENCH_TOKEN not set")
@pytest.mark.asyncio
async def test_50tenants_5concurrent():
    tenants = [f"bench-{i:02d}" for i in range(50)]
    end_at = time.time() + DURATION_S

    async with httpx.AsyncClient(timeout=30.0) as client:
        # Phase 1 : start initial wave (5 per tenant).
        await asyncio.gather(*[
            _spawn_runs(client, tenant, count=5)
            for tenant in tenants
        ])

        # Phase 2 : sustain. As runs complete, replenish so each tenant
        # always has 5 active.
        while time.time() < end_at:
            await asyncio.sleep(30)
            for tenant in tenants:
                active = await _count_active(client, tenant)
                if active < 5:
                    await _spawn_runs(client, tenant, count=5 - active)

    # Phase 3 : verify metrics + pass criteria.
    r = await _http_get(API_BASE + "/metrics")
    assert "efe_agent_sandbox_escape_attempts_total" in r
    # No escapes.
    for line in r.splitlines():
        if line.startswith("efe_agent_sandbox_escape_attempts_total{"):
            value = float(line.split()[-1])
            assert value == 0, f"sandbox escape detected during bench : {line}"


async def _spawn_runs(client: httpx.AsyncClient, tenant: str, count: int) -> None:
    for _ in range(count):
        try:
            await client.post(
                f"{API_BASE}/v1/agent/runs",
                json={
                    "tenant_id": tenant,
                    "template_slug": "noop_research",
                    "initial_prompt": {"prompt": "bench"},
                    "max_cost_usd": 0.10,
                    "timeout_seconds": 600,
                },
                headers=headers(),
            )
        except Exception:
            pass


async def _count_active(client: httpx.AsyncClient, tenant: str) -> int:
    r = await client.get(
        f"{API_BASE}/v1/agent/runs?tenant={tenant}",
        headers=headers(),
    )
    if not r.is_success:
        return 0
    return len(r.json())


async def _http_get(url: str) -> str:
    async with httpx.AsyncClient(timeout=10.0) as client:
        r = await client.get(url)
        return r.text
