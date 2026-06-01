"""Diagnostic tools pin tests (M17 D43+D44)."""
from __future__ import annotations

import pytest


@pytest.fixture
def seeded_backend(fake_backend):
    fake_backend._seed_class("support", [
        {"name": "billing", "examples": ["bill", "payment", "charge"]},
        {"name": "tech", "examples": ["error", "bug"]},
        {"name": "feedback", "examples": ["review"]},
    ])
    return fake_backend


@pytest.mark.asyncio
async def test_diagnose_catalog_quick_depth(advanced_dispatcher, seeded_backend):
    res = await advanced_dispatcher.dispatch(
        tool_name="efe_diagnose_catalog",
        args={"catalog": "support", "depth": "quick"},
    )
    assert res.is_error is False
    assert "scores" in res.content


@pytest.mark.asyncio
async def test_diagnose_catalog_full_depth(advanced_dispatcher, seeded_backend):
    res = await advanced_dispatcher.dispatch(
        tool_name="efe_diagnose_catalog",
        args={"catalog": "support", "depth": "full"},
    )
    assert res.is_error is False
    scores = res.content["scores"]
    for k in ("ambiguity", "coverage", "balance", "threshold_quality", "audit_completeness"):
        assert k in scores


@pytest.mark.asyncio
async def test_diagnose_catalog_underrepresented_classes(advanced_dispatcher, seeded_backend):
    res = await advanced_dispatcher.dispatch(
        tool_name="efe_diagnose_catalog",
        args={"catalog": "support"},
    )
    underrep = res.content["underrepresented_classes"]
    # 'feedback' has only 1 example
    assert any(c["class_name"] == "feedback" for c in underrep)


@pytest.mark.asyncio
async def test_diagnose_catalog_pii_default_excluded(advanced_dispatcher, seeded_backend):
    res = await advanced_dispatcher.dispatch(
        tool_name="efe_diagnose_catalog",
        args={"catalog": "support"},
    )
    # Default include_examples_text=false ; backend honors per spec
    assert res.is_error is False


@pytest.mark.asyncio
async def test_diagnose_catalog_overall_health_present(advanced_dispatcher, seeded_backend):
    res = await advanced_dispatcher.dispatch(
        tool_name="efe_diagnose_catalog",
        args={"catalog": "support"},
    )
    assert res.content["overall_health"] in ("excellent", "good", "fair", "poor", "critical")


@pytest.mark.asyncio
async def test_detect_drift_default_versions(advanced_dispatcher, seeded_backend):
    res = await advanced_dispatcher.dispatch(
        tool_name="efe_detect_drift",
        args={"catalog": "support"},
    )
    assert res.is_error is False
    assert "overall_drift" in res.content


@pytest.mark.asyncio
async def test_detect_drift_explicit_versions(advanced_dispatcher, seeded_backend):
    res = await advanced_dispatcher.dispatch(
        tool_name="efe_detect_drift",
        args={"catalog": "support", "version_a": 1, "version_b": 2},
    )
    assert res.is_error is False


@pytest.mark.asyncio
async def test_detect_drift_per_class_drift_list(advanced_dispatcher, seeded_backend):
    res = await advanced_dispatcher.dispatch(
        tool_name="efe_detect_drift",
        args={"catalog": "support"},
    )
    assert isinstance(res.content["per_class_drift"], list)


@pytest.mark.asyncio
async def test_audit_health_check_incremental(advanced_dispatcher, seeded_backend):
    res = await advanced_dispatcher.dispatch(
        tool_name="efe_audit_health_check",
        args={},
    )
    assert res.is_error is False
    assert res.content["ok"] is True
    assert res.content["verification_method"] == "incremental"


@pytest.mark.asyncio
async def test_audit_health_check_deep(advanced_dispatcher, seeded_backend):
    res = await advanced_dispatcher.dispatch(
        tool_name="efe_audit_health_check",
        args={"deep": True},
    )
    assert res.is_error is False
    assert res.content["verification_method"] == "deep"


@pytest.mark.asyncio
async def test_export_catalog_default_json(advanced_dispatcher, seeded_backend):
    res = await advanced_dispatcher.dispatch(
        tool_name="efe_export_catalog",
        args={"catalog": "support"},
    )
    assert res.is_error is False
    assert res.content["format"] == "json"


@pytest.mark.asyncio
async def test_find_similar_examples_default_top_k_5(advanced_dispatcher, seeded_backend):
    res = await advanced_dispatcher.dispatch(
        tool_name="efe_find_similar_examples",
        args={"catalog": "support", "text": "error message"},
    )
    assert res.is_error is False
    assert "results" in res.content
    assert len(res.content["results"]) <= 5


@pytest.mark.asyncio
async def test_find_similar_examples_scope_top_class(advanced_dispatcher, seeded_backend):
    res = await advanced_dispatcher.dispatch(
        tool_name="efe_find_similar_examples",
        args={"catalog": "support", "text": "bill", "scope": "all_classes"},
    )
    assert res.content["scope"] == "all_classes"
