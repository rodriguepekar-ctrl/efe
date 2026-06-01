"""M20 output validation unit tests."""
from __future__ import annotations

import json

import pytest

from mcp_server.tools.m20_validation import validate_with_retry
from mcp_server.tools.types import ToolError


# ─────────────────────────────────────────────────────────────────────
# summarize_classification
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_validate_summarize_happy_path():
    raw = json.dumps({
        "explanation": "x" * 50,
        "key_factors": ["a", "b"],
        "confidence_assessment": "high",
        "suggested_alternatives": [],
    })
    result = await validate_with_retry(
        helper="summarize_classification",
        raw_output=raw,
        retry_callback=None,
        max_retries=0,
    )
    assert result["confidence_assessment"] == "high"
    assert len(result["explanation"]) >= 20


@pytest.mark.asyncio
async def test_validate_strips_markdown_fences():
    raw = '```json\n{"explanation": "' + "x" * 50 + '", "key_factors": ["a"], "confidence_assessment": "low", "suggested_alternatives": []}\n```'
    result = await validate_with_retry(
        helper="summarize_classification",
        raw_output=raw,
        retry_callback=None,
    )
    assert result["confidence_assessment"] == "low"


@pytest.mark.asyncio
async def test_validate_retries_on_invalid_json():
    bad = "not json"
    good = json.dumps({
        "explanation": "x" * 50,
        "key_factors": ["a"],
        "confidence_assessment": "medium",
        "suggested_alternatives": [],
    })

    async def cb(_errors):
        return good

    result = await validate_with_retry(
        helper="summarize_classification",
        raw_output=bad,
        retry_callback=cb,
        max_retries=2,
    )
    assert result["confidence_assessment"] == "medium"


@pytest.mark.asyncio
async def test_validate_fails_after_max_retries():
    async def cb(_errors):
        return "still bad"

    with pytest.raises(ToolError):
        await validate_with_retry(
            helper="summarize_classification",
            raw_output="bad",
            retry_callback=cb,
            max_retries=2,
        )


@pytest.mark.asyncio
async def test_validate_rejects_invalid_enum():
    raw = json.dumps({
        "explanation": "x" * 50,
        "key_factors": ["a"],
        "confidence_assessment": "supreme",  # invalid
        "suggested_alternatives": [],
    })
    with pytest.raises(ToolError):
        await validate_with_retry(
            helper="summarize_classification",
            raw_output=raw,
            retry_callback=None,
            max_retries=0,
        )


# ─────────────────────────────────────────────────────────────────────
# generate_test_sentences (count enforced)
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_validate_generate_sentences_count_match():
    raw = json.dumps({
        "sentences": ["s1", "s2", "s3"],
        "difficulty_self_assessment": {"s1": "easy", "s2": "medium", "s3": "hard"},
        "rationale": "x" * 30,
    })
    result = await validate_with_retry(
        helper="generate_test_sentences",
        raw_output=raw,
        retry_callback=None,
        expected_count=3,
    )
    assert len(result["sentences"]) == 3


@pytest.mark.asyncio
async def test_validate_generate_sentences_count_mismatch_fails():
    raw = json.dumps({
        "sentences": ["s1", "s2"],
        "difficulty_self_assessment": {"s1": "easy", "s2": "medium"},
        "rationale": "x" * 30,
    })
    with pytest.raises(ToolError):
        await validate_with_retry(
            helper="generate_test_sentences",
            raw_output=raw,
            retry_callback=None,
            expected_count=5,
        )


# ─────────────────────────────────────────────────────────────────────
# compare_bundles_quality
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_validate_compare_bundles_recommendation_enum():
    raw = json.dumps({
        "accuracy_delta_pp": 1.5,
        "divergence_examples": [],
        "summary": "x" * 30,
        "recommendation": "ship_it",  # invalid
    })
    with pytest.raises(ToolError):
        await validate_with_retry(
            helper="compare_bundles_quality",
            raw_output=raw,
            retry_callback=None,
        )


@pytest.mark.asyncio
async def test_validate_compare_bundles_delta_range():
    raw = json.dumps({
        "accuracy_delta_pp": 9999,  # out of range
        "divergence_examples": [],
        "summary": "x" * 30,
        "recommendation": "promote",
    })
    with pytest.raises(ToolError):
        await validate_with_retry(
            helper="compare_bundles_quality",
            raw_output=raw,
            retry_callback=None,
        )


# ─────────────────────────────────────────────────────────────────────
# analyze_confusion_matrix
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_validate_confusion_action_enum():
    raw = json.dumps({
        "top_confusion_pairs": [],
        "proposed_actions": [{"action": "rename", "classes": ["a"]}],  # invalid
        "summary": "x" * 30,
    })
    with pytest.raises(ToolError):
        await validate_with_retry(
            helper="analyze_confusion_matrix",
            raw_output=raw,
            retry_callback=None,
        )


# ─────────────────────────────────────────────────────────────────────
# draft_runbook
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_validate_runbook_eta_range():
    raw = json.dumps({
        "runbook_markdown": "x" * 250,
        "estimated_resolution_time_min": 9999,  # out of [1, 240]
        "primary_diagnostics": ["check x"],
        "risk_level": "low",
    })
    with pytest.raises(ToolError):
        await validate_with_retry(
            helper="draft_runbook",
            raw_output=raw,
            retry_callback=None,
        )


@pytest.mark.asyncio
async def test_validate_runbook_min_length():
    raw = json.dumps({
        "runbook_markdown": "too short",
        "estimated_resolution_time_min": 30,
        "primary_diagnostics": ["check x"],
        "risk_level": "low",
    })
    with pytest.raises(ToolError):
        await validate_with_retry(
            helper="draft_runbook",
            raw_output=raw,
            retry_callback=None,
        )
