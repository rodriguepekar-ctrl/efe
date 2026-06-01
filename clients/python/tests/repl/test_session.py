"""Tests for :mod:`efe_tool.repl.session`."""

from __future__ import annotations

from types import SimpleNamespace

from efe_tool.repl.session import SessionState


class TestFromBoot:
    def test_basic_fields_copied(self, cfg):
        s = SessionState.from_boot(cfg)
        assert s.endpoint == cfg.endpoint
        assert s.budget_ms == cfg.budget_ms
        assert s.timeout_s == cfg.timeout_s
        assert s.max_rounds == cfg.max_rounds
        assert s.strict == cfg.strict
        assert s.boot_cfg is cfg

    def test_output_format_is_human_by_default(self, cfg):
        # Even if cfg.format == "json", B flips to human on boot.
        s = SessionState.from_boot(cfg)
        assert s.output_format == "human"

    def test_no_color_respected_when_true(self, cfg):
        s = SessionState.from_boot(cfg)
        assert s.no_color is True

    def test_session_id_unique(self, cfg):
        a = SessionState.from_boot(cfg)
        b = SessionState.from_boot(cfg)
        assert a.session_id != b.session_id

    def test_wall_clock_start_time_is_populated(self, cfg):
        """Regression: ``_cmd_save`` relied on a bogus ``monotonic=True``
        path that turned ``started_at`` into "now". A dedicated
        ``started_at_wall`` field now carries the real wall-clock."""
        import time
        t0 = time.time()
        s = SessionState.from_boot(cfg)
        t1 = time.time()
        assert t0 <= s.started_at_wall <= t1
        # Monotonic clock is still in place for duration math.
        assert s.started_at > 0.0


class TestDerivedProperties:
    def test_catalog_name_sentinel(self, state):
        assert state.catalog_name == "(no catalog)"

    def test_catalog_name_uses_attribute(self, state):
        state.catalog = SimpleNamespace(name="my_cat")
        assert state.catalog_name == "my_cat"

    def test_effective_thresholds_none_when_no_catalog(self, state):
        assert state.effective_high_threshold is None
        assert state.effective_medium_threshold is None

    def test_effective_thresholds_from_catalog(self, state, fake_catalog):
        state.catalog = fake_catalog
        assert state.effective_high_threshold == 0.80
        assert state.effective_medium_threshold == 0.50

    def test_override_wins_over_catalog(self, state, fake_catalog):
        state.catalog = fake_catalog
        state.threshold_high_override = 0.95
        state.threshold_medium_override = 0.70
        assert state.effective_high_threshold == 0.95
        assert state.effective_medium_threshold == 0.70


class TestMutations:
    def test_record_query_counts(self, state):
        r = SimpleNamespace(label="alpha", confidence=0.7, rounds=3)
        state.record_query("hi", r, 42.0)
        assert state.queries_count == 1
        assert state.label_counts == {"alpha": 1}
        assert state.latencies_ms == [42.0]
        assert state.history == ["hi"]

    def test_record_query_label_distribution(self, state):
        for lbl in ("alpha", "alpha", "beta"):
            r = SimpleNamespace(label=lbl, confidence=0.5, rounds=2)
            state.record_query("x", r, 1.0)
        assert state.label_counts == {"alpha": 2, "beta": 1}

    def test_record_command(self, state):
        state.record_command("reload", [], outcome="ok", details="done")
        assert state.commands_count == 1
        assert state.transcript[0]["kind"] == "command"
        assert state.history[-1] == ":reload"

    def test_record_command_with_args(self, state):
        state.record_command("budget", ["3000"], outcome="ok")
        assert state.history[-1] == ":budget 3000"

    def test_record_error_increments(self, state):
        state.record_error("NET", "server unreachable")
        assert state.error_count == 1
        assert state.transcript[0]["kind"] == "error"

    def test_reset_threshold_overrides(self, state):
        state.threshold_high_override = 0.9
        state.threshold_medium_override = 0.6
        state.reset_threshold_overrides()
        assert state.threshold_high_override is None
        assert state.threshold_medium_override is None


class TestCaps:
    def test_history_fifo_trim(self, state):
        state.history_cap = 5
        for i in range(10):
            r = SimpleNamespace(label="alpha", confidence=0.1, rounds=1)
            state.record_query(f"q{i}", r, 1.0)
        assert len(state.history) == 5
        assert state.history[0] == "q5"
        assert state.history[-1] == "q9"

    def test_last_results_cap(self, state):
        state.last_results_cap = 3
        for i in range(10):
            r = SimpleNamespace(label="x", confidence=0.1, rounds=1)
            state.record_query(f"q{i}", r, 1.0)
        assert len(state.last_results) == 3

    def test_transcript_cap(self, state):
        state.transcript_cap = 4
        for i in range(10):
            state.record_command("noop", [], outcome="ok")
        assert len(state.transcript) == 4
