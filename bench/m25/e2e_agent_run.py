"""End-to-end happy path : create AgentRun → wait COMPLETED → assert audit chain.

Pre-requisites :
  - efe-server reachable at $EFE_API_BASE
  - efe-agent-runner deployed (Helm) with EFE_FEATURE_AGENT_RUNTIME_ENABLED=true
  - DPIA signed for tenant 'bench'
  - At least one AgentRunTemplate 'noop_research' upserted
"""
from __future__ import annotations

import os
import time

import httpx
import pytest

API = os.environ.get("EFE_API_BASE", "http://localhost:8081")
TOKEN = os.environ.get("EFE_BENCH_TOKEN", "")
TENANT = "bench"


def _h() -> dict[str, str]:
    return {"Authorization": f"Bearer {TOKEN}"}


@pytest.mark.skipif(not TOKEN, reason="EFE_BENCH_TOKEN not set")
def test_create_run_reaches_completed() -> None:
    body = {
        "tenant_id": TENANT,
        "template_slug": "noop_research",
        "initial_prompt": {"messages": [{"role": "user", "content": "hello"}]},
        "max_cost_usd": 0.10,
        "timeout_seconds": 300,
        "created_by": "bench@efe",
    }
    r = httpx.post(f"{API}/v1/agent/runs", json=body, headers=_h(), timeout=15)
    r.raise_for_status()
    run_id = r.json()["id"]

    deadline = time.time() + 300
    last_status = ""
    while time.time() < deadline:
        d = httpx.get(f"{API}/v1/agent/runs/{run_id}", headers=_h(), timeout=10).json()
        last_status = d["run"]["status"]
        if last_status in ("COMPLETED", "FAILED", "TERMINATED", "TIMED_OUT", "COST_CAPPED"):
            break
        time.sleep(2)
    assert last_status == "COMPLETED", f"unexpected terminal status : {last_status}"

    # Cost must be non-negative + ≤ cap.
    final = httpx.get(f"{API}/v1/agent/runs/{run_id}", headers=_h(), timeout=10).json()["run"]
    assert 0 <= final["current_cost_cents"] <= final["max_cost_cents"]

    # Audit chain must verify.
    v = httpx.get(
        f"{API}/v1/audit/verify?tenant={TENANT}&module=agent_runtime",
        headers=_h(),
        timeout=15,
    )
    v.raise_for_status()
    assert v.json().get("ok") is True
