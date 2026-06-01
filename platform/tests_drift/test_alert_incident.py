"""AlertIncident FSM + store."""

from __future__ import annotations

from datetime import datetime, timezone

import pytest

from efe_drift.alerting import (
    IncidentFsm,
    InMemoryAlertIncidentStore,
)
from efe_drift.clock import FakeClock
from efe_drift.detection.base import DriftResult
from efe_drift.errors import NotFound, ValidationError
from efe_drift.ids import new_uuid


def _result(**overrides) -> DriftResult:
    base = dict(
        evaluation_id=new_uuid(),
        tenant_id="acme", task_id="summarize",
        feature_id="f1", feature_name="f1",
        baseline_id="b1",
        window_start=datetime(2026, 4, 10, 10, 0, tzinfo=timezone.utc),
        window_end=datetime(2026, 4, 10, 11, 0, tzinfo=timezone.utc),
        method="psi",
        score=0.5, severity="major",
        computed_at=datetime(2026, 4, 10, 11, 0, tzinfo=timezone.utc),
    )
    base.update(overrides)
    return DriftResult(**base)


def test_fsm_transitions_whitelist():
    IncidentFsm.validate("firing", "acked")
    IncidentFsm.validate("firing", "resolved")
    IncidentFsm.validate("acked", "resolved")
    with pytest.raises(ValidationError):
        IncidentFsm.validate("firing", "firing")
    with pytest.raises(ValidationError):
        IncidentFsm.validate("resolved", "acked")


def test_fsm_terminal_state():
    assert IncidentFsm.is_terminal("resolved")
    assert not IncidentFsm.is_terminal("firing")
    assert not IncidentFsm.is_terminal("acked")


@pytest.mark.asyncio
async def test_store_create_fire_ack_resolve():
    clock = FakeClock()
    store = InMemoryAlertIncidentStore(clock=clock)
    incident = await store.create(
        rule_id="r1", result=_result(), now=clock.now(),
    )
    assert incident.state == "firing"
    clock.advance(60)
    acked = await store.ack(
        incident.incident_id, actor="alice", now=clock.now(),
    )
    assert acked.state == "acked"
    assert acked.acked_by == "alice"
    clock.advance(60)
    resolved = await store.resolve(
        incident.incident_id, now=clock.now(),
    )
    assert resolved.state == "resolved"
    # Second resolve is a no-op.
    again = await store.resolve(
        incident.incident_id, now=clock.now(),
    )
    assert again.state == "resolved"


@pytest.mark.asyncio
async def test_store_find_active_returns_incident():
    store = InMemoryAlertIncidentStore()
    incident = await store.create(
        rule_id="r1", result=_result(),
        now=datetime(2026, 4, 10, tzinfo=timezone.utc),
    )
    found = await store.find_active(
        rule_id="r1", feature_id="f1", severity="major",
    )
    assert found is not None and found.incident_id == incident.incident_id
    # Wrong severity → no match.
    assert await store.find_active(
        rule_id="r1", feature_id="f1", severity="critical",
    ) is None


@pytest.mark.asyncio
async def test_store_ack_unknown_incident_raises():
    store = InMemoryAlertIncidentStore()
    with pytest.raises(NotFound):
        await store.ack(
            "nope", actor="x",
            now=datetime(2026, 4, 10, tzinfo=timezone.utc),
        )


@pytest.mark.asyncio
async def test_store_record_dispatch_persists():
    store = InMemoryAlertIncidentStore()
    incident = await store.create(
        rule_id="r1", result=_result(),
        now=datetime(2026, 4, 10, tzinfo=timezone.utc),
    )
    updated = await store.record_dispatch(
        incident.incident_id, channel="slack",
        result={"ok": True},
    )
    assert updated.dispatch_results["slack"] == {"ok": True}


@pytest.mark.asyncio
async def test_store_list_filters():
    clock = FakeClock()
    store = InMemoryAlertIncidentStore(clock=clock)
    await store.create(
        rule_id="r1", result=_result(tenant_id="acme"),
        now=clock.now(),
    )
    await store.create(
        rule_id="r2", result=_result(tenant_id="other"),
        now=clock.now(),
    )
    assert len(await store.list(tenant_id="acme")) == 1
    assert len(await store.list(tenant_id="other")) == 1
    assert len(await store.list(state="firing")) == 2
    assert len(await store.list(state="resolved")) == 0
