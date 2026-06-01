"""Token pricing table and cost estimation for Letter-C scripts.

Anthropic does not expose pricing programmatically, so we maintain a
hand-curated table here. Update on every price change and bump the
``PRICING_TABLE_VERSION`` constant so consumers can see (via a log
event) when their estimated numbers drifted.

Pricing is **per million tokens** in USD. Input and output rates are
tracked separately (output is usually 5× input).

See ``LETTER_C_ARCHITECTURE.md`` §12 for the design rationale.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Dict


PRICING_TABLE_VERSION: str = "2026-04-19"


@dataclass(frozen=True)
class ModelPricing:
    """Per-model input/output pricing in USD per million tokens."""

    model_prefix: str
    input_per_million: float
    output_per_million: float
    tier: str = "general"

    def cost_usd(self, input_tokens: int, output_tokens: int) -> float:
        """Compute the USD cost of a call with the given token counts."""
        cost_in = (max(0, input_tokens) / 1_000_000.0) * self.input_per_million
        cost_out = (max(0, output_tokens) / 1_000_000.0) * self.output_per_million
        return round(cost_in + cost_out, 6)


# ---------------------------------------------------------------------------
#  Pricing table (USD per million tokens)
# ---------------------------------------------------------------------------
#
# Order matters: the lookup walks entries top-down and returns the first
# whose ``model_prefix`` is a prefix of the user-supplied model id.

PRICING: tuple[ModelPricing, ...] = (
    # Claude 4 family
    ModelPricing(
        model_prefix="claude-opus-4",
        input_per_million=15.00,
        output_per_million=75.00,
        tier="frontier",
    ),
    ModelPricing(
        model_prefix="claude-sonnet-4",
        input_per_million=3.00,
        output_per_million=15.00,
        tier="general",
    ),
    # Claude 3.5 family (still common in late 2026)
    ModelPricing(
        model_prefix="claude-haiku-3-5",
        input_per_million=0.80,
        output_per_million=4.00,
        tier="fast",
    ),
    ModelPricing(
        model_prefix="claude-sonnet-3-5",
        input_per_million=3.00,
        output_per_million=15.00,
        tier="general",
    ),
    ModelPricing(
        model_prefix="claude-opus-3",
        input_per_million=15.00,
        output_per_million=75.00,
        tier="frontier",
    ),
)


# Fallback used when no prefix matches. Picks middle-of-the-road sonnet
# pricing so cost estimates are at least order-of-magnitude correct.
FALLBACK_PRICING: ModelPricing = ModelPricing(
    model_prefix="",
    input_per_million=3.00,
    output_per_million=15.00,
    tier="unknown",
)


# ---------------------------------------------------------------------------
#  Public API
# ---------------------------------------------------------------------------


def lookup_pricing(model: str) -> ModelPricing:
    """Return the :class:`ModelPricing` for ``model``, or the fallback."""
    if not model:
        return FALLBACK_PRICING
    for entry in PRICING:
        if model.startswith(entry.model_prefix):
            return entry
    return FALLBACK_PRICING


def estimate_cost_usd(
    model: str, *, input_tokens: int, output_tokens: int,
) -> float:
    """Return the USD cost estimate for a call with the given token counts.

    Silently rounds to 6 decimals. Returns 0.0 when both counts are zero
    (avoids polluting stats with negligible noise).
    """
    if input_tokens == 0 and output_tokens == 0:
        return 0.0
    return lookup_pricing(model).cost_usd(input_tokens, output_tokens)


def format_cost_usd(cost: float) -> str:
    """Render a cost for human-facing display: ``$0.0123`` or ``<$0.0001``."""
    if cost <= 0:
        return "$0.0000"
    if cost < 1e-4:
        return "<$0.0001"
    # 4 decimals for anything under $1, 2 decimals otherwise.
    if cost < 1.0:
        return f"${cost:.4f}"
    return f"${cost:.2f}"


__all__ = [
    "PRICING_TABLE_VERSION",
    "PRICING",
    "FALLBACK_PRICING",
    "ModelPricing",
    "lookup_pricing",
    "estimate_cost_usd",
    "format_cost_usd",
]
