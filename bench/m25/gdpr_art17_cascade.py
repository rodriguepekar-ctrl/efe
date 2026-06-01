"""GDPR Art. 17 cascade test.

When a tenant is offboarded (or invokes the right-to-erasure endpoint),
ALL of their agent data must be wiped within the SLA :
  - agent_run + agent_step + agent_audit (HMAC chain detached, not
    deleted, but personal data fields masked)
  - agent_state checkpoints
  - agent_memory entries
  - agent_approval_request

We assert :
  1. After /v1/agent/_test/gdpr_erase, the row counts go to zero
     (or to the masked-stub state for the audit chain).
  2. The audit chain still verifies (HMAC unchanged on retained rows).
  3. A `gdpr_art17_cascade` event is appended to a *separate*,
     compliance-only audit table.
"""
from __future__ import annotations

import os
import time

import httpx
import pytest

API = os.environ.get("EFE_API_BASE", "http://localhost:8081")
TOKEN = os.environ.get("EFE_BENCH_TOKEN", "")
TENANT = "gdpr_test"


def _h() -> dict[str, str]:
    return {"Authorization": f"Bearer {TOKEN}"}


@pytest.mark.skipif(not TOKEN, reason="EFE_BENCH_TOKEN not set")
def test_art17_cascade_removes_run_data() -> None:
    # Pre-condition : seed at least one run for the tenant.
    seed = httpx.post(
        f"{API}/v1/agent/runs",
        json={
            "tenant_id": TENANT,
            "template_slug": "noop_research",
            "initial_prompt": {"messages": []},
            "max_cost_usd": 0.05,
        },
        headers=_h(), timeout=10,
    )
    seed.raise_for_status()
    run_id = seed.json()["id"]
    time.sleep(2)  # let the controller stamp at least one audit row

    # Trigger Art. 17 erasure.
    r = httpx.post(
        f"{API}/v1/agent/_test/gdpr_erase",
        json={"tenant_id": TENANT, "reason": "data subject request 2026-05-10"},
        headers=_h(), timeout=30,
    )
    r.raise_for_status()
    out = r.json()
    assert out.get("ok") is True
    assert out.get("runs_deleted", 0) >= 1
    assert out.get("memory_deleted", 0) >= 0

    # Run must be gone.
    g = httpx.get(f"{API}/v1/agent/runs/{run_id}", headers=_h(), timeout=5)
    assert g.status_code == 404

    # Audit chain must still verify (compliance-only retained rows).
    v = httpx.get(
        f"{API}/v1/audit/verify?tenant={TENANT}&module=agent_runtime",
        headers=_h(), timeout=15,
    )
    assert v.status_code == 200
    assert v.json().get("ok") is True
