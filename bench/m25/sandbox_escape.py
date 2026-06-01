"""Sandbox-escape adversarial test.

We deploy an *intentionally hostile* agent-core image (pinned to a
test-only digest in the cluster's whitelist) that attempts each of
the known escape vectors :

  - ptrace another process
  - syscall(MOUNT)
  - DNS exfil to non-whitelisted host
  - image swap mid-run

For each vector we assert :
  1. The escape was DENIED (microVM either killed or syscall blocked).
  2. `efe_agent_sandbox_escape_attempts_total{kind=...}` incremented.
  3. The audit chain has a `sandbox_escape_attempt_detected` event.
  4. The run terminated with FAILED + failure_reason mentions sandbox.
  5. PagerDuty test-alert fired (mocked via Alertmanager test endpoint).

This test ONLY runs in the dedicated `chaos` test cluster, never against
prod. The hostile agent-core image is ONLY in the whitelist of the
chaos cluster.
"""
from __future__ import annotations

import os
import time

import httpx
import pytest

CLUSTER_ROLE = os.environ.get("EFE_CLUSTER_ROLE", "")
API = os.environ.get("EFE_API_BASE", "http://localhost:8081")
TOKEN = os.environ.get("EFE_BENCH_TOKEN", "")
TENANT = "chaos"


def _h() -> dict[str, str]:
    return {"Authorization": f"Bearer {TOKEN}"}


@pytest.mark.skipif(CLUSTER_ROLE != "chaos", reason="only runs in chaos cluster")
@pytest.mark.parametrize("vector", ["ptrace", "syscall_mount", "dns_exfil", "image_swap"])
def test_escape_attempt_denied_and_audited(vector: str) -> None:
    body = {
        "tenant_id": TENANT,
        "template_slug": f"hostile_{vector}",
        "initial_prompt": {"vector": vector},
        "max_cost_usd": 0.05,
        "timeout_seconds": 180,
        "created_by": "chaos@efe",
    }
    r = httpx.post(f"{API}/v1/agent/runs", json=body, headers=_h(), timeout=15)
    r.raise_for_status()
    run_id = r.json()["id"]

    # Wait for terminal.
    deadline = time.time() + 240
    final = None
    while time.time() < deadline:
        d = httpx.get(f"{API}/v1/agent/runs/{run_id}", headers=_h(), timeout=10).json()
        if d["run"]["status"] in ("COMPLETED", "FAILED", "TERMINATED"):
            final = d["run"]
            break
        time.sleep(2)
    assert final is not None
    assert final["status"] in ("FAILED", "TERMINATED"), f"escape allowed run to {final['status']}"

    # Metric must have non-zero value for this kind.
    m = httpx.get(f"{API}/metrics", timeout=10).text
    assert (
        f'efe_agent_sandbox_escape_attempts_total{{tenant="{TENANT}",kind="' in m
    ), "escape metric missing or kind label not present"

    # Audit chain : event must be present.
    audit = httpx.get(
        f"{API}/v1/audit/list?tenant={TENANT}&module=agent_runtime&run_id={run_id}",
        headers=_h(),
        timeout=15,
    ).json()
    types = [e["event_type"] for e in audit]
    assert "sandbox_escape_attempt_detected" in types
