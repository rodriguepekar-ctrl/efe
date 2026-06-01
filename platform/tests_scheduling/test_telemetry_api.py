"""Tests for telemetry and API stub."""

from __future__ import annotations

import asyncio
import json
from datetime import datetime, timezone

from efe_scheduling.api.deps import Deps
from efe_scheduling.api.http_app import build_app
from efe_scheduling.storage import (
    InMemoryIdempotencyStore,
    InMemoryJobStore,
    InMemoryRunStore,
    InMemoryScheduleStore,
    InMemoryTriggerStore,
)
from efe_scheduling.telemetry.audit import InMemoryAuditSink, write_audit
from efe_scheduling.telemetry.events import InMemoryEventBus, seal_event
from efe_scheduling.telemetry.metrics import MetricsRegistry


# ---------------------------------------------------------------------------
# Telemetry
# ---------------------------------------------------------------------------


def test_metrics_registry_can_create_counter_and_register_defaults() -> None:
    reg = MetricsRegistry()
    c = reg.counter("x_total", "foo", ("label",))
    assert c is not None
    names = reg.names()
    assert "x_total" in names


def test_event_bus_publishes_and_drains() -> None:
    async def scenario() -> None:
        bus = InMemoryEventBus()
        ev = seal_event("efe.sched.job.enqueued", "t1", {"job_id": "abc"})
        await bus.publish(ev)
        assert await bus.count() == 1
        drained = await bus.drain()
        assert drained[0].kind == "efe.sched.job.enqueued"

    asyncio.run(scenario())


def test_audit_sink_jsonl() -> None:
    async def scenario() -> None:
        sink = InMemoryAuditSink()
        ev = seal_event("efe.sched.schedule.created", "t1", {"sid": "1"})
        await write_audit(sink, ev)
        lines = await sink.read_all()
        assert lines[0]["kind"] == "efe.sched.schedule.created"
        assert "seal" in lines[0]
        # Each line must be valid JSON on its own.
        for raw in sink.lines:
            json.loads(raw.rstrip(b"\n").decode("utf-8"))

    asyncio.run(scenario())


# ---------------------------------------------------------------------------
# API app builder (stub without FastAPI installed)
# ---------------------------------------------------------------------------


def test_build_app_registers_health_route() -> None:
    deps = Deps(
        schedule_store=InMemoryScheduleStore(),
        job_store=InMemoryJobStore(),
        run_store=InMemoryRunStore(),
        trigger_store=InMemoryTriggerStore(),
        idempotency_store=InMemoryIdempotencyStore(),
    )
    app = build_app(deps)
    paths: list[str] = []
    for r in getattr(app, "routes", []):
        if isinstance(r, tuple):
            if r and r[0]:
                paths.append(str(r[0][0]))
        else:
            p = getattr(r, "path", None)
            if p is not None:
                paths.append(str(p))
    assert any(p == "/health" for p in paths)
    assert any(p == "/v1/schedules" for p in paths)
