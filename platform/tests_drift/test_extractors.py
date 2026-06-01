"""Feature extractors — per-type correctness."""

from __future__ import annotations

from datetime import datetime, timezone

import pytest

from efe_drift.extraction import (
    CountryExtractor,
    DeterministicHashEmbeddingExtractor,
    ExtractorRegistry,
    HasCodeExtractor,
    LanguageDetectExtractor,
    LatencyExtractor,
    ModelVariantExtractor,
    PromptTokenLengthExtractor,
    ResponseTokenLengthExtractor,
    TemperatureExtractor,
    TopPExtractor,
    build_default_registry,
    simple_heuristic_language,
)
from efe_drift.extraction.numeric import CostExtractor
from efe_drift.records import InferenceEvent


def _event(**overrides) -> InferenceEvent:
    base = dict(
        event_id="e",
        tenant_id="acme",
        task_id="summarize",
        user_id="u",
        observed_at=datetime(2026, 4, 20, tzinfo=timezone.utc),
    )
    base.update(overrides)
    return InferenceEvent(**base)


# ---------------------------------------------------------------------------
# Numeric extractors
# ---------------------------------------------------------------------------


def test_prompt_token_length_prefers_reported_count():
    ext = PromptTokenLengthExtractor()
    fv = ext.extract(_event(prompt_token_count=42))
    assert fv is not None and fv.unwrap_numeric() == 42.0


def test_prompt_token_length_falls_back_to_split():
    ext = PromptTokenLengthExtractor()
    fv = ext.extract(_event(prompt_text="one two three four"))
    assert fv is not None and fv.unwrap_numeric() == 4.0


def test_prompt_token_length_returns_none_without_signal():
    ext = PromptTokenLengthExtractor()
    assert ext.extract(_event()) is None


def test_response_token_length():
    ext = ResponseTokenLengthExtractor()
    assert ext.extract(_event(response_token_count=7)).unwrap_numeric() == 7.0


def test_latency_extractor():
    fv = LatencyExtractor().extract(_event(latency_ms=128.5))
    assert fv.unwrap_numeric() == 128.5


def test_cost_extractor_uses_reported_cost():
    ext = CostExtractor(pricing_per_model={"llama-7b": 0.5})
    fv = ext.extract(_event(cost_usd=0.01))
    assert fv.unwrap_numeric() == 0.01


def test_cost_extractor_applies_pricing_table():
    ext = CostExtractor(pricing_per_model={"llama-7b": 0.5})
    fv = ext.extract(_event(
        model_id="llama-7b",
        prompt_token_count=2000,
        response_token_count=0,
    ))
    # 0.5 USD / 1k tokens × 2k tokens = 1.0 USD
    assert fv.unwrap_numeric() == pytest.approx(1.0)


def test_cost_extractor_missing_model_returns_none():
    ext = CostExtractor(pricing_per_model={"llama-7b": 0.5})
    assert ext.extract(_event(
        model_id="unknown", prompt_token_count=1000,
    )) is None


def test_temperature_extractor():
    fv = TemperatureExtractor().extract(_event(temperature=0.9))
    assert fv.unwrap_numeric() == pytest.approx(0.9)


def test_top_p_extractor():
    fv = TopPExtractor().extract(_event(top_p=0.5))
    assert fv.unwrap_numeric() == pytest.approx(0.5)


# ---------------------------------------------------------------------------
# Categorical / binary
# ---------------------------------------------------------------------------


def test_simple_heuristic_language_french():
    # Requires at least one accented Latin character for the heuristic
    # to return "fr" — unaccented prose falls through to "en".
    assert simple_heuristic_language("Bonjour, ça va très bien") == "fr"


def test_simple_heuristic_language_english():
    assert simple_heuristic_language("Hello there friend") == "en"


def test_simple_heuristic_language_cjk():
    assert simple_heuristic_language("这是一个测试字符串") == "zh"


def test_simple_heuristic_language_short_returns_none():
    assert simple_heuristic_language("abc") is None


def test_language_detect_extractor_returns_none_without_prompt():
    ext = LanguageDetectExtractor()
    assert ext.extract(_event()) is None


def test_language_detect_extractor_happy_path():
    ext = LanguageDetectExtractor()
    fv = ext.extract(_event(prompt_text="Hello how are you today"))
    assert fv is not None
    assert fv.unwrap_categorical() == "en"


def test_has_code_extractor_detects_fence():
    ext = HasCodeExtractor()
    fv = ext.extract(_event(prompt_text="```python\nprint('x')\n```"))
    assert fv.unwrap_numeric() == 1.0


def test_has_code_extractor_detects_sql_keyword():
    ext = HasCodeExtractor()
    fv = ext.extract(_event(prompt_text="SELECT * FROM users LIMIT 10"))
    assert fv.unwrap_numeric() == 1.0


def test_has_code_extractor_negative():
    ext = HasCodeExtractor()
    fv = ext.extract(_event(prompt_text="Tell me a poem about autumn"))
    assert fv.unwrap_numeric() == 0.0


def test_country_extractor_normalises():
    ext = CountryExtractor()
    fv = ext.extract(_event(covariates={"country": "fr"}))
    assert fv.unwrap_categorical() == "FR"


def test_country_extractor_rejects_non_iso():
    ext = CountryExtractor()
    assert ext.extract(_event(covariates={"country": "France"})) is None


def test_model_variant_extractor():
    ext = ModelVariantExtractor()
    fv = ext.extract(_event(model_id="llama-7b"))
    assert fv.unwrap_categorical() == "llama-7b"


# ---------------------------------------------------------------------------
# Embedding
# ---------------------------------------------------------------------------


def test_deterministic_embedding_is_reproducible():
    ext = DeterministicHashEmbeddingExtractor(dim=32)
    e = _event(prompt_text="Hello there")
    fv1 = ext.extract(e)
    fv2 = ext.extract(e)
    assert fv1 is not None and fv2 is not None
    assert fv1.unwrap_embedding() == fv2.unwrap_embedding()


def test_deterministic_embedding_l2_normalised():
    ext = DeterministicHashEmbeddingExtractor(dim=16)
    fv = ext.extract(_event(prompt_text="hello world"))
    vec = fv.unwrap_embedding()
    norm = sum(c * c for c in vec) ** 0.5
    assert abs(norm - 1.0) < 1e-9


def test_deterministic_embedding_different_inputs_diverge():
    ext = DeterministicHashEmbeddingExtractor(dim=32)
    a = ext.extract(_event(prompt_text="hello"))
    b = ext.extract(_event(prompt_text="world"))
    assert a.unwrap_embedding() != b.unwrap_embedding()


# ---------------------------------------------------------------------------
# Registry
# ---------------------------------------------------------------------------


def test_registry_register_and_get():
    reg = ExtractorRegistry()
    ext = PromptTokenLengthExtractor()
    reg.register(ext)
    assert reg.has(ext.name)
    assert reg.get(ext.name) is ext


def test_registry_rejects_duplicate():
    reg = ExtractorRegistry()
    reg.register(PromptTokenLengthExtractor())
    with pytest.raises(ValueError):
        reg.register(PromptTokenLengthExtractor())


def test_build_default_registry_has_all_stock_extractors():
    reg = build_default_registry()
    names = set(reg.names())
    assert {
        "token_length",
        "response_token_length",
        "backend_latency",
        "temperature",
        "top_p",
        "lang_detect",
        "has_code_heuristic",
        "country",
        "model_variant",
        "deterministic_hash_embedding",
    } <= names
