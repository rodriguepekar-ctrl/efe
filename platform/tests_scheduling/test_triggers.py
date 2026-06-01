"""Tests for triggers : debouncer + drift + metric + composite + manual."""

from __future__ import annotations

from datetime import datetime, timedelta, timezone

import pytest

from efe_scheduling.clock import FakeClock
from efe_scheduling.randomness import FakeRandomness
from efe_scheduling.triggers import (
    AllOfTrigger,
    AnyOfTrigger,
    Debouncer,
    DebouncerConfig,
    DebouncerState,
    DriftEvent,
    DriftSeverity,
    DriftThresholdTrigger,
    DriftTriggerSpec,
    Fire,
    Hold,
    ManualTrigger,
    MetricSample,
    MetricTrigger,
    MetricTriggerSpec,
    NotTrigger,
    Reset,
    TriggerContext,
)
from efe_scheduling.triggers.manual import ManualSignal
from efe_scheduling.types import JobKind


# ---------------------------------------------------------------------------
# Debouncer
# ---------------------------------------------------------------------------


def test_debouncer_fires_on_nth_breach() -> None:
    deb = Debouncer(DebouncerConfig(consecutive_breaches=3, suppression_s=10))
    now = datetime(2026, 4, 21, tzinfo=timezone.utc)
    st = DebouncerState()

    # 2 breaches → no fire
    st, fire = deb.step(st, is_breach=True, now=now)
    assert not fire and st.count == 1
    st, fire = deb.step(st, is_breach=True, now=now)
    assert not fire and st.count == 2
    st, fire = deb.step(st, is_breach=True, now=now)
    assert fire and st.phase == "suppressed"


def test_debouncer_suppression() -> None:
    deb = Debouncer(DebouncerConfig(consecutive_breaches=1, suppression_s=60))
    now = datetime(2026, 4, 21, tzinfo=timezone.utc)
    st, fire = deb.step(DebouncerState(), is_breach=True, now=now)
    assert fire
    # Within the suppression window, additional breaches don't fire again.
    later = now + timedelta(seconds=30)
    st, fire = deb.step(st, is_breach=True, now=later)
    assert not fire
    # After the window, a breach fires again.
    much_later = now + timedelta(seconds=120)
    st, fire = deb.step(st, is_breach=True, now=much_later)
    assert fire


def test_debouncer_normal_resets_count() -> None:
    deb = Debouncer(DebouncerConfig(consecutive_breaches=3, suppression_s=60))
    now = datetime(2026, 4, 21, tzinfo=timezone.utc)
    st, _ = deb.step(DebouncerState(), is_breach=True, now=now)
    st, _ = deb.step(st, is_breach=True, now=now)
    # "normal" sample
    st, fire = deb.step(st, is_breach=False, now=now)
    assert not fire and st.count == 0 and st.phase == "idle"


# ---------------------------------------------------------------------------
# Drift threshold trigger
# ---------------------------------------------------------------------------


def _drift_event(score: float = 0.4, severity: DriftSeverity = DriftSeverity.major) -> DriftEvent:
    now = datetime(2026, 4, 21, tzinfo=timezone.utc)
    return DriftEvent(
        result_id="r",
        tenant_id="t1",
        task_id=None,
        feature_name="latency_ms",
        method="psi",
        score=score,
        severity=severity,
        observed_at=now,
        window_start=now - timedelta(hours=1),
        window_end=now,
    )


def _ctx(signal, now=None, state=None):
    return TriggerContext(
        tenant_id="t1",
        now=now or datetime(2026, 4, 21, tzinfo=timezone.utc),
        clock=FakeClock(),
        random_source=FakeRandomness(),
        signal=signal,
        debouncer_state=state or {},
    )


def test_drift_trigger_fires_after_threshold() -> None:
    trg = DriftThresholdTrigger(
        trigger_id="drift-1",
        tenant_id="t1",
        task_id=None,
        job_kind=JobKind.retrain_decision,
        spec=DriftTriggerSpec(
            threshold=0.3,
            min_severity=DriftSeverity.major,
            consecutive_breaches=2,
            suppression_s=10,
        ),
    )
    state = {}
    # First breach → Hold
    ev = _drift_event(score=0.5, severity=DriftSeverity.critical)
    d = trg.should_fire(_ctx(ev, state=state))
    assert isinstance(d, Hold)

    # Second breach → Fire. We manually push the new state forward using
    # what the previous step emitted.
    # (Here the trigger returns a Fire payload that carries the updated
    # debouncer_state.)
    d = trg.should_fire(
        _ctx(
            ev,
            state={"count": 1, "last_fired_at": None, "phase": "counting"},
        )
    )
    assert isinstance(d, Fire)
    assert d.payload["severity"] == "critical"


