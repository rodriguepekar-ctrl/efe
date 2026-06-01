"""efe_summarize_classification (M20) — pin tests."""
from __future__ import annotations

import json

import pytest

from m20_helpers import make_summarize_response




@pytest.mark.asyncio
async def test_summarize_classification_basic_path(
    m20_dispatcher, fake_sampling_session,
):
    fake_sampling_session.next_response = make_summarize_response()
    result = await m20_dispatcher.dispatch(
        tool_name="efe_summarize_classification",
        args={
            "catalog": "support-routing",
            "sentence": "My laptop won't boot",
            "classification_result": {
                "label": "hardware_issue",
                "confidence": 0.87,
                "anchors_used": ["hardware", "boot"],
            },
        },
    )
    assert result.is_error is False
    body = result.content
    assert body["confidence_assessment"] == "high"
    assert "explanation" in body
    assert isinstance(body["key_factors"], list)


@pytest.mark.asyncio
async def test_summarize_classification_validates_output_schema(
    m20_dispatcher, fake_sampling_session,
):
    fake_sampling_session.next_response = json.dumps({
        "explanation": "x" * 50,
        "key_factors": ["a"],
        "confidence_assessment": "supreme",  # invalid enum
        "suggested_alternatives": [],
    })
    result = await m20_dispatcher.dispatch(
        tool_name="efe_summarize_classification",
        args={
            "catalog": "x",
            "sentence": "hello",
            "classification_result": {"label": "x", "confidence": 0.5},
        },
    )
    # After 2 retries with same bad enum, dispatch returns error
    assert result.is_error is True


@pytest.mark.asyncio
async def test_summarize_classification_retries_on_invalid_json(
    m20_dispatcher, fake_sampling_session,
):
    """First call returns garbage, retries find valid output."""
    fake_sampling_session.responses_by_prompt = {
        "Validation errors": make_summarize_response(),
    }
    fake_sampling_session.next_response = "not json"

    result = await m20_dispatcher.dispatch(
        tool_name="efe_summarize_classification",
        args={
            "catalog": "x",
            "sentence": "hello",
            "classification_result": {"label": "x", "confidence": 0.5},
        },
    )
    assert result.is_error is False
    assert len(fake_sampling_session.calls) >= 2  # at least 1 retry


@pytest.mark.asyncio
async def test_summarize_classification_redacts_pii_in_prompt(
    m20_dispatcher, fake_sampling_session,
):
    """PII in input sentence must be redacted before reaching sampling."""
    fake_sampling_session.next_response = make_summarize_response()
    await m20_dispatcher.dispatch(
        tool_name="efe_summarize_classification",
        args={
            "catalog": "x",
            "sentence": "Contact john@example.com",
            "classification_result": {"label": "x", "confidence": 0.5},
        },
    )
    # Sampling input must NOT contain the raw email
    for call in fake_sampling_session.calls:
        msgs = call.get("messages", [])
        for m in msgs:
            content = m.get("content")
            if isinstance(content, dict):
                text = content.get("text", "")
            else:
                text = str(content)
            assert "john@example.com" not in text


@pytest.mark.asyncio
async def test_summarize_classification_emits_audit_event(
    m20_dispatcher, fake_sampling_session, audit_emitter,
):
    audit_emitter.enabled = False  # don't actually POST
    fake_sampling_session.next_response = make_summarize_response()
    before = len(audit_emitter.recent(limit=500))
    await m20_dispatcher.dispatch(
        tool_name="efe_summarize_classification",
        args={
            "catalog": "x",
            "sentence": "hello",
            "classification_result": {"label": "x", "confidence": 0.5},
        },
    )
    after = len(audit_emitter.recent(limit=500))
    assert after == before + 1
    last = audit_emitter.recent(limit=1)[0]
    assert last.tool == "efe_summarize_classification"


@pytest.mark.asyncio
async def test_summarize_classification_blocks_when_feature_disabled(
    m20_off_dispatcher, fake_sampling_session,
):
    fake_sampling_session.next_response = make_summarize_response()
    result = await m20_off_dispatcher.dispatch(
        tool_name="efe_summarize_classification",
        args={
            "catalog": "x",
            "sentence": "hello",
            "classification_result": {"label": "x", "confidence": 0.5},
        },
    )
    # Tool not registered → dispatch returns tool_not_found error
    assert result.is_error is True


@pytest.mark.asyncio
async def test_summarize_classification_accepts_language_hint(
    m20_dispatcher, fake_sampling_session,
):
    fake_sampling_session.next_response = make_summarize_response()
    result = await m20_dispatcher.dispatch(
        tool_name="efe_summarize_classification",
        args={
            "catalog": "x",
            "sentence": "hello",
            "classification_result": {"label": "x", "confidence": 0.5},
            "language_hint": "fr",
        },
    )
    assert result.is_error is False
    assert result.content["metadata"]["language"] == "fr"
