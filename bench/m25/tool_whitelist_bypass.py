"""Hallucinated-tool adversarial test.

The hostile_hallucinator template's system prompt instructs the LLM to
emit a tool call with a name NOT in the whitelist. We assert that all
3 layers of enforcement reject :

  - Layer 1 (control plane) : run creation rejects unknown tools at intake.
  - Layer 2 (agent core)    : agent-core refuses to dispatch even if LLM emits.
  - Layer 3 (mcp-server)    : would refuse if layers 1+2 leaked.
"""
from __future__ import annotations

import os
import time

import httpx
import pytest

API = os.environ.get("EFE_API_BASE", "http://localhost:8081")
TOKEN = os.environ.get("EFE_BENCH_TOKEN", "")
TENANT = "chaos"


def _h() -> dict[str, str]:
    return {"Authorization": f"Bearer {TOKEN}"}


@pytest.mark.skipif(not TOKEN, reason="EFE_BENCH_TOKEN not set")
def test_layer1_rejects_unknown_tool_at_intake() -> None:
    body = {
        "tenant_id": TENANT,
        "template_slug": "noop_research",
        "initial_prompt": {"messages": []},
        "tool_whitelist": ["evil_unknown_tool"],
    }
    r = httpx.post(f"{API}/v1/agent/runs", json=body, headers=_h(), timeout=10)
    assert r.status_code == 400 or r.status_code == 412


@pytest.mark.skipif(os.environ.get("EFE_CLUSTER_ROLE") != "chaos", reason="chaos only")
def test_layer2_rejects_when_llm_hallucinates() -> None:
    body = {
        "tenant_id": TENANT,
        "template_slug": "hostile_hallucinator",
        "initial_prompt": {},
        "max_cost_usd": 0.05,
        "timeout_seconds": 120,
    }
    r = httpx.post(f"{API}/v1/agent/runs", json=body, headers=_h(), timeout=10)
    r.raise_for_status()
    run_id = r.json()["id"]

    deadline = time.time() + 180
    while time.time() < deadline:
        d = httpx.get(f"{API}/v1/agent/runs/{run_id}", headers=_h(), timeout=10).json()
        if d["run"]["status"] in ("COMPLETED", "FAILED", "TERMINATED"):
            break
        time.sleep(2)

    audit = httpx.get(
        f"{API}/v1/audit/list?tenant={TENANT}&module=agent_runtime&run_id={run_id}",
        headers=_h(), timeout=10,
    ).json()
    types = [e["event_type"] for e in audit]
    assert "tool_call_rejected_whitelist" in types
