"""Pin tests for the efe-byo-agent _validation module.

Mirrors mcp-server/tests/test_m20_validation.py — both implementations
should accept/reject the same payloads (parity guarantee).
"""
from __future__ import annotations

import json

import pytest

from efe_byo_agent._validation import validate_with_retry
from efe_byo_agent.exceptions import ValidationFailedError


@pytest.mark.asyncio
async def test_validate_summarize_classification_happy_path():
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
    )
    assert result["confidence_assessment"] == "high"


@pytest.mark.asyncio
async def test_validate_strips_markdown_fences():
    body = json.dumps({
        "explanation": "x" * 50,
        "key_factors": ["a"],
        "confidence_assessment": "low",
        "suggested_alternatives": [],
    })
    raw = f"```json\n{body}\n```"
    result = await validate_with_retry(
        helper="summarize_classification",
        raw_output=raw,
        retry_callback=None,
    )
    assert result["confidence_assessment"] == "low"


@pytest.mark.asyncio
async def test_validate_retries_on_invalid_then_succeeds():
    good = json.dumps({
        "explanation": "x" * 50,
        "key_factors": ["a"],
        "confidence_assessment": "medium",
        "suggested_alternatives": [],
    })

    async def cb(_errs):
        return good

    result = await validate_with_retry(
        helper="summarize_classification",
        raw_output="not json",
        retry_callback=cb,
        max_retries=2,
    )
    assert result["confidence_assessment"] == "medium"


@pytest.mark.asyncio
async def test_validate_fails_after_max_retries():
    async def cb(_errs):
        return "still bad"

    with pytest.raises(ValidationFailedError) as exc:
        await validate_with_retry(
            helper="summarize_classification",
            raw_output="bad",
            retry_callback=cb,
            max_retries=2,
        )
    assert exc.value.helper == "summarize_classification"


@pytest.mark.asyncio
async def test_validate_compare_bundles_recommendation_enum():
    raw = json.dumps({
        "accuracy_delta_pp": 1.0,
        "divergence_examples": [],
        "summary": "x" * 30,
        "recommendation": "ship_it",
    })
    with pytest.raises(ValidationFailedError):
        await validate_with_retry(
            helper="compare_bundles_quality",
            raw_output=raw,
            retry_callback=None,
        )


@pytest.mark.asyncio
async def test_validate_generate_sentences_count_match():
    raw = json.dumps({
        "sentences": ["a", "b", "c"],
        "difficulty_self_assessment": {"a": "easy", "b": "medium", "c": "hard"},
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
async def test_validate_runbook_eta_range():
    raw = json.dumps({
        "runbook_markdown": "x" * 250,
        "estimated_resolution_time_min": 9999,
        "primary_diagnostics": ["d"],
        "risk_level": "low",
    })
    with pytest.raises(ValidationFailedError):
        await validate_with_retry(
            helper="draft_runbook",
            raw_output=raw,
            retry_callback=None,
        )


@pytest.mark.asyncio
async def test_validate_confusion_action_enum():
    raw = json.dumps({
        "top_confusion_pairs": [],
        "proposed_actions": [{"action": "rename", "classes": ["a"]}],
        "summary": "x" * 30,
    })
    with pytest.raises(ValidationFailedError):
        await validate_with_retry(
            helper="analyze_confusion_matrix",
            raw_output=raw,
            retry_callback=None,
        )
