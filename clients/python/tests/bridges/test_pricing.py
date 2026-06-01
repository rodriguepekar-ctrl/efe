"""Tests for :mod:`efe_tool.bridges.pricing`."""

from __future__ import annotations

import pytest

from efe_tool.bridges.pricing import (
    FALLBACK_PRICING,
    PRICING,
    PRICING_TABLE_VERSION,
    estimate_cost_usd,
    format_cost_usd,
    lookup_pricing,
)


class TestLookup:
    def test_sonnet_4_6(self):
        p = lookup_pricing("claude-sonnet-4-6")
        assert p.input_per_million == 3.00
        assert p.output_per_million == 15.00

    def test_sonnet_4_prefix(self):
        # Any sonnet-4-* should map to the same row.
        p = lookup_pricing("claude-sonnet-4-7-beta")
        assert p.input_per_million == 3.00

    def test_haiku_3_5(self):
        p = lookup_pricing("claude-haiku-3-5")
        assert p.input_per_million == 0.80
        assert p.output_per_million == 4.00

    def test_opus_4(self):
        p = lookup_pricing("claude-opus-4")
        assert p.input_per_million == 15.00
        assert p.output_per_million == 75.00

    def test_unknown_returns_fallback(self):
        p = lookup_pricing("llama-70b")
        assert p is FALLBACK_PRICING

    def test_empty_returns_fallback(self):
        p = lookup_pricing("")
        assert p is FALLBACK_PRICING


class TestEstimateCost:
    def test_zero_tokens_returns_zero(self):
        assert estimate_cost_usd(
            "claude-sonnet-4-6", input_tokens=0, output_tokens=0,
        ) == 0.0

    def test_sonnet_1_million_input(self):
        # 1_000_000 input tokens at $3/M = $3.00 exactly.
        assert estimate_cost_usd(
            "claude-sonnet-4-6", input_tokens=1_000_000, output_tokens=0,
        ) == 3.00

    def test_sonnet_1_million_output(self):
        assert estimate_cost_usd(
            "claude-sonnet-4-6", input_tokens=0, output_tokens=1_000_000,
        ) == 15.00

    def test_mixed_tokens(self):
        # 100k in + 30k out on sonnet = 0.1*3 + 0.03*15 = 0.3 + 0.45 = 0.75
        assert estimate_cost_usd(
            "claude-sonnet-4-6", input_tokens=100_000, output_tokens=30_000,
        ) == pytest.approx(0.75, rel=1e-6)

    def test_haiku_is_cheaper_than_sonnet(self):
        c_s = estimate_cost_usd(
            "claude-sonnet-4-6", input_tokens=1000, output_tokens=200,
        )
        c_h = estimate_cost_usd(
            "claude-haiku-3-5", input_tokens=1000, output_tokens=200,
        )
        assert c_h < c_s


class TestFormat:
    def test_zero(self):
        assert format_cost_usd(0.0) == "$0.0000"

    def test_very_small(self):
        assert format_cost_usd(1e-5) == "<$0.0001"

    def test_under_one_dollar(self):
        assert format_cost_usd(0.0123) == "$0.0123"

    def test_above_one_dollar(self):
        assert format_cost_usd(1.2345) == "$1.23"


class TestStructuralInvariants:
    def test_pricing_table_version_is_iso_ish_date(self):
        import re
        assert re.match(r"^\d{4}-\d{2}-\d{2}$", PRICING_TABLE_VERSION)

    def test_each_row_has_positive_pricing(self):
        for row in PRICING:
            assert row.input_per_million > 0
            assert row.output_per_million > 0
            # Output is almost always more expensive than input.
            assert row.output_per_million >= row.input_per_million
