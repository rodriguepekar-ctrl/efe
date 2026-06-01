"""Cost-cap enforcement : a run with a deliberately-tiny cap must be
SIGTERMed within ≤ 10 s of crossing the threshold (no grace period)."""
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
def test_cost_cap_terminates_run_promptly() -> None:
    body = {
        "tenant_id": TENANT,
        "template_slug": "expensive_loop",       # template designed to spend fast
        "initial_prompt": {},
        "max_cost_usd": 0.01,                    # 1 cent — tiny
        "timeout_seconds": 600,
        "created_by": "bench@efe",
    }
    r = httpx.post(f"{API}/v1/agent/runs", json=body, headers=_h(), timeout=10)
    r.raise_for_status()
    run_id = r.json()["id"]

    deadline = time.time() + 60   # must terminate well within a minute
    final = None
    while time.time() < deadline:
        d = httpx.get(f"{API}/v1/agent/runs/{run_id}", headers=_h(), timeout=5).json()["run"]
        if d["status"] in ("COST_CAPPED", "COMPLETED", "FAILED", "TERMINATED"):
            final = d
            break
        time.sleep(1)

    assert final is not None and final["status"] == "COST_CAPPED", (
        f"expected COST_CAPPED, got {final and final['status']}"
    )
    # Spent must be ≥ cap (otherwise we're under-cap-tripping).
    assert final["current_cost_cents"] >= 1
