"""Tests for :mod:`efe_worker.version`."""

from __future__ import annotations

import pytest

from efe_worker.version import VERSION, __version__, version_tuple


def test_version_shape() -> None:
    assert __version__ == VERSION
    assert len(VERSION.split(".")) == 3


def test_version_tuple() -> None:
    maj, mn, patch = version_tuple()
    assert isinstance(maj, int)
    assert isinstance(mn, int)
    assert isinstance(patch, int)
    assert maj >= 1


def test_version_tuple_rejects_malformed(monkeypatch: pytest.MonkeyPatch) -> None:
    import efe_worker.version as mod
    monkeypatch.setattr(mod, "VERSION", "bad.version")
    with pytest.raises(ValueError):
        mod.version_tuple()
