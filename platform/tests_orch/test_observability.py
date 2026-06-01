"""Tests for :mod:`efe_orch.observability`."""

from __future__ import annotations

from efe_orch.observability import get_logger, init_logging, noop_registry


def test_noop_registry_snapshot_roundtrips() -> None:
    reg = noop_registry()
    reg.runs_submitted.inc(1.0, tenant="acme", pool="cpu-small")
    reg.run_duration_seconds.observe(42.0, pool="cpu-small")
    reg.runs_in_state.set(3.0, state="queued")
    snap = reg.snapshot()
    assert "efe_orch_runs_submitted_total" in snap
    assert snap["efe_orch_runs_submitted_total"] == [
        ("inc", 1.0, {"tenant": "acme", "pool": "cpu-small"}),
    ]


def test_get_logger_returns_something() -> None:
    init_logging(level="WARNING", json=False)
    log = get_logger("test")
    log.info("hello", foo="bar")
    log.warning("careful")


def test_logger_bind_and_info() -> None:
    log = get_logger("test")
    child = log.bind(run_id="abc")
    child.info("x")
