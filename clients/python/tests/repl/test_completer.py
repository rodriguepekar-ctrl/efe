"""Tests for :mod:`efe_tool.repl.completer`."""

from __future__ import annotations

import pytest

from efe_tool.repl.completer import Completer


# readline context is shared process-wide, but the tests that exercise
# readline via ``get_line_buffer`` mock it; otherwise we fall back to
# simple prefix matching on ``text`` alone.


class TestCommandNameCompletion:
    def test_prefix_matches(self, monkeypatch):
        c = Completer(
            command_names=("reload", "reset", "rounds", "quit"),
        )
        # Fake readline buffer
        import readline  # type: ignore
        monkeypatch.setattr(readline, "get_line_buffer", lambda: ":re")
        monkeypatch.setattr(readline, "get_begidx", lambda: 0)
        assert c.complete(":re", 0) == ":reload"
        assert c.complete(":re", 1) == ":reset"
        assert c.complete(":re", 2) is None

    def test_no_match(self, monkeypatch):
        c = Completer(command_names=("reload",))
        import readline
        monkeypatch.setattr(readline, "get_line_buffer", lambda: ":xyz")
        monkeypatch.setattr(readline, "get_begidx", lambda: 0)
        assert c.complete(":xyz", 0) is None


class TestEnumCompletion:
    def test_format_values(self, monkeypatch):
        c = Completer(
            command_names=("format",),
            enum_values={"format": ("human", "json", "table", "text")},
        )
        import readline
        monkeypatch.setattr(readline, "get_line_buffer", lambda: ":format j")
        monkeypatch.setattr(readline, "get_begidx", lambda: 8)
        assert c.complete("j", 0) == "json"

    def test_log_values(self, monkeypatch):
        c = Completer(
            command_names=("log",),
            enum_values={"log": ("debug", "info", "warn", "error")},
        )
        import readline
        monkeypatch.setattr(readline, "get_line_buffer", lambda: ":log e")
        monkeypatch.setattr(readline, "get_begidx", lambda: 5)
        assert c.complete("e", 0) == "error"


class TestPathCompletion:
    def test_path_arg_completes_files(self, tmp_path, monkeypatch):
        # Create a couple of files
        (tmp_path / "alpha.yaml").write_text("", encoding="utf-8")
        (tmp_path / "alphabet.yaml").write_text("", encoding="utf-8")
        (tmp_path / "beta.txt").write_text("", encoding="utf-8")

        c = Completer(
            command_names=("catalog",),
            path_args=("catalog",),
        )
        # Simulate ":catalog <tmp_path>/alp"
        prefix = f"{tmp_path}/alp"
        import readline
        buf = f":catalog {prefix}"
        monkeypatch.setattr(readline, "get_line_buffer", lambda: buf)
        monkeypatch.setattr(readline, "get_begidx", lambda: buf.index(prefix))
        c.complete(prefix, 0)
        matches = c._matches  # internal list, order sorted
        assert any("alpha.yaml" in m for m in matches)
        assert any("alphabet.yaml" in m for m in matches)
        assert not any("beta" in m for m in matches)


class TestFreeText:
    def test_no_completion_for_free_text(self, monkeypatch):
        c = Completer(command_names=("help",))
        import readline
        monkeypatch.setattr(readline, "get_line_buffer", lambda: "hello")
        monkeypatch.setattr(readline, "get_begidx", lambda: 0)
        assert c.complete("hello", 0) is None
