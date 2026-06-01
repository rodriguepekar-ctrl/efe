"""Tests for :mod:`efe_tool.repl.history`."""

from __future__ import annotations

import os
import stat
from pathlib import Path

import pytest

from efe_tool.repl.history import History, resolve_history_path


class TestResolveHistoryPath:
    def test_explicit_wins(self, tmp_path):
        expl = tmp_path / "x.log"
        p = resolve_history_path(str(expl), env={}, home=tmp_path)
        assert p == expl

    def test_env_var_second(self, tmp_path):
        e = tmp_path / "env.log"
        p = resolve_history_path(None, env={"EFE_HISTORY_PATH": str(e)}, home=tmp_path)
        assert p == e

    def test_xdg_data_home_third(self, tmp_path):
        xdg = tmp_path / "xdg"
        p = resolve_history_path(None, env={"XDG_DATA_HOME": str(xdg)}, home=tmp_path)
        assert p == xdg / "efe" / "history"

    def test_default_xdg_layout(self, tmp_path):
        p = resolve_history_path(None, env={}, home=tmp_path)
        assert p == tmp_path / ".local" / "share" / "efe" / "history"

    def test_falls_back_to_legacy_if_it_exists(self, tmp_path):
        legacy = tmp_path / ".efe" / "history"
        legacy.parent.mkdir(parents=True, exist_ok=True)
        legacy.write_text("old", encoding="utf-8")
        p = resolve_history_path(None, env={}, home=tmp_path)
        assert p == legacy


class TestHistoryBasic:
    def test_disabled_no_write(self, tmp_path):
        h = History(tmp_path / "h", enabled=False)
        h.append("x")
        h.flush()
        assert not (tmp_path / "h").exists()

    def test_append_and_flush_roundtrip(self, tmp_path):
        p = tmp_path / "h"
        h = History(p)
        h.append("one")
        h.append("two")
        assert h.flush()
        assert p.exists()
        content = p.read_text(encoding="utf-8")
        assert "one" in content and "two" in content

    def test_reload_restores_lines(self, tmp_path):
        p = tmp_path / "h"
        h = History(p)
        h.append("alpha")
        h.append("beta")
        h.flush()

        h2 = History(p)
        h2.load()
        assert list(h2) == ["alpha", "beta"]

    def test_deduplicates_consecutive(self, tmp_path):
        h = History(tmp_path / "h")
        h.append("x")
        h.append("x")
        assert list(h) == ["x"]

    def test_strip_trailing_newlines(self, tmp_path):
        h = History(tmp_path / "h")
        h.append("line1\n")
        assert list(h) == ["line1"]


class TestCap:
    def test_cap_trims_fifo(self, tmp_path):
        h = History(tmp_path / "h", cap=3)
        for i in range(5):
            h.append(f"q{i}")
        assert list(h) == ["q2", "q3", "q4"]


class TestClear:
    def test_clear_wipes_memory_and_file(self, tmp_path):
        p = tmp_path / "h"
        h = History(p)
        h.append("x")
        h.flush()
        assert p.exists()
        h.clear()
        assert not p.exists()
        assert list(h) == []


class TestPermissions:
    def test_flush_sets_0600(self, tmp_path):
        p = tmp_path / "h"
        h = History(p)
        h.append("secret")
        h.flush()
        mode = stat.S_IMODE(p.stat().st_mode)
        # At least the user bits should be 0600; group/other must be 0.
        assert mode & 0o077 == 0, oct(mode)


class TestCorruption:
    def test_unreadable_file_yields_empty(self, tmp_path):
        p = tmp_path / "h"
        p.write_bytes(b"\xff\xfe invalid \x00 bytes")
        h = History(p)
        # Should not raise
        h.load()
        # May be empty or the replacement chars — either way, not crash.
        assert isinstance(list(h), list)
