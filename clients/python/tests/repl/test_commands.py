"""Unit tests for the REPL command registry and every tuning /
output / logging / session command.

These tests do not require a running server: the fake client and
fake catalog (see ``conftest.py``) stand in for the real thing.
"""

from __future__ import annotations

import pytest

from efe_tool.cli.errors import CliError, UsageError
from efe_tool.repl.commands import (
    REGISTRY,
    canonical_names,
    dispatch,
)
from efe_tool.repl.commands.session import QuitSignal


# ---------------------------------------------------------------------------
#  Registry invariants
# ---------------------------------------------------------------------------


class TestRegistry:
    def test_26_canonical_commands(self):
        # Architecture §23.B: 26 commands (with some aliases on top).
        assert len(canonical_names()) == 26

    def test_aliases_resolve_to_same_entry(self):
        assert REGISTRY["quit"] is REGISTRY["exit"]
        assert REGISTRY["quit"] is REGISTRY["q"]

    def test_unknown_name_raises_with_suggestion(self, state, noop_log):
        with pytest.raises(UsageError) as exc_info:
            dispatch("relod", [], state, noop_log)
        assert ":reload" in (exc_info.value.hint or "")

    def test_needs_catalog_enforced(self, state, noop_log):
        # :reload needs a catalog; state has none yet
        with pytest.raises(UsageError):
            dispatch("reload", [], state, noop_log)


# ---------------------------------------------------------------------------
#  :help
# ---------------------------------------------------------------------------


class TestHelp:
    def test_lists_all_groups(self, state, noop_log):
        out = dispatch("help", [], state, noop_log)
        assert "[session]" in out
        assert "[catalog]" in out

    def test_help_specific_command(self, state, noop_log):
        out = dispatch("help", ["reload"], state, noop_log)
        assert "reload" in out.lower()
        assert "requires" in out.lower() or "catalog" in out.lower()

    def test_help_unknown_raises(self, state, noop_log):
        with pytest.raises(UsageError):
            dispatch("help", ["nope"], state, noop_log)


# ---------------------------------------------------------------------------
#  :quit
# ---------------------------------------------------------------------------


class TestQuit:
    def test_raises_quitsignal(self, state, noop_log):
        with pytest.raises(QuitSignal):
            dispatch("quit", [], state, noop_log)

    def test_q_alias(self, state, noop_log):
        with pytest.raises(QuitSignal):
            dispatch("q", [], state, noop_log)


# ---------------------------------------------------------------------------
#  :info / :version / :stats
# ---------------------------------------------------------------------------


class TestSessionInfo:
    def test_info_has_essentials(self, state, noop_log):
        out = dispatch("info", [], state, noop_log)
        assert "session" in out and "endpoint" in out

    def test_version(self, state, noop_log):
        out = dispatch("version", [], state, noop_log)
        assert "efe-tool" in out

    def test_stats_empty(self, state, noop_log):
        out = dispatch("stats", [], state, noop_log)
        assert "Queries:" in out


# ---------------------------------------------------------------------------
#  :budget / :timeout / :rounds
# ---------------------------------------------------------------------------


class TestTuning:
    def test_budget_show_and_set(self, state, noop_log):
        show = dispatch("budget", [], state, noop_log)
        assert "5000" in show
        dispatch("budget", ["8000"], state, noop_log)
        assert state.budget_ms == 8000

    def test_budget_reset(self, state, noop_log):
        state.budget_ms = 9999
        dispatch("budget", ["reset"], state, noop_log)
        assert state.budget_ms == 5000

    def test_budget_invalid_raises(self, state, noop_log):
        with pytest.raises(UsageError):
            dispatch("budget", ["-1"], state, noop_log)

    def test_timeout_set(self, state, noop_log):
        dispatch("timeout", ["3.5"], state, noop_log)
        assert state.timeout_s == 3.5

    def test_rounds_bounds(self, state, noop_log):
        with pytest.raises(UsageError):
            dispatch("rounds", ["0"], state, noop_log)
        with pytest.raises(UsageError):
            dispatch("rounds", ["1000"], state, noop_log)


class TestStrict:
    def test_on_off(self, state, noop_log):
        dispatch("strict", ["on"], state, noop_log)
        assert state.strict is True
        dispatch("strict", ["off"], state, noop_log)
        assert state.strict is False


# ---------------------------------------------------------------------------
#  :thresholds
# ---------------------------------------------------------------------------


