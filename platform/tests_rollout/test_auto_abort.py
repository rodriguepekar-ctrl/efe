"""Auto-abort executor — interaction with the callback + M06 rollback."""

from __future__ import annotations

from typing import Any

import pytest

from efe_rollout.clock import FakeClock
from efe_rollout.errors import AutoAbortFailed
from efe_rollout.experiment import GuardrailSpec
from efe_rollout.guardrails import AbortContext, AutoAbortExecutor


class _Callback:
    def __init__(self):
        self.transitions: list[tuple[str, str]] = []
        self.events: list[AbortContext] = []

    async def transition_to_aborted(
        self, experiment_id: str, *, reason: str,
    ) -> None:
        self.transitions.append((experiment_id, reason))

    async def publish_aborted_event(self, ctx: AbortContext) -> None:
        self.events.append(ctx)


class _Rollback:
    def __init__(self):
        self.calls: list[dict[str, Any]] = []
        self.raise_exc: Exception | None = None

    async def execute(
        self, *, tenant_id: str, task_id: str,
        actor: str, reason: str,
    ) -> None:
        if self.raise_exc is not None:
            raise self.raise_exc
        self.calls.append({
            "tenant_id": tenant_id,
            "task_id": task_id,
            "actor": actor,
            "reason": reason,
        })


@pytest.mark.asyncio
async def test_auto_abort_full_flow():
    clock = FakeClock()
    cb = _Callback()
    rb = _Rollback()
    executor = AutoAbortExecutor(
        callback=cb, rollback=rb, clock=clock, deadline_s=60.0,
    )
    ctx = AbortContext(
        experiment_id="exp-1",
        tenant_id="acme",
        task_id="summarize",
        layer="prompt",
        guardrail=GuardrailSpec(
            metric_name="latency_ms",
            direction="up",
            threshold_pct=30.0,
        ),
        ratio=1.6,
        breaches=3,
    )
    await executor.abort(ctx)
    assert len(cb.transitions) == 1
    assert cb.transitions[0][0] == "exp-1"
    assert "latency_ms" in cb.transitions[0][1]
    assert len(cb.events) == 1
    assert len(rb.calls) == 1
    assert rb.calls[0]["tenant_id"] == "acme"


@pytest.mark.asyncio
async def test_auto_abort_wraps_downstream_failure():
    clock = FakeClock()
    cb = _Callback()
    rb = _Rollback()
    rb.raise_exc = RuntimeError("M06 is down")
    executor = AutoAbortExecutor(
        callback=cb, rollback=rb, clock=clock,
    )
    ctx = AbortContext(
        experiment_id="exp-1",
        tenant_id="acme",
        task_id="summarize",
        layer="prompt",
        guardrail=GuardrailSpec(
            metric_name="latency_ms",
            direction="up",
            threshold_pct=30.0,
        ),
        ratio=2.0,
        breaches=5,
    )
    with pytest.raises(AutoAbortFailed):
        await executor.abort(ctx)
