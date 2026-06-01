"""Composite tools pin tests (M17 D45)."""
from __future__ import annotations

import pytest


@pytest.fixture
def seeded_backend(fake_backend):
    fake_backend._seed_class("c", [
        {"name": "alpha", "examples": ["alpha-ex"] * 3},
        {"name": "beta", "examples": ["beta-ex"]},
    ])
    return fake_backend


@pytest.mark.asyncio
async def test_curate_catalog_requires_confirm(agent_native_dispatcher, seeded_backend):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_curate_catalog",
        args={"catalog": "c", "intent": "balance_class_sizes"},
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_curate_catalog_with_confirm_returns_plan(agent_native_dispatcher, seeded_backend):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_curate_catalog",
        args={
            "catalog": "c",
            "intent": "augment_underrepresented",
            "confirm": True,
        },
    )
    assert res.is_error is False, res.content
    assert "plan" in res.content
    assert "actions" in res.content["plan"]


@pytest.mark.asyncio
async def test_curate_catalog_intent_remove_ambiguity(agent_native_dispatcher, seeded_backend):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_curate_catalog",
        args={"catalog": "c", "intent": "remove_ambiguity", "confirm": True},
    )
    assert res.is_error is False


@pytest.mark.asyncio
async def test_curate_catalog_invalid_intent(agent_native_dispatcher, seeded_backend):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_curate_catalog",
        args={"catalog": "c", "intent": "made-up", "confirm": True},
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_curate_catalog_max_changes_enforced(agent_native_dispatcher, seeded_backend):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_curate_catalog",
        args={
            "catalog": "c",
            "intent": "augment_underrepresented",
            "max_changes": 200,
            "confirm": True,
        },
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_calibrate_thresholds_internal_holdout_method(agent_native_dispatcher, seeded_backend):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_calibrate_thresholds",
        args={"catalog": "c"},
    )
    assert res.is_error is False
    assert res.content["calibration_method"] == "internal_holdout"


@pytest.mark.asyncio
async def test_calibrate_thresholds_provided_validation_method(agent_native_dispatcher, seeded_backend):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_calibrate_thresholds",
        args={
            "catalog": "c",
            "validation_set": [
                {"text": "alpha-ex", "true_class": "alpha"},
                {"text": "beta-ex", "true_class": "beta"},
            ],
        },
    )
    assert res.is_error is False
    assert res.content["calibration_method"] == "provided_validation"


@pytest.mark.asyncio
async def test_calibrate_thresholds_global_summary(agent_native_dispatcher, seeded_backend):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_calibrate_thresholds",
        args={"catalog": "c"},
    )
    assert "global_summary" in res.content
    assert "avg_expected_precision" in res.content["global_summary"]


@pytest.mark.asyncio
async def test_consolidate_versions_min_keep_last_n_1(agent_native_dispatcher, seeded_backend):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_consolidate_versions",
        args={"catalog": "c", "keep_last_n": 0},
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_consolidate_versions_returns_preserved(agent_native_dispatcher, seeded_backend):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_consolidate_versions",
        args={"catalog": "c", "keep_last_n": 2},
    )
    assert res.is_error is False
    assert "preserved_versions" in res.content
    assert "to_delete_versions" in res.content


@pytest.mark.asyncio
async def test_curate_catalog_streaming_token_accepted(agent_native_dispatcher, seeded_backend):
    """Should not blow up when a progress_token is supplied."""
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_curate_catalog",
        args={"catalog": "c", "intent": "balance_class_sizes", "confirm": True},
        progress_token="prog-tok-123",
    )
    assert res.is_error is False


@pytest.mark.asyncio
async def test_calibrate_thresholds_progress_token_accepted(agent_native_dispatcher, seeded_backend):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_calibrate_thresholds",
        args={"catalog": "c"},
        progress_token="prog-tok-456",
    )
    assert res.is_error is False
