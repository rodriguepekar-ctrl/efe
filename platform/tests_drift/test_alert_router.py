"""AlertRouter — hysteresis, suppression, dispatch, resolution."""

from __future__ import annotations

from datetime import datetime, timezone

import pytest

from efe_drift.alerting import (
    AlertRouter,
    ChannelRef,
    ChannelRegistry,
    DispatchResult,
    InMemoryAlertIncidentStore,
    InMemoryAlertRuleStore,
)
from efe_drift.alerting.channels.base import IncidentPayload
from efe_drift.clock import FakeClock
from efe_drift.detection import SeverityThresholds
from efe_drift.detection.base import DriftResult
from efe_drift.ids import new_uuid


# ---------------------------------------------------------------------------
# Test channel — captures dispatches in-memory
# ---------------------------------------------------------------------------


class _CaptureChannel:
    name: str = "capture"

    def __init__(self, *, fail: bool = False) -> None:
        self.dispatches: list[IncidentPayload] = []
        self._fail = fail

    async def dispatch(
        self, payload: IncidentPayload,
    ) -> DispatchResult:
        self.dispatches.append(payload)
        if self._fail:
            return DispatchResult(
                ok=False, channel=self.name, error="forced",
            )
        return DispatchResult(
            ok=True, channel=self.name,
            delivered_at=datetime(2026, 4, 10, tzinfo=timezone.utc),
        )


def _result(
    *, score: float, severity: str = "major",
    feature: str = "f1",
) -> DriftResult:
    return DriftResult(
        evaluation_id=new_uuid(),
        tenant_id="acme", task_id="summarize",
        feature_id=feature, feature_name=feature,
        baseline_id="b1",
        window_start=datetime(2026, 4, 10, 10, 0, tzinfo=timezone.utc),
        window_end=datetime(2026, 4, 10, 11, 0, tzinfo=timezone.utc),
        method="psi", score=score, severity=severity,  # type: ignore[arg-type]
        computed_at=datetime(2026, 4, 10, 11, 0, tzinfo=timezone.utc),
    )


async def _build_router(
    *, consecutive: int = 3, suppression_s: int = 1800,
    fail_channel: bool = False,
) -> tuple[AlertRouter, FakeClock, _CaptureChannel, str]:
    clock = FakeClock()
    rules = InMemoryAlertRuleStore(clock=clock)
    incidents = InMemoryAlertIncidentStore(clock=clock)
    channel = _CaptureChannel(fail=fail_channel)
    channels = ChannelRegistry()
    channels.register(channel)
    rule = await rules.create(
        tenant_id="acme", method="psi",
        thresholds=SeverityThresholds(
            warning=0.1, major=0.2, critical=0.4,
        ),
        channels=(ChannelRef(channel="capture", min_severity="warning"),),
        consecutive_breaches=consecutive,
        suppression_s=suppression_s,
    )
    router = AlertRouter(
        rules=rules, incidents=incidents,
        channels=channels, clock=clock,
    )
    return router, clock, channel, rule.rule_id


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_router_hysteresis_delays_firing():
    router, clock, channel, _ = await _build_router(consecutive=3)
    for _ in range(2):
        decisions = await router.handle(_result(score=0.3))
        assert decisions[0].fired is False
        assert decisions[0].note.startswith("hysteresis")
    # Third breach fires.
    decisions = await router.handle(_result(score=0.3))
    assert decisions[0].fired is True
    assert channel.dispatches, "channel should have received the payload"


@pytest.mark.asyncio
async def test_router_suppression_blocks_redispatch():
    router, clock, channel, _ = await _build_router(
        consecutive=1, suppression_s=600,
    )
    first = await router.handle(_result(score=0.3))
    assert first[0].fired is True
    clock.advance(60)
    suppressed = await router.handle(_result(score=0.3))
    assert suppressed[0].suppressed is True
    assert suppressed[0].note == "suppressed"
    # Only one dispatch total.
    assert len(channel.dispatches) == 1
    # After the suppression window, firing re-enables. But since the
    # incident is still active, we'd need it resolved first → emit a
    # non-breach result to close it.
    await router.handle(_result(score=0.0, severity="none"))
    clock.advance(700)
    # A fresh breach now should re-fire.
    fresh = await router.handle(_result(score=0.3))
    assert fresh[0].fired is True
    assert len(channel.dispatches) == 2


@pytest.mark.asyncio
async def test_router_resolves_incident_on_no_breach():
    router, clock, channel, _ = await _build_router(consecutive=1)
    await router.handle(_result(score=0.3))
    # Non-breach result — should resolve.
    decisions = await router.handle(_result(score=0.0, severity="none"))
    assert decisions[0].resolved is True
    assert decisions[0].fired is False


@pytest.mark.asyncio
async def test_router_tracks_failed_channel_dispatch():
    router, clock, channel, _ = await _build_router(
        consecutive=1, fail_channel=True,
    )
    decisions = await router.handle(_result(score=0.3))
    assert decisions[0].fired is True
    assert not decisions[0].dispatches[0].ok


@pytest.mark.asyncio
async def test_router_channel_min_severity_filters():
    clock = FakeClock()
    rules = InMemoryAlertRuleStore(clock=clock)
    incidents = InMemoryAlertIncidentStore(clock=clock)
    channel = _CaptureChannel()
    channels = ChannelRegistry()
    channels.register(channel)
    await rules.create(
        tenant_id="acme", method="psi",
        thresholds=SeverityThresholds(
            warning=0.1, major=0.2, critical=0.4,
        ),
        channels=(ChannelRef(
            channel="capture", min_severity="critical",
        ),),
        consecutive_breaches=1,
    )
    router = AlertRouter(
        rules=rules, incidents=incidents,
        channels=channels, clock=clock,
    )
    # major breach — below channel's critical floor → incident fires
    # but nothing is dispatched to this channel.
    decisions = await router.handle(_result(score=0.3, severity="major"))
    assert decisions[0].fired is True
    assert len(channel.dispatches) == 0
    # Critical breach now dispatches.
    decisions = await router.handle(_result(score=0.5, severity="critical"))
    # Note: different severity → separate incident track.
    assert len(channel.dispatches) == 1


@pytest.mark.asyncio
async def test_router_missing_channel_records_error():
    clock = FakeClock()
    rules = InMemoryAlertRuleStore(clock=clock)
    incidents = InMemoryAlertIncidentStore(clock=clock)
    channels = ChannelRegistry()
    # NOTE: no channel registered
    await rules.create(
        tenant_id="acme", method="psi",
        thresholds=SeverityThresholds(
            warning=0.1, major=0.2, critical=0.4,
        ),
        channels=(ChannelRef(channel="nonexistent"),),
        consecutive_breaches=1,
    )
    router = AlertRouter(
        rules=rules, incidents=incidents,
        channels=channels, clock=clock,
    )
    decisions = await router.handle(_result(score=0.3))
    assert decisions[0].fired is True
    assert decisions[0].dispatches[0].error == "channel_not_registered"
