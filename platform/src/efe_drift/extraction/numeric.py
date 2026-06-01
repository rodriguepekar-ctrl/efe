"""Numeric extractors — pull a single scalar out of an event."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass

from efe_drift.extraction.base import (
    numeric,
)
from efe_drift.records import FeatureKind, FeatureValue, InferenceEvent


@dataclass(frozen=True, slots=True)
class PromptTokenLengthExtractor:
    """``prompt_length_tokens`` — reads ``event.prompt_token_count``.

    Falls back to a whitespace-based token count when the backend
    didn't report a token count (rare — M07 always populates it, but
    we keep a sane default so batch ingestion from HTTP still works).
    """

    name: str = "token_length"
    kind: FeatureKind = "numeric"
    feature_name: str = "prompt_length_tokens"

    def extract(self, event: InferenceEvent) -> FeatureValue | None:
        if event.prompt_token_count is not None:
            return numeric(self.feature_name, event.prompt_token_count)
        if event.prompt_text:
            approx = len(event.prompt_text.split())
            return numeric(self.feature_name, approx)
        return None


@dataclass(frozen=True, slots=True)
class ResponseTokenLengthExtractor:
    name: str = "response_token_length"
    kind: FeatureKind = "numeric"
    feature_name: str = "response_length_tokens"

    def extract(self, event: InferenceEvent) -> FeatureValue | None:
        if event.response_token_count is not None:
            return numeric(self.feature_name, event.response_token_count)
        if event.response_text:
            approx = len(event.response_text.split())
            return numeric(self.feature_name, approx)
        return None


@dataclass(frozen=True, slots=True)
class LatencyExtractor:
    name: str = "backend_latency"
    kind: FeatureKind = "numeric"
    feature_name: str = "latency_ms"

    def extract(self, event: InferenceEvent) -> FeatureValue | None:
        return numeric(self.feature_name, event.latency_ms)


@dataclass(frozen=True, slots=True)
class CostExtractor:
    """Applies a pricing table when the backend didn't report cost."""

    pricing_per_model: Mapping[str, float]          # USD / 1k tokens
    name: str = "cost_pricing"
    kind: FeatureKind = "numeric"
    feature_name: str = "cost_usd"

    def extract(self, event: InferenceEvent) -> FeatureValue | None:
        if event.cost_usd is not None:
            return numeric(self.feature_name, event.cost_usd)
        if event.model_id is None:
            return None
        rate = self.pricing_per_model.get(event.model_id)
        if rate is None:
            return None
        tokens = (
            (event.prompt_token_count or 0)
            + (event.response_token_count or 0)
        )
        if tokens <= 0:
            return None
        return numeric(self.feature_name, rate * tokens / 1000.0)


@dataclass(frozen=True, slots=True)
class TemperatureExtractor:
    name: str = "temperature"
    kind: FeatureKind = "numeric"
    feature_name: str = "temperature"

    def extract(self, event: InferenceEvent) -> FeatureValue | None:
        return numeric(self.feature_name, event.temperature)


@dataclass(frozen=True, slots=True)
class TopPExtractor:
    name: str = "top_p"
    kind: FeatureKind = "numeric"
    feature_name: str = "top_p"

    def extract(self, event: InferenceEvent) -> FeatureValue | None:
        return numeric(self.feature_name, event.top_p)


__all__ = [
    "CostExtractor",
    "LatencyExtractor",
    "PromptTokenLengthExtractor",
    "ResponseTokenLengthExtractor",
    "TemperatureExtractor",
    "TopPExtractor",
]
