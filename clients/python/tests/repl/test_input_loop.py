"""Tests for :mod:`efe_tool.repl.input_loop`.

We focus on the parts we can drive without sockets or real I/O:
line reading (including multi-line and backslash continuation),
query execution against a fake client, and error recovery in the
main loop.
"""

from __future__ import annotations

import io
from types import SimpleNamespace

import pytest

from efe_tool.cli.errors import CliError, UsageError
from efe_tool.repl.input_loop import read_line, run_query


class _FakeStream(io.StringIO):
    def __init__(self, text: str, is_tty: bool = False):
        super().__init__(text)
        self._tty = is_tty

    def isatty(self) -> bool:
        return self._tty


# ---------------------------------------------------------------------------
#  read_line — batch mode
# ---------------------------------------------------------------------------


class TestReadLineBatch:
    def test_single_line(self, state):
        stdin = _FakeStream("hello\n")
        stdout = _FakeStream("", is_tty=False)
        out = read_line(state, prompt_template="EFE> ",
                        stdin=stdin, stdout=stdout)
        assert out == "hello"

    def test_eof_returns_none(self, state):
        stdin = _FakeStream("")
        stdout = _FakeStream("", is_tty=False)
        assert read_line(state, prompt_template="EFE> ",
                         stdin=stdin, stdout=stdout) is None

    def test_backslash_continuation(self, state):
        stdin = _FakeStream("first \\\nsecond\n")
        stdout = _FakeStream("", is_tty=False)
        out = read_line(state, prompt_template="EFE> ",
                        stdin=stdin, stdout=stdout)
        assert out == "first \nsecond"

    def test_triple_quote_inline_block(self, state):
        stdin = _FakeStream('"""hello"""\n')
        stdout = _FakeStream("", is_tty=False)
        out = read_line(state, prompt_template="EFE> ",
                        stdin=stdin, stdout=stdout)
        assert out == "hello"

    def test_triple_quote_multiline(self, state):
        stdin = _FakeStream('"""\nline1\nline2\n"""\n')
        stdout = _FakeStream("", is_tty=False)
        out = read_line(state, prompt_template="EFE> ",
                        stdin=stdin, stdout=stdout)
        assert "line1" in out
        assert "line2" in out


# ---------------------------------------------------------------------------
#  run_query against a fake client
# ---------------------------------------------------------------------------


class TestRunQuery:
    def test_basic_success(self, state_with_fakes, noop_log):
        out = run_query("hello", state_with_fakes, noop_log)
        assert "alpha" in out  # fake client returns label "alpha"
        assert state_with_fakes.queries_count == 1
        assert state_with_fakes.label_counts == {"alpha": 1}

    def test_no_catalog_raises(self, state, noop_log):
        state.client = SimpleNamespace()
        with pytest.raises(UsageError):
            run_query("hi", state, noop_log)

    def test_no_client_raises(self, state, fake_catalog, noop_log):
        state.catalog = fake_catalog
        state.client = None
        with pytest.raises(UsageError):
            run_query("hi", state, noop_log)

    def test_none_embedder_at_boot_keeps_client_none(self, cfg, monkeypatch):
        """Regression: when the embedder fails to load at boot, the REPL
        must NOT construct a client with embedder=None (which would
        crash later at classify time). Instead, state.client stays
        None and run_query reports a clean UsageError."""
        from efe_tool import embedder as emb_mod
        from efe_tool.errors import ConfigError

        def _raise(model):
            raise ConfigError("sentence-transformers missing")

        monkeypatch.setattr(emb_mod, "LocalSTEmbedder", _raise)

        # Driving the full boot surfaces the bug by calling run_query
        # after boot. We just check that state.client stays None.
        from efe_tool.repl.input_loop import _boot_resources
        from efe_tool.repl.session import SessionState
        state = SessionState.from_boot(cfg)
        _boot_resources(state, cfg, _FakeLog(), bootstrap_catalog=False)
        assert state.embedder is None
        assert state.client is None


class _FakeLog:
    def debug(self, *a, **kw): pass
    def info(self, *a, **kw): pass
    def warn(self, *a, **kw): pass
    def error(self, *a, **kw): pass

    def test_bumps_timeout_when_too_small(self, state_with_fakes, noop_log):
        state_with_fakes.budget_ms = 10000
        state_with_fakes.timeout_s = 2.0
        run_query("hi", state_with_fakes, noop_log)
        # 10000 / 1000 + 0.5 = 10.5
        assert state_with_fakes.timeout_s == 10.5

    def test_json_format_returns_json(self, state_with_fakes, noop_log):
        state_with_fakes.output_format = "json"
        out = run_query("hi", state_with_fakes, noop_log)
        import json
        doc = json.loads(out)
        assert doc["label"] == "alpha"

    def test_records_transcript(self, state_with_fakes, noop_log):
        run_query("hi", state_with_fakes, noop_log)
        assert state_with_fakes.transcript[-1]["kind"] == "query"
        assert state_with_fakes.transcript[-1]["text"] == "hi"

    def test_strict_flag_marker_in_human(self, state_with_fakes, noop_log):
        state_with_fakes.strict = True
        state_with_fakes.client.confidence = 0.3  # below high threshold
        out = run_query("hi", state_with_fakes, noop_log)
        # Result still returned; session continues; marker visible.
        assert "⚠" in out or "strict" in out


# ---------------------------------------------------------------------------
#  Full loop integration (manual drive)
# ---------------------------------------------------------------------------


class TestRunSessionBatch:
    def test_help_then_quit(self, cfg):
        """Drive ``run_session`` in batch mode with stdin piped."""
        from efe_tool.repl.input_loop import run_session

        stdin = _FakeStream(":help\n:quit\n")
        stdout = _FakeStream("", is_tty=False)
        stderr = _FakeStream("", is_tty=False)

        rc = run_session(
            cfg, stdin=stdin, stdout=stdout, stderr=stderr,
            history_enabled=False, bootstrap_catalog=False,
        )
        assert rc == 0
        assert ":help" in stdout.getvalue()

    def test_invalid_command_does_not_crash(self, cfg):
        from efe_tool.repl.input_loop import run_session

        stdin = _FakeStream(":nopecommand\n:quit\n")
        stdout = _FakeStream("", is_tty=False)
        stderr = _FakeStream("", is_tty=False)

        rc = run_session(
            cfg, stdin=stdin, stdout=stdout, stderr=stderr,
            history_enabled=False, bootstrap_catalog=False,
        )
        assert rc == 0
        assert "unknown command" in stderr.getvalue()

    def test_empty_lines_are_ignored(self, cfg):
        from efe_tool.repl.input_loop import run_session

        stdin = _FakeStream("\n\n:quit\n")
        stdout = _FakeStream("", is_tty=False)
        stderr = _FakeStream("", is_tty=False)

        rc = run_session(
            cfg, stdin=stdin, stdout=stdout, stderr=stderr,
            history_enabled=False, bootstrap_catalog=False,
        )
        assert rc == 0
