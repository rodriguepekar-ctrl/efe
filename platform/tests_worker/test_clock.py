"""Tests for :mod:`efe_worker.clock`."""

from __future__ import annotations

from datetime import datetime, timedelta, timezone

import pytest

from efe_worker.clock import FakeClock, SystemClock


def test_system_clock_utc() -> None:
    t = SystemClock().now()
    assert t.tzinfo is not None


def test_system_clock_monotonic() -> None:
    clock = SystemClock()
    a = clock.monotonic()
    b = clock.monotonic()
    assert b >= a


def test_fake_clock_default_epoch() -> None:
    c = FakeClock()
    assert c.now().year == 2026
    assert c.monotonic() == 0.0


def test_fake_clock_advance() -> None:
    c = FakeClock()
    base = c.now()
    t = c.advance(60)
    assert (t - base) == timedelta(seconds=60)
    assert c.monotonic() == 60.0


def test_fake_clock_rejects_naive_epoch() -> None:
    with pytest.raises(ValueError):
        FakeClock(epoch=datetime(2030, 1, 1))


def test_fake_clock_rejects_backward() -> None:
    with pytest.raises(ValueError):
        FakeClock().advance(-1.0)


def test_fake_clock_custom_monotonic() -> None:
    c = FakeClock(
        epoch=datetime(2026, 6, 1, tzinfo=timezone.utc),
        monotonic=100.0,
    )
    assert c.monotonic() == 100.0
    assert c.now().month == 6
