"""Tests for :class:`EarlyStopping`."""

from __future__ import annotations

import pytest

from efe_worker.training.early_stop import EarlyStopping


def test_defaults() -> None:
    es = EarlyStopping("val/loss", patience=2)
    assert es.patience == 2
    assert es.mode == "min"
    assert es.best is None


def test_min_mode_improves() -> None:
    es = EarlyStopping("val/loss", patience=2, mode="min")
    assert es.step({"val/loss": 1.0}, at_step=1) is False
    assert es.best == 1.0
    assert es.step({"val/loss": 0.9}, at_step=2) is False
    assert es.best == 0.9


def test_min_mode_patience_exhausted() -> None:
    es = EarlyStopping("val/loss", patience=2)
    es.step({"val/loss": 1.0}, at_step=1)
    assert es.step({"val/loss": 1.1}, at_step=2) is False
    assert es.step({"val/loss": 1.2}, at_step=3) is True


def test_max_mode() -> None:
    es = EarlyStopping("val/f1", patience=1, mode="max")
    assert es.step({"val/f1": 0.5}, at_step=1) is False
    assert es.step({"val/f1": 0.4}, at_step=2) is True
    assert es.best == 0.5


def test_missing_metric_no_update() -> None:
    es = EarlyStopping("val/loss", patience=1)
    es.step({"val/loss": 0.5}, at_step=1)
    # metric missing — no improvement signal, no bad bump.
    result = es.step({"other": 0.1}, at_step=2)
    assert result is False


def test_bad_count_observable() -> None:
    es = EarlyStopping("val/loss", patience=3)
    es.step({"val/loss": 1.0}, at_step=1)
    es.step({"val/loss": 1.1}, at_step=2)
    assert es.bad_count == 1


def test_reset() -> None:
    es = EarlyStopping("val/loss", patience=1)
    es.step({"val/loss": 1.0}, at_step=1)
    es.reset()
    assert es.best is None
    assert es.best_step == -1


def test_invalid_mode() -> None:
    with pytest.raises(ValueError):
        EarlyStopping("x", patience=1, mode="galaxy")  # type: ignore[arg-type]


def test_invalid_patience() -> None:
    with pytest.raises(ValueError):
        EarlyStopping("x", patience=0)


def test_best_step_tracked() -> None:
    es = EarlyStopping("val/loss", patience=3)
    es.step({"val/loss": 1.0}, at_step=5)
    es.step({"val/loss": 0.9}, at_step=10)
    assert es.best_step == 10


def test_min_delta() -> None:
    es = EarlyStopping("val/loss", patience=1, min_delta=0.1)
    es.step({"val/loss": 1.0}, at_step=1)
    # 0.95 is an improvement but not beyond min_delta
    assert es.step({"val/loss": 0.95}, at_step=2) is True
