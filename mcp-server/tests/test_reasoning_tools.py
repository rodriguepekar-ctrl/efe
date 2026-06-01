"""Reasoning tools pin tests (M17 D43)."""
from __future__ import annotations

import pytest


@pytest.fixture
def seeded_backend(fake_backend):
    fake_backend._seed_class("greetings", [
        {"name": "hi", "examples": ["hello", "hi", "hey"], "confidence_threshold": 0.5},
        {"name": "bye", "examples": ["goodbye", "see ya"], "confidence_threshold": 0.5},
    ])
    return fake_backend


@pytest.mark.asyncio
async def test_explain_classification_returns_top_contributors(advanced_dispatcher, seeded_backend):
    res = await advanced_dispatcher.dispatch(
        tool_name="efe_explain_classification",
        args={"catalog": "greetings", "text": "hello there"},
    )
    assert res.is_error is False, res.content
    assert "predicted_class" in res.content
    assert "runner_up" in res.content


@pytest.mark.asyncio
async def test_explain_classification_predicts_when_not_provided(advanced_dispatcher, seeded_backend):
    res = await advanced_dispatcher.dispatch(
        tool_name="efe_explain_classification",
        args={"catalog": "greetings", "text": "hi"},
    )
    assert res.is_error is False
    assert res.content["predicted_class"] in ("hi", "bye")


@pytest.mark.asyncio
async def test_explain_classification_threshold_sensitivity_range(advanced_dispatcher, seeded_backend):
    res = await advanced_dispatcher.dispatch(
        tool_name="efe_explain_classification",
        args={"catalog": "greetings", "text": "hi"},
    )
    assert "threshold_sensitivity" in res.content
    sens = res.content["threshold_sensitivity"]
    assert "decision_unchanged_in_range" in sens


@pytest.mark.asyncio
async def test_explain_classification_unknown_catalog_returns_not_found(advanced_dispatcher):
    res = await advanced_dispatcher.dispatch(
        tool_name="efe_explain_classification",
        args={"catalog": "missing", "text": "hi"},
    )
    assert res.is_error is True
    assert res.content["code"] == "not_found"


@pytest.mark.asyncio
async def test_compare_classes_returns_separation_score(advanced_dispatcher, seeded_backend):
    res = await advanced_dispatcher.dispatch(
        tool_name="efe_compare_classes",
        args={"catalog": "greetings", "class_a": "hi", "class_b": "bye"},
    )
    assert res.is_error is False
    assert "separation_score" in res.content
    assert 0.0 <= res.content["separation_score"] <= 1.0


@pytest.mark.asyncio
async def test_compare_classes_identifies_boundary_examples(advanced_dispatcher, seeded_backend):
    res = await advanced_dispatcher.dispatch(
        tool_name="efe_compare_classes",
        args={"catalog": "greetings", "class_a": "hi", "class_b": "bye"},
    )
    assert "boundary_examples" in res.content
    assert isinstance(res.content["boundary_examples"], list)


@pytest.mark.asyncio
async def test_compare_classes_invalid_class_name_400(advanced_dispatcher, seeded_backend):
    # Identical classes should return invalid_arguments
    res = await advanced_dispatcher.dispatch(
        tool_name="efe_compare_classes",
        args={"catalog": "greetings", "class_a": "hi", "class_b": "hi"},
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_compare_classes_band_in_known_set(advanced_dispatcher, seeded_backend):
    res = await advanced_dispatcher.dispatch(
        tool_name="efe_compare_classes",
        args={"catalog": "greetings", "class_a": "hi", "class_b": "bye"},
    )
    assert res.content["ambiguity_band"] in ("none", "moderate", "high", "critical")


@pytest.mark.asyncio
async def test_score_distribution_default_sample_count_100(advanced_dispatcher, seeded_backend):
    res = await advanced_dispatcher.dispatch(
        tool_name="efe_score_distribution",
        args={"catalog": "greetings", "class_name": "hi"},
    )
    assert res.is_error is False
    assert "histogram" in res.content


@pytest.mark.asyncio
async def test_score_distribution_with_provided_texts(advanced_dispatcher, seeded_backend):
    res = await advanced_dispatcher.dispatch(
        tool_name="efe_score_distribution",
        args={
            "catalog": "greetings",
            "class_name": "hi",
            "texts": ["hello", "hey", "yo"],
            "sample_count": 10,
        },
    )
    assert res.is_error is False


@pytest.mark.asyncio
async def test_score_distribution_recommends_threshold_in_range(advanced_dispatcher, seeded_backend):
    res = await advanced_dispatcher.dispatch(
        tool_name="efe_score_distribution",
        args={"catalog": "greetings", "class_name": "hi", "sample_count": 20},
    )
    rec = res.content["recommended_threshold"]
    assert 0.0 <= rec["value"] <= 1.0
    assert 0.0 <= rec["expected_precision"] <= 1.0


@pytest.mark.asyncio
async def test_score_distribution_max_sample_count_1000_enforced(advanced_dispatcher, seeded_backend):
    res = await advanced_dispatcher.dispatch(
        tool_name="efe_score_distribution",
        args={"catalog": "greetings", "class_name": "hi", "sample_count": 1500},
    )
    # Schema enforces max=1000, so this should be rejected
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"
