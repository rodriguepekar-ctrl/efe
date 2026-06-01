"""Tests for :mod:`efe_tool.repl.render`."""

from __future__ import annotations

from types import SimpleNamespace

from efe_tool.repl.render import (
    color_for_label,
    render_classes,
    render_error_human,
    render_info,
    render_query_result_delegate,
    render_query_result_human,
    render_stats,
)


class TestColorForLabel:
    def test_no_color_returns_empty(self, fake_catalog):
        assert color_for_label("alpha", fake_catalog, use_color=False) == ""

    def test_respects_catalog_color_when_color(self, fake_catalog):
        s = color_for_label("alpha", fake_catalog, use_color=True)
        assert s != ""

    def test_missing_catalog_returns_empty(self):
        assert color_for_label("x", None, use_color=True) == ""


class TestRenderQueryHuman:
    def test_contains_label_confidence_rounds(self, fake_catalog):
        r = SimpleNamespace(
            label="alpha", confidence=0.675, rounds=4,
            reject=False, alignment_delta=0.1,
        )
        out = render_query_result_human(
            r, catalog=fake_catalog, elapsed_ms=123.4,
            diagnostics=False, use_color=False,
        )
        assert "alpha" in out
        assert "0.675" in out
        assert "rounds 4" in out

    def test_reject_is_reddish_verdict(self, fake_catalog):
        r = SimpleNamespace(
            label="alpha", confidence=0.10, rounds=4,
            reject=True, alignment_delta=0.1,
        )
        out = render_query_result_human(
            r, catalog=fake_catalog, elapsed_ms=10,
            diagnostics=False, use_color=False,
        )
        assert "reject" in out

    def test_diagnostics_shows_sub_scores(self, fake_catalog):
        r = SimpleNamespace(
            label="alpha", confidence=0.7, rounds=3,
            reject=False, alignment_delta=0.1,
            sub_scores=[
                SimpleNamespace(label="alpha", cosine=0.8),
                SimpleNamespace(label="beta", cosine=0.2),
            ],
        )
        out = render_query_result_human(
            r, catalog=fake_catalog, elapsed_ms=10,
            diagnostics=True, use_color=False,
        )
        assert "diagnostics:" in out
        assert "alpha" in out and "beta" in out
        assert "← top" in out


class TestRenderError:
    def test_message_and_hint(self):
        out = render_error_human(
            "CATALOG_NOT_FOUND", "missing file",
            hint="check the path", use_color=False,
        )
        assert "CATALOG_NOT_FOUND" in out
        assert "missing file" in out
        assert "check the path" in out


class TestRenderInfo:
    def test_all_rows_present(self, state):
        out = render_info(state, use_color=False)
        for key in ("session", "catalog", "endpoint", "budget", "timeout"):
            assert key in out


class TestRenderStats:
    def test_empty_stats(self, state):
        out = render_stats(state, use_color=False)
        assert "Queries:" in out

    def test_with_latencies(self, state):
        r = SimpleNamespace(label="alpha", confidence=0.5, rounds=2)
        for ms in (10.0, 20.0, 30.0, 40.0):
            state.record_query("x", r, ms)
        out = render_stats(state, use_color=False)
        assert "Latency p50" in out
        assert "alpha=4" in out


class TestRenderClasses:
    def test_no_catalog(self):
        assert "no catalog" in render_classes(None, use_color=False)

    def test_with_classes(self, fake_catalog):
        out = render_classes(fake_catalog, use_color=False)
        assert "alpha" in out and "beta" in out


class TestDelegateFormats:
    def test_json_format(self, fake_catalog):
        r = SimpleNamespace(
            label="alpha", confidence=0.7, rounds=4,
            reject=False, alignment_delta=0.1,
            total_ms=42, text_hash="h", swarm_id="s",
            top_cosine=0.9, sub_scores=None,
        )
        out = render_query_result_delegate(
            r, catalog=fake_catalog, elapsed_ms=50.0,
            request_id="rid", diagnostics=False,
            output_format="json", pretty=False,
        )
        import json
        doc = json.loads(out)
        assert doc["label"] == "alpha"
        assert doc["schema_version"] >= 1
