"""Tests for :mod:`efe_tool.repl.prompt`."""

from __future__ import annotations

from efe_tool.repl.prompt import (
    DEFAULT_TEMPLATE,
    build_prompt,
    farewell_line,
    welcome_banner,
)


class TestDefaultTemplate:
    def test_basic_no_catalog(self, state):
        out = build_prompt(state, force_color=False, for_readline=False)
        assert "EFE " in out
        assert "no catalog" in out
        assert out.endswith("> ")


class TestCustomTemplate:
    def test_queries_placeholder(self, state):
        state.queries_count = 17
        out = build_prompt(
            state, template="[{queries}]> ",
            force_color=False, for_readline=False,
        )
        assert out == "[17]> "

    def test_endpoint_placeholder(self, state):
        state.endpoint = "1.2.3.4:50051"
        out = build_prompt(
            state, template="{endpoint}> ",
            force_color=False, for_readline=False,
        )
        assert out == "1.2.3.4:50051> "

    def test_unknown_placeholder_falls_back(self, state):
        out = build_prompt(
            state, template="{bogus}> ",
            force_color=False, for_readline=False,
        )
        # Falls back to the default template (no crash)
        assert "EFE " in out


class TestColorMarkers:
    def test_for_readline_wraps_with_01_02(self, state, fake_catalog):
        state.catalog = fake_catalog
        out = build_prompt(state, force_color=True, for_readline=True)
        assert "\x01" in out
        assert "\x02" in out

    def test_no_readline_wrap_when_false(self, state, fake_catalog):
        state.catalog = fake_catalog
        out = build_prompt(state, force_color=True, for_readline=False)
        assert "\x01" not in out
        assert "\x02" not in out

    def test_readline_bracketing_leaves_visible_text_outside_markers(
        self, state, fake_catalog,
    ):
        """Regression: ``_paint_readline_safe`` must place the visible
        text *between* the ``\\x01...\\x02`` markers, not inside them,
        so readline can compute the correct cursor width."""
        state.catalog = fake_catalog
        out = build_prompt(state, force_color=True, for_readline=True)
        # Locate the visible catalog name. It must appear OUTSIDE any
        # \x01..\x02 invisible block.
        name = state.catalog_name
        idx = out.index(name)
        # Count the parity of \x01/\x02 markers before the name. If the
        # name is inside an invisible block, this parity would be > 0.
        before = out[:idx]
        opens = before.count("\x01")
        closes = before.count("\x02")
        assert opens == closes, (
            f"visible text {name!r} inside invisible block "
            f"(opens={opens}, closes={closes}, out={out!r})"
        )

    def test_readline_bracketing_includes_reset(self, state, fake_catalog):
        """The RESET (\\x1b[0m) must actually be emitted after the
        colored chunk, otherwise colors leak into what the user types."""
        state.catalog = fake_catalog
        out = build_prompt(state, force_color=True, for_readline=True)
        assert "\x1b[0m" in out


class TestFarewell:
    def test_farewell_contains_counts(self, state):
        state.queries_count = 3
        state.commands_count = 5
        out = farewell_line(state)
        assert "3 queries" in out
        assert "5 commands" in out


class TestBanner:
    def test_banner_has_three_lines_with_no_catalog(self, state):
        lines = welcome_banner(state, version="0.1.0", force_color=False)
        assert len(lines) >= 3
        assert any("interactive REPL" in ln for ln in lines)

    def test_banner_with_catalog(self, state, fake_catalog):
        state.catalog = fake_catalog
        lines = welcome_banner(state, version="0.1.0", force_color=False)
        assert any("fake" in ln for ln in lines)
