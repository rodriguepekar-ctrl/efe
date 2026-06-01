"""Discovery tools pin tests (M17 D43 + D45 sampling)."""
from __future__ import annotations

import pytest


@pytest.fixture
def seeded_backend(fake_backend):
    fake_backend._seed_class("d", [
        {"name": "first", "examples": ["alpha", "ace", "ant"]},
        {"name": "second", "examples": ["beta", "bee", "bear"]},
    ])
    return fake_backend


@pytest.mark.asyncio
async def test_find_similar_examples_top_k_5_default(advanced_dispatcher, seeded_backend):
    res = await advanced_dispatcher.dispatch(
        tool_name="efe_find_similar_examples",
        args={"catalog": "d", "text": "alpha"},
    )
    assert res.is_error is False
    assert len(res.content["results"]) <= 5


@pytest.mark.asyncio
async def test_find_similar_examples_max_top_k_32(advanced_dispatcher, seeded_backend):
    res = await advanced_dispatcher.dispatch(
        tool_name="efe_find_similar_examples",
        args={"catalog": "d", "text": "alpha", "top_k": 50},
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_suggest_classes_requires_confirm(agent_native_dispatcher):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_suggest_classes",
        args={
            "examples": [{"text": f"text-{i}"} for i in range(20)],
            "target_class_count": 3,
        },
    )
    # Schema requires confirm
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_suggest_classes_min_examples_10(agent_native_dispatcher):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_suggest_classes",
        args={"examples": [{"text": "x"}], "confirm": True},
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_suggest_classes_returns_proposed(agent_native_dispatcher, fake_sampling_session):
    fake_sampling_session.next_response = "billing-class"
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_suggest_classes",
        args={
            "examples": [{"text": f"sample text {i}"} for i in range(20)],
            "target_class_count": 2,
            "confirm": True,
        },
    )
    assert res.is_error is False, res.content
    assert "proposed_classes" in res.content


@pytest.mark.asyncio
async def test_suggest_classes_naming_method_recorded(agent_native_dispatcher, fake_sampling_session):
    fake_sampling_session.next_response = "auto-named"
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_suggest_classes",
        args={
            "examples": [{"text": f"x-{i}"} for i in range(15)],
            "target_class_count": 2,
            "confirm": True,
        },
    )
    assert res.content["naming_method"] in ("sampling", "heuristic_top_terms")


@pytest.mark.asyncio
async def test_propose_examples_requires_confirm(agent_native_dispatcher, seeded_backend):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_propose_examples",
        args={"catalog": "d", "class_name": "first"},
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_propose_examples_returns_candidates(agent_native_dispatcher, seeded_backend, fake_sampling_session):
    # Make the LLM return three candidate lines
    fake_sampling_session.next_response = (
        "alpha plus extension\n"
        "alpha new variation\n"
        "alpha alternate phrasing\n"
    )
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_propose_examples",
        args={
            "catalog": "d", "class_name": "first",
            "n_proposed": 3, "confirm": True,
        },
    )
    assert res.is_error is False, res.content
    assert "proposed_examples" in res.content


@pytest.mark.asyncio
async def test_propose_examples_n_proposed_max_10(agent_native_dispatcher, seeded_backend):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_propose_examples",
        args={
            "catalog": "d", "class_name": "first",
            "n_proposed": 50, "confirm": True,
        },
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"