class TestThresholds:
    def test_show_needs_catalog(self, state, noop_log):
        out = dispatch("thresholds", [], state, noop_log)
        assert "catalog not loaded" in out

    def test_override_high_and_medium(self, state, fake_catalog, noop_log):
        state.catalog = fake_catalog
        dispatch("thresholds", ["high", "0.9", "medium", "0.6"],
                 state, noop_log)
        assert state.threshold_high_override == 0.9
        assert state.threshold_medium_override == 0.6

    def test_reset(self, state, fake_catalog, noop_log):
        state.catalog = fake_catalog
        state.threshold_high_override = 0.9
        dispatch("thresholds", ["reset"], state, noop_log)
        assert state.threshold_high_override is None

    def test_out_of_order_raises(self, state, fake_catalog, noop_log):
        state.catalog = fake_catalog
        with pytest.raises(UsageError):
            dispatch("thresholds",
                     ["high", "0.3", "medium", "0.7"],
                     state, noop_log)


# ---------------------------------------------------------------------------
#  :format / :pretty / :diagnostics
# ---------------------------------------------------------------------------


class TestOutput:
    def test_format_change(self, state, noop_log):
        dispatch("format", ["json"], state, noop_log)
        assert state.output_format == "json"

    def test_format_invalid(self, state, noop_log):
        with pytest.raises(UsageError):
            dispatch("format", ["csv"], state, noop_log)

    def test_pretty_toggle(self, state, noop_log):
        dispatch("pretty", ["on"], state, noop_log)
        assert state.pretty is True

    def test_diagnostics_toggle(self, state, noop_log):
        dispatch("diagnostics", ["on"], state, noop_log)
        assert state.diagnostics is True


# ---------------------------------------------------------------------------
#  :history
# ---------------------------------------------------------------------------


class TestHistoryCmd:
    def test_empty(self, state, noop_log):
        out = dispatch("history", [], state, noop_log)
        assert "empty" in out

    def test_shows_recent(self, state, noop_log):
        state.history = ["q1", "q2", "q3", ":help"]
        out = dispatch("history", [], state, noop_log)
        assert ":help" in out
        assert "q1" in out

    def test_clear(self, state, noop_log):
        state.history = ["a", "b"]
        dispatch("history", ["clear"], state, noop_log)
        assert state.history == []

    def test_invalid_arg(self, state, noop_log):
        with pytest.raises(UsageError):
            dispatch("history", ["abc"], state, noop_log)


# ---------------------------------------------------------------------------
#  :save
# ---------------------------------------------------------------------------


class TestSaveCmd:
    def test_requires_path(self, state, noop_log):
        with pytest.raises(UsageError):
            dispatch("save", [], state, noop_log)

    def test_writes_transcript_file(self, tmp_path, state, noop_log):
        state.record_command("help", [], outcome="ok")
        target = tmp_path / "session.json"
        msg = dispatch("save", [str(target)], state, noop_log)
        assert target.exists()
        import json
        doc = json.loads(target.read_text(encoding="utf-8"))
        assert doc["schema_version"] == 1
        assert doc["entries"][0]["kind"] == "command"
        assert "saved" in msg

    def test_transcript_started_at_is_past_not_now(
        self, tmp_path, state, noop_log,
    ):
        """Regression: ``_cmd_save`` used to call ``_iso(started_at,
        monotonic=True)`` which silently used ``time.time()`` and produced
        an ended_at-equivalent string. Now ``started_at`` must reflect
        the *actual* boot time via ``state.started_at_wall``."""
        import time
        # Force the session boot time to be 1 hour ago.
        state.started_at_wall = time.time() - 3600.0
        target = tmp_path / "session.json"
        dispatch("save", [str(target)], state, noop_log)
        import json
        doc = json.loads(target.read_text(encoding="utf-8"))
        # started_at must be ~3600s before ended_at (within generous margin).
        from datetime import datetime
        sa = datetime.strptime(doc["started_at"], "%Y-%m-%dT%H:%M:%SZ")
        ea = datetime.strptime(doc["ended_at"], "%Y-%m-%dT%H:%M:%SZ")
        delta = (ea - sa).total_seconds()
        assert 3590 < delta < 3610, (doc["started_at"], doc["ended_at"])


# ---------------------------------------------------------------------------
#  :endpoint (uses fake client, tests plumbing only)
# ---------------------------------------------------------------------------


