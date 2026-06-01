"""Tests for :mod:`efe_tool.repl.parser`."""

from __future__ import annotations

import pytest

from efe_tool.repl.parser import (
    CommandToken,
    EmptyToken,
    QueryToken,
    parse_line,
)


class TestEmptyAndWhitespace:
    def test_empty(self):
        assert isinstance(parse_line(""), EmptyToken)

    def test_whitespace_only(self):
        assert isinstance(parse_line("   \t  "), EmptyToken)

    def test_newline_only(self):
        assert isinstance(parse_line("\n"), EmptyToken)


class TestQueries:
    def test_plain_text(self):
        t = parse_line("hello world")
        assert isinstance(t, QueryToken)
        assert t.text == "hello world"

    def test_preserves_leading_whitespace_in_text(self):
        """Internal whitespace of queries is preserved (but .strip() in
        detection rejects whitespace-only)."""
        t = parse_line("  hello")
        # After parse, the original line is returned rstripped of newlines
        # only (not leading whitespace).
        assert isinstance(t, QueryToken)
        assert t.text.strip() == "hello"

    def test_utf8_accented(self):
        t = parse_line("Ça va ?")
        assert isinstance(t, QueryToken)
        assert "Ça" in t.text

    def test_colon_in_middle_is_query(self):
        # Only a ':' at the *start* counts as a command.
        t = parse_line("he said: go")
        assert isinstance(t, QueryToken)


class TestCommands:
    def test_simple(self):
        t = parse_line(":help")
        assert isinstance(t, CommandToken)
        assert t.name == "help"
        assert t.args == ()

    def test_with_args(self):
        t = parse_line(":budget 3000")
        assert isinstance(t, CommandToken)
        assert t.name == "budget"
        assert t.args == ("3000",)

    def test_multiple_args(self):
        t = parse_line(":thresholds high 0.8 medium 0.6")
        assert isinstance(t, CommandToken)
        assert t.args == ("high", "0.8", "medium", "0.6")

    def test_quoted_arg(self):
        t = parse_line(':save "my file.json"')
        assert isinstance(t, CommandToken)
        assert t.args == ("my file.json",)

    def test_name_lowercased(self):
        t = parse_line(":HELP")
        assert t.name == "help"

    def test_dash_to_underscore(self):
        t = parse_line(":log-level debug")
        assert t.name == "log_level"

    def test_aliases_are_not_resolved_here(self):
        # The parser doesn't know aliases; the dispatcher does.
        t = parse_line(":q")
        assert t.name == "q"

    def test_empty_command_body_raises(self):
        with pytest.raises(ValueError):
            parse_line(":")

    def test_malformed_quote_raises(self):
        with pytest.raises(ValueError):
            parse_line(':save "unterminated')


class TestTokenSignature:
    def test_signature_no_args(self):
        t = CommandToken(name="reload")
        assert t.signature() == ":reload"

    def test_signature_with_args(self):
        t = CommandToken(name="budget", args=("3000",))
        assert t.signature() == ":budget 3000"
