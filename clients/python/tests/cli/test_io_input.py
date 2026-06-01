"""Tests for :mod:`efe_tool.cli.io_input`."""

from __future__ import annotations

import io
from pathlib import Path

import pytest

from efe_tool.cli.errors import UsageError
from efe_tool.cli.io_input import read_text_input, redact


class _FakeStream(io.StringIO):
    def __init__(self, content: str, is_tty: bool = False):
        super().__init__(content)
        self._is_tty = is_tty

    def isatty(self) -> bool:
        return self._is_tty


class TestResolutionHappy:
    def test_text_inline(self):
        assert read_text_input(text="hello") == "hello"

    def test_text_file(self, tmp_path: Path):
        p = tmp_path / "x.txt"
        p.write_text("hi there", encoding="utf-8")
        assert read_text_input(text_file=p) == "hi there"

    def test_stdin_explicit(self):
        s = _FakeStream("piped\n")
        assert read_text_input(use_stdin=True, stdin=s) == "piped"

    def test_stdin_implicit_when_not_tty(self):
        s = _FakeStream("piped", is_tty=False)
        assert read_text_input(stdin=s) == "piped"


class TestResolutionErrors:
    def test_no_source_with_tty_fails(self):
        s = _FakeStream("", is_tty=True)
        with pytest.raises(UsageError):
            read_text_input(stdin=s)

    def test_both_text_and_stdin_fails(self):
        with pytest.raises(UsageError):
            read_text_input(text="x", use_stdin=True,
                            stdin=_FakeStream(""))

    def test_missing_file(self, tmp_path: Path):
        with pytest.raises(UsageError):
            read_text_input(text_file=tmp_path / "nope.txt")

    def test_empty_stdin(self):
        s = _FakeStream("", is_tty=False)
        with pytest.raises(UsageError):
            read_text_input(use_stdin=True, stdin=s)

    def test_empty_text(self):
        with pytest.raises(UsageError):
            read_text_input(text="   ")


class TestEncoding:
    def test_utf8_accented(self):
        assert read_text_input(text="Ça va ?") == "Ça va ?"

    def test_file_non_utf8(self, tmp_path: Path):
        p = tmp_path / "latin1.txt"
        p.write_bytes("Café".encode("latin-1"))
        with pytest.raises(UsageError):
            read_text_input(text_file=p)


class TestLength:
    def test_max_length_enforced(self):
        from efe_tool.cli.metadata import MAX_TEXT_LENGTH
        big = "x" * (MAX_TEXT_LENGTH + 1)
        with pytest.raises(UsageError):
            read_text_input(text=big)


class TestRedact:
    def test_short_text(self):
        assert "len=5" in redact("hello")

    def test_long_text(self):
        s = "A" * 100
        out = redact(s)
        assert "len=100" in out
        assert len(out) < 100

    def test_empty(self):
        assert "empty" in redact("")


class _FakeLogger:
    def __init__(self):
        self.records: list[tuple[str, dict]] = []

    def warn(self, evt: str, **kw):
        self.records.append((evt, kw))

    # other levels unused
    def debug(self, *a, **kw): pass
    def info(self, *a, **kw): pass
    def error(self, *a, **kw): pass


class TestSensitivePatternWarning:
    def test_warns_on_password_keyword(self):
        log = _FakeLogger()
        read_text_input(text="my password = hunter2", logger=log)
        assert any(evt == "sensitive_text_on_cli" for evt, _ in log.records)

    def test_warns_on_long_opaque_token(self):
        log = _FakeLogger()
        read_text_input(
            text="here is the key abc" + "1" * 40,
            logger=log,
        )
        assert any(evt == "sensitive_text_on_cli" for evt, _ in log.records)

    def test_no_warn_on_normal_text(self):
        log = _FakeLogger()
        read_text_input(text="How do I reset my two-factor authentication?",
                        logger=log)
        assert not any(
            evt == "sensitive_text_on_cli" for evt, _ in log.records
        )