class TestEndpointCmd:
    def test_show(self, state, noop_log):
        out = dispatch("endpoint", [], state, noop_log)
        assert state.endpoint in out

    def test_invalid_endpoint_rejected(self, state, noop_log):
        with pytest.raises(UsageError):
            dispatch("endpoint", ["http://foo"], state, noop_log)


# ---------------------------------------------------------------------------
#  :classify explicit
# ---------------------------------------------------------------------------


class TestClassifyCmd:
    def test_needs_catalog(self, state, noop_log):
        with pytest.raises(UsageError):
            dispatch("classify", ["hello"], state, noop_log)

    def test_needs_text(self, state, fake_catalog, noop_log):
        state.catalog = fake_catalog
        with pytest.raises(UsageError):
            dispatch("classify", [], state, noop_log)


# ---------------------------------------------------------------------------
#  :log / :quiet / :verbose
# ---------------------------------------------------------------------------


class TestEmbedderCmd:
    """Regression tests for the ``:embedder`` swap. The previous version
    left the client pointing at the old embedder, so the fingerprint
    check in :meth:`EfeClient.classify` kept using the stale value."""

    def test_show_without_args(self, state_with_fakes, noop_log):
        out = dispatch("embedder", [], state_with_fakes, noop_log)
        assert state_with_fakes.embedder_model in out

    def test_swap_rebuilds_client(self, state_with_fakes, noop_log,
                                  monkeypatch):
        """After ``:embedder MODEL``, the client must be rebuilt so it
        references the new embedder (not the one captured at boot)."""
        from efe_tool import client as client_mod
        from efe_tool import embedder as emb_mod

        created_clients: list = []
        closed_clients: list = []

        class _FakeEmbedder:
            def __init__(self, name): self.name = name
            def fingerprint(self): return f"fake/{self.name}@4"
            def dim(self): return 4

        class _FakeClient:
            def __init__(self, **kw):
                self.kw = kw
                created_clients.append(self)
            def close(self):
                closed_clients.append(self)

        monkeypatch.setattr(emb_mod, "LocalSTEmbedder", _FakeEmbedder)
        monkeypatch.setattr(client_mod, "EfeClient", _FakeClient)

        # Replace the fake client with the tracked fake and record baseline.
        state_with_fakes.client = _FakeClient()
        created_clients.clear()

        dispatch("embedder", ["new-model"], state_with_fakes, noop_log)

        # The old client must have been closed and a new one built.
        assert len(closed_clients) >= 1
        assert len(created_clients) == 1
        # And the session's client now points at the fresh instance.
        assert state_with_fakes.client is created_clients[-1]
        # New embedder name is recorded.
        assert state_with_fakes.embedder_model == "new-model"


class TestThresholdsAffectVerdict:
    """Regression: the human-format verdict must apply the session's
    effective thresholds, including overrides set via ``:thresholds``."""

    def test_override_high_turns_trusted_into_warn(
        self, state_with_fakes, fake_catalog, noop_log,
    ):
        from efe_tool.repl.input_loop import run_query
        state_with_fakes.fake_catalog = fake_catalog  # kept for clarity
        # With defaults (high=0.80), a confidence of 0.70 is "warn".
        state_with_fakes.client.confidence = 0.70
        out = run_query("hi", state_with_fakes, noop_log)
        assert "warn" in out

        # Push confidence above high → "trusted" without override.
        state_with_fakes.client.confidence = 0.90
        out2 = run_query("hi again", state_with_fakes, noop_log)
        assert "trusted" in out2

        # Now raise the high threshold so 0.90 falls below it → "warn".
        dispatch("thresholds", ["high", "0.95", "medium", "0.80"],
                 state_with_fakes, noop_log)
        state_with_fakes.client.confidence = 0.90
        out3 = run_query("third", state_with_fakes, noop_log)
        assert "warn" in out3


class TestLogCmd:
    def test_log_level(self, state, noop_log):
        dispatch("log", ["warn"], state, noop_log)
        assert state.log_level == "warn"

    def test_log_off_maps_to_error(self, state, noop_log):
        dispatch("log", ["off"], state, noop_log)
        assert state.log_level == "error"

    def test_quiet_alias(self, state, noop_log):
        dispatch("quiet", [], state, noop_log)
        assert state.log_level == "error"

    def test_verbose_alias(self, state, noop_log):
        dispatch("verbose", [], state, noop_log)
        assert state.log_level == "debug"

    def test_unknown_level(self, state, noop_log):
        with pytest.raises(UsageError):
            dispatch("log", ["LOUD"], state, noop_log)
