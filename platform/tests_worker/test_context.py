"""Tests for :mod:`efe_worker.context`."""

from __future__ import annotations

import pytest

from efe_worker.clock import FakeClock
from efe_worker.config import WorkerSettings
from efe_worker.context import PHASES, RunContext


def _cfg(user_deadline: int | None = None) -> WorkerSettings:
    return WorkerSettings.from_env(overrides={
        "identity": {
            "run_id": "r", "tenant_id": "t", "task_id": "t",
            "snapshot_id": "s", "worker_id": "w",
        },
        "training": {"user_deadline_seconds": user_deadline},
    })


def test_phases_closed_set() -> None:
    assert "training" in PHASES
    assert "upload" in PHASES


def test_set_phase_valid() -> None:
    ctx = RunContext(settings=_cfg())
    ctx.set_phase("training")
    assert ctx.phase == "training"


def test_set_phase_invalid() -> None:
    ctx = RunContext(settings=_cfg())
    with pytest.raises(ValueError):
        ctx.set_phase("galaxy_brain")


def test_record_updates_metrics_and_step() -> None:
    ctx = RunContext(settings=_cfg())
    ctx.record(step=5, loss=0.3, lr=1e-5)
    assert ctx.step == 5
    assert ctx.live_metrics["loss"] == 0.3
    assert ctx.live_metrics["lr"] == 1e-5


def test_record_silently_skips_non_numeric() -> None:
    ctx = RunContext(settings=_cfg())
    ctx.record(weird="hello", n=3)
    assert "weird" not in ctx.live_metrics
    assert ctx.live_metrics["n"] == 3.0


def test_progress_clamped() -> None:
    ctx = RunContext(settings=_cfg())
    ctx.progress_ratio = 1.5
    assert ctx.progress() == 1.0
    ctx.progress_ratio = -0.2
    assert ctx.progress() == 0.0


def test_mark_started_idempotent() -> None:
    ctx = RunContext(settings=_cfg(), clock=FakeClock())
    ctx.mark_started()
    first = ctx.started_at
    ctx.mark_started()
    assert ctx.started_at == first


def test_user_deadline_reached_default_false() -> None:
    ctx = RunContext(settings=_cfg(), clock=FakeClock())
    ctx.mark_started()
    assert ctx.user_deadline_reached() is False


def test_user_deadline_reached_true_after_advance() -> None:
    clock = FakeClock()
    ctx = RunContext(settings=_cfg(user_deadline=30), clock=clock)
    ctx.mark_started()
    assert ctx.user_deadline_reached() is False
    clock.advance(60)
    assert ctx.user_deadline_reached() is True


def test_stop_event_flow() -> None:
    ctx = RunContext(settings=_cfg())
    assert ctx.should_stop() is False
    ctx.request_stop()
    assert ctx.should_stop() is True


def test_identity_accessors() -> None:
    ctx = RunContext(settings=_cfg())
    assert ctx.run_id == "r"
    assert ctx.tenant_id == "t"
    assert ctx.worker_id == "w"


def test_user_deadline_without_start(
) -> None:
    """If mark_started wasn't called, deadline is never reached."""
    clock = FakeClock()
    ctx = RunContext(settings=_cfg(user_deadline=1), clock=clock)
    clock.advance(1000)
    assert ctx.user_deadline_reached() is False
