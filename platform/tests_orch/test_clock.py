"""Tests for :mod:`efe_orch.clock`."""

from __future__ import annotations

from datetime import datetime, timedelta, timezone

import pytest

from efe_orch.clock import FakeClock, SystemClock, ensure_utc


def test_system_clock_utc() -> None:
    ts = SystemClock().now()
    assert ts.tzinfo is not None
    assert ts.utcoffset() == timedelta(0)


def test_fake_clock_deterministic() -> None:
    c = FakeClock()
    t0 = c.now()
    t1 = c.advance(60)
    assert (t1 - t0) == timedelta(seconds=60)
    assert c.now() == t1


def test_fake_clock_set() -> None:
    c = FakeClock()
    c.set(datetime(2030, 5, 1, tzinfo=timezone.utc))
    assert c.now().year == 2030


def test_fake_clock_rejects_naive_epoch() -> None:
    with pytest.raises(ValueError):
        FakeClock(epoch=datetime(2030, 1, 1))


def test_fake_clock_rejects_backwards() -> None:
    c = FakeClock()
    with pytest.raises(ValueError):
        c.advance(-5)


def test_ensure_utc() -> None:
    t = datetime(2026, 1, 1, tzinfo=timezone.utc)
    assert ensure_utc(t) == t
    with pytest.raises(ValueError):
        ensure_utc(datetime(2026, 1, 1))
