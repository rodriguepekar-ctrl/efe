"""Streaming-tool pin tests (M17 D45+D46)."""
from __future__ import annotations

import pytest


@pytest.fixture
def seeded_backend(fake_backend):
    fake_backend._seed_class("bulk", [
        {"name": "a", "examples": ["alpha", "ace", "ant"]},
        {"name": "b", "examples": ["beta", "bee"]},
    ])
    return fake_backend


@pytest.mark.asyncio
async def test_bulk_classify_max_1000_enforced(agent_native_dispatcher, seeded_backend):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_bulk_classify",
        args={"catalog": "bulk", "texts": ["x"] * 1500},
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_bulk_classify_returns_summary(agent_native_dispatcher, seeded_backend):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_bulk_classify",
        args={"catalog": "bulk", "texts": ["alpha", "beta", "gamma"]},
    )
    assert res.is_error is False, res.content
    assert "summary" in res.content
    assert res.content["summary"]["total"] == 3


@pytest.mark.asyncio
async def test_bulk_classify_per_item_results(agent_native_dispatcher, seeded_backend):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_bulk_classify",
        args={"catalog": "bulk", "texts": ["alpha", "beta"]},
    )
    assert len(res.content["results"]) == 2
    for r in res.content["results"]:
        assert "predicted_class" in r
        assert "score" in r


@pytest.mark.asyncio
async def test_export_catalog_default_format_json(agent_native_dispatcher, seeded_backend):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_export_catalog",
        args={"catalog": "bulk"},
    )
    assert res.is_error is False
    assert res.content["format"] == "json"
    assert "content" in res.content


@pytest.mark.asyncio
async def test_calibrate_thresholds_returns_per_class(agent_native_dispatcher, seeded_backend):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_calibrate_thresholds",
        args={"catalog": "bulk", "target_metric": "f1", "target_value": 0.8},
    )
    assert res.is_error is False
    assert "per_class" in res.content
    assert len(res.content["per_class"]) == 2


@pytest.mark.asyncio
async def test_calibrate_thresholds_invalid_target_metric(agent_native_dispatcher, seeded_backend):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_calibrate_thresholds",
        args={"catalog": "bulk", "target_metric": "made-up"},
    )
    assert res.is_error is True
    assert res.content["code"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_consolidate_versions_dry_run_default(agent_native_dispatcher, seeded_backend):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_consolidate_versions",
        args={"catalog": "bulk", "keep_last_n": 1},
    )
    assert res.is_error is False
    assert res.content["dry_run"] is True


@pytest.mark.asyncio
async def test_consolidate_versions_apply_requires_confirm(agent_native_dispatcher, seeded_backend):
    """Audit fix M18-FINAL-A5 promoted this from a runtime check to a
    schema-level conditional (`if dry_run==false then required: confirm`).
    The error code is therefore `invalid_arguments` (schema rejected) ;
    pre-fix it was `confirm_required` (handler rejected). Either way
    the request is denied — schema catches it earlier."""
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_consolidate_versions",
        args={"catalog": "bulk", "keep_last_n": 1, "dry_run": False},
    )
    assert res.is_error is True
    assert res.content["code"] in ("confirm_required", "invalid_arguments")


@pytest.mark.asyncio
async def test_consolidate_versions_apply_with_confirm(agent_native_dispatcher, seeded_backend):
    res = await agent_native_dispatcher.dispatch(
        tool_name="efe_consolidate_versions",
        args={"catalog": "bulk", "keep_last_n": 1, "dry_run": False, "confirm": True},
    )
    assert res.is_error is False
    assert res.content["dry_run"] is False
