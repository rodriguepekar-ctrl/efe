"""Multi-scale window computation (1h / 1d / 7d)."""

from __future__ import annotations

from datetime import datetime, timedelta, timezone

import pytest

from efe_drift.detection.sequential import compute_scale_windows


def test_compute_scale_windows_aligns_to_minute():
    now = datetime(2026, 4, 10, 14, 37, 42, 500000, tzinfo=timezone.utc)
    windows = compute_scale_windows(now)
    assert len(windows) == 3
    w1h, w1d, w7d = windows
    assert w1h.label == "1h"
    assert w1h.end == datetime(2026, 4, 10, 14, 37, tzinfo=timezone.utc)
    assert w1h.start == w1h.end - timedelta(hours=1)
    assert w1d.start == w1d.end - timedelta(days=1)
    assert w7d.start == w7d.end - timedelta(days=7)


def test_compute_scale_windows_subset():
    now = datetime(2026, 4, 10, 14, 0, tzinfo=timezone.utc)
    windows = compute_scale_windows(now, enabled=("1h",))
    assert len(windows) == 1
    assert windows[0].label == "1h"


def test_compute_scale_windows_rejects_naive_datetime():
    with pytest.raises(ValueError):
        compute_scale_windows(datetime(2026, 4, 10))


def test_compute_scale_windows_unknown_label_raises():
    with pytest.raises(ValueError):
        compute_scale_windows(
            datetime(2026, 4, 10, tzinfo=timezone.utc),
            enabled=("1h", "1m"),  # type: ignore[arg-type]
        )
