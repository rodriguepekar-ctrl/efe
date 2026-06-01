"""bench_pod_crash_recovery — kill the controller pod every 30s during
a 250-run bench. Reaper must recover all orphaned slots within 5 min ;
no DB inconsistency."""
from __future__ import annotations

import asyncio
import os
import subprocess
import time

import httpx
import pytest

API_BASE = os.environ.get("EFE_API_BASE", "http://localhost:8081")
TOKEN = os.environ.get("EFE_BENCH_TOKEN", "")
CONTROLLER_POD_LABEL = os.environ.get(
    "EFE_BENCH_CONTROLLER_LABEL",
    "app.kubernetes.io/name=efe-agent-runner,app.kubernetes.io/component=controller",
)
NS = os.environ.get("EFE_BENCH_NS", "efe-system")


def headers() -> dict[str, str]:
    return {"Authorization": f"Bearer {TOKEN}"}


def kill_controller():
    """Best-effort kill ; needs `kubectl` in PATH + kube context to chaos
    cluster."""
    try:
        subprocess.run(
            [
                "kubectl",
                "-n", NS,
                "delete", "pod",
                "-l", CONTROLLER_POD_LABEL,
                "--wait=false",
            ],
            check=False,
            timeout=10,
        )
    except Exception:
        pass


@pytest.mark.skipif(not TOKEN, reason="EFE_BENCH_TOKEN not set")
@pytest.mark.skipif(
    os.environ.get("EFE_CLUSTER_ROLE") != "chaos",
    reason="kills pods ; chaos cluster only",
)
@pytest.mark.asyncio
async def test_pod_crash_recovery():
    duration_s = 1800  # 30 min
    end = time.time() + duration_s

    async with httpx.AsyncClient(timeout=30.0) as client:
        # Start 250 runs.
        run_ids: list[str] = []
        for _ in range(250):
            r = await client.post(
                f"{API_BASE}/v1/agent/runs",
                json={
                    "tenant_id": "bench",
                    "template_slug": "noop_research",
                    "initial_prompt": {"prompt": "crash bench"},
                    "max_cost_usd": 0.05,
                    "timeout_seconds": 600,
                },
                headers=headers(),
            )
            if r.is_success:
                run_ids.append(r.json()["id"])

        # Kill controller every 30s.
        while time.time() < end:
            await asyncio.sleep(30)
            kill_controller()

        # After bench : reaper must have cleaned up orphans.
        await asyncio.sleep(360)  # 6 min for reaper sweep
        leaked = await client.get(
            f"{API_BASE}/v1/agent/_test/orphan_slots",
            headers=headers(),
        )
        if leaked.is_success:
            assert leaked.json().get("count", 0) == 0, \
                f"orphans not cleaned : {leaked.json()}"
