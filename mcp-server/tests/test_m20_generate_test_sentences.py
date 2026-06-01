"""efe_generate_test_sentences (M20) — pin tests."""
from __future__ import annotations

import json

import pytest

from m20_helpers import make_generate_sentences_response




@pytest.mark.asyncio
async def test_generate_sentences_basic_path(
    m20_dispatcher, fake_sampling_session,
):
    fake_sampling_session.next_response = make_generate_sentences_response(count=5)
    result = await m20_dispatcher.dispatch(
        tool_name="efe_generate_test_sentences",
        args={
            "catalog": "support-routing",
            "target_class": "billing",
            "count": 5,
        },
    )
    assert result.is_error is False
    body = result.content
    assert len(body["sentences"]) == 5
    assert "rationale" in body


@pytest.mark.asyncio
async def test_generate_sentences_count_mismatch_fails(
    m20_dispatcher, fake_sampling_session,
):
    """LLM returned 3 sentences but caller requested 5 → validation fails."""
    fake_sampling_session.next_response = make_generate_sentences_response(count=3)
    result = await m20_dispatcher.dispatch(
        tool_name="efe_generate_test_sentences",
        args={"catalog": "x", "target_class": "billing", "count": 5},
    )
    assert result.is_error is True


@pytest.mark.asyncio
async def test_generate_sentences_default_count_16(
    m20_dispatcher, fake_sampling_session,
):
    fake_sampling_session.next_response = make_generate_sentences_response(count=16)
    result = await m20_dispatcher.dispatch(
        tool_name="efe_generate_test_sentences",
        args={"catalog": "x", "target_class": "billing"},
    )
    assert result.is_error is False
    assert result.content["metadata"]["count_requested"] == 16


@pytest.mark.asyncio
async def test_generate_sentences_difficulty_default_mixed(
    m20_dispatcher, fake_sampling_session,
):
    fake_sampling_session.next_response = make_generate_sentences_response(count=2)
    result = await m20_dispatcher.dispatch(
        tool_name="efe_generate_test_sentences",
        args={"catalog": "x", "target_class": "y", "count": 2},
    )
    assert result.content["metadata"]["difficulty"] == "mixed"


@pytest.mark.asyncio
async def test_generate_sentences_validates_difficulty_enum(
    m20_dispatcher, fake_sampling_session,
):
    """LLM-self-assessment difficulty must be in the valid set."""
    fake_sampling_session.next_response = json.dumps({
        "sentences": ["s1"],
        "difficulty_self_assessment": {"s1": "trivial"},  # invalid
        "rationale": "x" * 30,
    })
    result = await m20_dispatcher.dispatch(
        tool_name="efe_generate_test_sentences",
        args={"catalog": "x", "target_class": "y", "count": 1},
    )
    assert result.is_error is True


@pytest.mark.asyncio
async def test_generate_sentences_emits_audit(
    m20_dispatcher, fake_sampling_session, audit_emitter,
):
    audit_emitter.enabled = False
    fake_sampling_session.next_response = make_generate_sentences_response(count=2)
    before = len(audit_emitter.recent(limit=500))
    await m20_dispatcher.dispatch(
        tool_name="efe_generate_test_sentences",
        args={"catalog": "x", "target_class": "y", "count": 2},
    )
    after = len(audit_emitter.recent(limit=500))
    assert after == before + 1


@pytest.mark.asyncio
async def test_generate_sentences_blocks_when_feature_disabled(
    m20_off_dispatcher, fake_sampling_session,
):
    result = await m20_off_dispatcher.dispatch(
        tool_name="efe_generate_test_sentences",
        args={"catalog": "x", "target_class": "y", "count": 5},
    )
    assert result.is_error is True
