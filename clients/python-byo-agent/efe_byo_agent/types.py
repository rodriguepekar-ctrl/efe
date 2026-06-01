"""Shared Pydantic types for efe-byo-agent (M20).

Provider-agnostic LLM response + per-helper output schemas. Used by :
  - Provider implementations (`llm/*.py`) to standardise return value.
  - Helper modules (`helpers/*.py`) to validate sampling/direct outputs.
  - Agent class (`agent.py`) to track cost / tokens cumulatively.
"""
from __future__ import annotations

from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field


# ─── Provider abstraction ───────────────────────────────────────────


class CostModel(BaseModel):
    """Pricing parameters used by `Agent.cost_tracker`."""

    model_config = ConfigDict(frozen=True)

    input_per_1m_tokens_usd: float = Field(ge=0.0)
    output_per_1m_tokens_usd: float = Field(ge=0.0)
    currency: Literal["USD"] = "USD"


class LLMResponse(BaseModel):
    """Provider-agnostic LLM response."""

    model_config = ConfigDict(arbitrary_types_allowed=True)

    text: str
    tokens_in: int = Field(ge=0)
    tokens_out: int = Field(ge=0)
    cost_usd: float = Field(ge=0.0)
    finish_reason: Literal["stop", "length", "tool_use", "safety", "error"]
    tool_calls: list[dict] | None = None
    cache_hit: bool = False
    provider_name: str
    model_name: str
    raw_response: Any | None = Field(default=None, exclude=True, repr=False)


class ProviderHealth(BaseModel):
    """Result of `LLMProvider.health()`."""

    available: bool
    latency_p50_ms: float | None = None
    latency_p99_ms: float | None = None
    error_rate_30d: float | None = None
    last_check_ts: float


# ─── M20 helper input/output models ────────────────────────────────


class ClassificationResult(BaseModel):
    """Input fragment used by `summarize_classification`."""

    label: str = Field(min_length=1, max_length=100)
    confidence: float = Field(ge=0.0, le=1.0)
    anchors_used: list[str] = Field(default_factory=list, max_length=10)


class SummarizeClassificationOutput(BaseModel):
    """Output of `efe_summarize_classification` helper."""

    explanation: str = Field(min_length=20, max_length=1000)
    key_factors: list[str] = Field(min_length=1, max_length=5)
    confidence_assessment: Literal["high", "medium", "low"]
    suggested_alternatives: list[str] = Field(
        default_factory=list, max_length=3,
    )


class CompareBundlesOutput(BaseModel):
    """Output of `efe_compare_bundles_quality` helper."""

    accuracy_delta_pp: float = Field(ge=-100.0, le=100.0)
    divergence_examples: list[dict] = Field(
        default_factory=list, max_length=20,
    )
    summary: str = Field(min_length=20, max_length=500)
    recommendation: Literal["promote", "investigate", "do_not_promote"]


class GenerateTestSentencesOutput(BaseModel):
    """Output of `efe_generate_test_sentences` helper."""

    sentences: list[str] = Field(min_length=1, max_length=64)
    difficulty_self_assessment: dict[str, Literal["easy", "medium", "hard", "edge"]]
    rationale: str = Field(min_length=20, max_length=500)


class ConfusionAnalysisOutput(BaseModel):
    """Output of `efe_analyze_confusion_matrix` helper."""

    top_confusion_pairs: list[dict] = Field(
        default_factory=list, max_length=10,
    )
    proposed_actions: list[dict] = Field(
        default_factory=list, max_length=10,
    )
    summary: str = Field(min_length=20, max_length=1000)


class DraftRunbookOutput(BaseModel):
    """Output of `efe_draft_runbook` helper."""

    runbook_markdown: str = Field(min_length=200, max_length=20000)
    estimated_resolution_time_min: int = Field(ge=1, le=240)
    primary_diagnostics: list[str] = Field(min_length=1, max_length=10)
    risk_level: Literal["low", "medium", "high"]


# ─── Cost tracking ───────────────────────────────────────────────────


class CostTracker(BaseModel):
    """Cumulative cost / token counters for an Agent instance.

    Updated on every LLMResponse received. Useful for CI batch scripts
    that need to enforce per-job budgets client-side.
    """

    model_config = ConfigDict(frozen=False)

    total_calls: int = 0
    total_tokens_in: int = 0
    total_tokens_out: int = 0
    total_cost_usd: float = 0.0
    by_provider: dict[str, dict[str, float]] = Field(default_factory=dict)

    def record(self, response: LLMResponse) -> None:
        self.total_calls += 1
        self.total_tokens_in += response.tokens_in
        self.total_tokens_out += response.tokens_out
        self.total_cost_usd += response.cost_usd
        prov = self.by_provider.setdefault(
            response.provider_name,
            {"calls": 0, "tokens_in": 0, "tokens_out": 0, "cost_usd": 0.0},
        )
        prov["calls"] += 1
        prov["tokens_in"] += response.tokens_in
        prov["tokens_out"] += response.tokens_out
        prov["cost_usd"] += response.cost_usd


__all__ = [
    "CostModel",
    "LLMResponse",
    "ProviderHealth",
    "ClassificationResult",
    "SummarizeClassificationOutput",
    "CompareBundlesOutput",
    "GenerateTestSentencesOutput",
    "ConfusionAnalysisOutput",
    "DraftRunbookOutput",
    "CostTracker",
]
