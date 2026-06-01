"""Feature extractor package."""

from __future__ import annotations

from efe_drift.extraction.base import (
    FeatureExtractor,
    assert_extractor_produces,
    binary,
    categorical,
    embedding,
    numeric,
)
from efe_drift.extraction.categorical import (
    CountryExtractor,
    HasCodeExtractor,
    LanguageDetectExtractor,
    ModelVariantExtractor,
    simple_heuristic_language,
)
from efe_drift.extraction.embedding import (
    DeterministicHashEmbeddingExtractor,
    SbertEmbeddingExtractor,
)
from efe_drift.extraction.numeric import (
    CostExtractor,
    LatencyExtractor,
    PromptTokenLengthExtractor,
    ResponseTokenLengthExtractor,
    TemperatureExtractor,
    TopPExtractor,
)
from efe_drift.extraction.registry import ExtractorRegistry


def build_default_registry() -> ExtractorRegistry:
    """Return an :class:`ExtractorRegistry` populated with every
    built-in extractor."""
    reg = ExtractorRegistry()
    reg.register(PromptTokenLengthExtractor())
    reg.register(ResponseTokenLengthExtractor())
    reg.register(LatencyExtractor())
    reg.register(TemperatureExtractor())
    reg.register(TopPExtractor())
    reg.register(LanguageDetectExtractor())
    reg.register(HasCodeExtractor())
    reg.register(CountryExtractor())
    reg.register(ModelVariantExtractor())
    reg.register(DeterministicHashEmbeddingExtractor())
    return reg


__all__ = [
    "CostExtractor",
    "CountryExtractor",
    "DeterministicHashEmbeddingExtractor",
    "ExtractorRegistry",
    "FeatureExtractor",
    "HasCodeExtractor",
    "LanguageDetectExtractor",
    "LatencyExtractor",
    "ModelVariantExtractor",
    "PromptTokenLengthExtractor",
    "ResponseTokenLengthExtractor",
    "SbertEmbeddingExtractor",
    "TemperatureExtractor",
    "TopPExtractor",
    "assert_extractor_produces",
    "binary",
    "build_default_registry",
    "categorical",
    "embedding",
    "numeric",
    "simple_heuristic_language",
]