def test_drift_trigger_does_not_fire_below_severity() -> None:
    trg = DriftThresholdTrigger(
        trigger_id="d",
        tenant_id="t1",
        task_id=None,
        job_kind=JobKind.retrain_decision,
        spec=DriftTriggerSpec(
            threshold=0.1,
            min_severity=DriftSeverity.critical,
            consecutive_breaches=1,
        ),
    )
    ev = _drift_event(score=0.5, severity=DriftSeverity.major)
    d = trg.should_fire(_ctx(ev))
    assert isinstance(d, Hold)


def test_drift_trigger_matches_feature_glob() -> None:
    trg = DriftThresholdTrigger(
        trigger_id="d",
        tenant_id="t1",
        task_id=None,
        job_kind=JobKind.retrain_decision,
        spec=DriftTriggerSpec(
            feature_match="latency_*",
            threshold=0.1,
            min_severity=DriftSeverity.major,
            consecutive_breaches=1,
        ),
    )
    ev = _drift_event(score=0.5, severity=DriftSeverity.major)
    d = trg.should_fire(_ctx(ev))
    assert isinstance(d, Fire)


# ---------------------------------------------------------------------------
# Metric trigger
# ---------------------------------------------------------------------------


def test_metric_trigger_ge_operator() -> None:
    trg = MetricTrigger(
        trigger_id="m",
        tenant_id="t1",
        task_id=None,
        job_kind=JobKind.rollback,
        spec=MetricTriggerSpec(
            metric="latency_p99_ms",
            threshold=500,
            op=">=",
            consecutive_breaches=1,
        ),
    )
    now = datetime(2026, 4, 21, tzinfo=timezone.utc)
    sample = MetricSample(
        tenant_id="t1",
        task_id=None,
        metric="latency_p99_ms",
        value=600,
        observed_at=now,
    )
    d = trg.should_fire(_ctx(sample, now=now))
    assert isinstance(d, Fire)


def test_metric_trigger_metric_name_mismatch() -> None:
    trg = MetricTrigger(
        trigger_id="m",
        tenant_id="t1",
        task_id=None,
        job_kind=JobKind.rollback,
        spec=MetricTriggerSpec(
            metric="other_metric", threshold=0, op=">=", consecutive_breaches=1
        ),
    )
    now = datetime(2026, 4, 21, tzinfo=timezone.utc)
    sample = MetricSample(
        tenant_id="t1",
        task_id=None,
        metric="latency_p99_ms",
        value=10,
        observed_at=now,
    )
    d = trg.should_fire(_ctx(sample, now=now))
    assert isinstance(d, Hold)


# ---------------------------------------------------------------------------
# Manual trigger
# ---------------------------------------------------------------------------


def test_manual_trigger_fires() -> None:
    trg = ManualTrigger(
        trigger_id="x",
        tenant_id="t1",
        task_id=None,
        job_kind=JobKind.retrain_decision,
    )
    sig = ManualSignal(
        actor="alice", idempotency_key="key123-abcdef", payload={"why": "hotfix"}
    )
    d = trg.should_fire(_ctx(sig))
    assert isinstance(d, Fire)
    assert d.payload["actor"] == "alice"


def test_manual_trigger_disabled() -> None:
    trg = ManualTrigger(
        trigger_id="x",
        tenant_id="t1",
        task_id=None,
        job_kind=JobKind.retrain_decision,
        enabled=False,
    )
    sig = ManualSignal(actor="a", idempotency_key="key12345")
    d = trg.should_fire(_ctx(sig))
    assert isinstance(d, Hold)


# ---------------------------------------------------------------------------
# Composite
# ---------------------------------------------------------------------------


def test_anyof_composite_fires_when_one_child_fires() -> None:
    m1 = ManualTrigger(
        trigger_id="m1", tenant_id="t1", task_id=None, job_kind=JobKind.noop
    )
    m2 = ManualTrigger(
        trigger_id="m2",
        tenant_id="t1",
        task_id=None,
        job_kind=JobKind.noop,
        enabled=False,
    )
    composite = AnyOfTrigger(
        trigger_id="any",
        tenant_id="t1",
        task_id=None,
        job_kind=JobKind.noop,
        children=[m1, m2],
    )
    sig = ManualSignal(actor="a", idempotency_key="long-enough-key")
    d = composite.should_fire(_ctx(sig))
    assert isinstance(d, Fire)


def test_not_trigger_inverts_child() -> None:
    m = ManualTrigger(
        trigger_id="m",
        tenant_id="t1",
        task_id=None,
        job_kind=JobKind.noop,
        enabled=False,
    )
    inv = NotTrigger(
        trigger_id="not",
        tenant_id="t1",
        task_id=None,
        job_kind=JobKind.noop,
        child=m,
    )
    sig = ManualSignal(actor="a", idempotency_key="long-enough-key")
    d = inv.should_fire(_ctx(sig))
    # child held → NOT fires
    assert isinstance(d, Fire)
