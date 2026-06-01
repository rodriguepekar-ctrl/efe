"""Tests for :mod:`efe_worker.observability`."""

from __future__ import annotations

from efe_worker.observability import get_logger, init_logging, noop_registry


def test_noop_registry() -> None:
    r = noop_registry()
    r.training_step_loss.observe(0.5, method="infonce")
    r.eval_metric_value.set(0.9, method="infonce", metric="recall@1")
    r.heartbeat_total.inc(1.0, outcome="ok")
    snap = r.snapshot()
    assert "efe_worker_training_step_loss" in snap


def test_init_logging_does_not_raise() -> None:
    init_logging(level="WARNING", json=False)


def test_get_logger_has_expected_methods() -> None:
    log = get_logger("test")
    log.info("hello", foo="bar")
    log.warning("warn")
    log.error("err", code=1)
    log.debug("dbg")
    child = log.bind(run_id="r")
    child.info("bound")
