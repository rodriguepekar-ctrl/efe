"""Tests for the observability layer."""

from __future__ import annotations

from efe_gate.observability import get_logger, init_logging, noop_registry


def test_noop_registry_records() -> None:
    r = noop_registry()
    r.eval_total.inc(1.0, tenant="acme", task="routing", decision="promote")
    r.primary_metric_delta.set(0.01,
                                 tenant="acme", task="routing", metric="f1")
    snap = r.snapshot()
    assert "efe_gate_eval_total" in snap


def test_logger_roundtrip() -> None:
    init_logging(level="WARNING", json=False)
    log = get_logger("efe_gate.test")
    log.info("hello", run_id="rid")
    log.warning("careful")
    child = log.bind(tenant="acme")
    child.error("boom", code="x")
