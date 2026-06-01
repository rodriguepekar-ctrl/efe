"""efe_analyze_confusion_matrix (M20) — pin tests."""
from __future__ import annotations

import json

import pytest

from m20_helpers import make_confusion_response




@pytest.mark.asyncio
async def test_analyze_confusion_basic(
    m20_dispatcher, fake_sampling_session,
):
    fake_sampling_session.next_response = make_confusion_response()
    result = await m20_dispatcher.dispatch(
        tool_name="efe_analyze_confusion_matrix",
        args={
            "catalog": "support-routing",
            "confusion_matrix": {
                "billing": {"billing": 95, "refund": 5},
                "refund":  {"billing": 12, "refund": 88},
            },
            "min_confusion_rate": 0.05,
            "max_pairs": 5,
        },
    )
    assert result.is_error is False
    assert "proposed_actions" in result.content


@pytest.mark.asyncio
async def test_analyze_confusion_no_pairs_above_threshold(
    m20_dispatcher, fake_sampling_session,
):
    """No call to LLM if no pair above min_rate ; canned response."""
    result = await m20_dispatcher.dispatch(
        tool_name="efe_analyze_confusion_matrix",
        args={
            "catalog": "x",
            "confusion_matrix": {
                "a": {"a": 99, "b": 1},
                "b": {"a": 1, "b": 99},
            },
            "min_confusion_rate": 0.5,  # very high
        },
    )
    assert result.is_error is False
    assert result.content["top_confusion_pairs"] == []
    assert result.content["proposed_actions"] == []
    # No sampling call should have been made
    assert len(fake_sampling_session.calls) == 0


@pytest.mark.asyncio
async def test_analyze_confusion_filters_below_min_rate(
    m20_dispatcher, fake_sampling_session,
):
    fake_sampling_session.next_response = make_confusion_response()
    await m20_dispatcher.dispatch(
        tool_name="efe_analyze_confusion_matrix",
        args={
            "catalog": "x",
            "confusion_matrix": {
                "a": {"a": 95, "b": 5},  # rate 0.05
                "b": {"a": 12, "b": 88},  # rate 0.12
            },
            "min_confusion_rate": 0.10,
        },
    )
    # The prompt must contain only pair {b → a}, not {a → b}
    call = fake_sampling_session.calls[-1]
    prompt_text = ""
    for m in call["messages"]:
        c = m.get("content")
        if isinstance(c, dict):
            prompt_text += c.get("text", "")
    assert '"true_class": "b"' in prompt_text
    assert '"true_class": "a"' not in prompt_text


@pytest.mark.asyncio
async def test_analyze_confusion_caps_at_max_pairs(
    m20_dispatcher, fake_sampling_session,
):
    fake_sampling_session.next_response = make_confusion_response()
    matrix = {
        f"c{i}": {f"c{i}": 50, f"c{j}": 50}
        for i, j in zip(range(5), [1, 2, 3, 4, 0])
    }
    await m20_dispatcher.dispatch(
        tool_name="efe_analyze_confusion_matrix",
        args={
            "catalog": "x",
            "confusion_matrix": matrix,
            "min_confusion_rate": 0.05,
            "max_pairs": 2,
        },
    )
    call = fake_sampling_session.calls[-1]
    prompt_text = ""
    for m in call["messages"]:
        c = m.get("content")
        if isinstance(c, dict):
            prompt_text += c.get("text", "")
    # We count `"rate": 0.5` — a per-pair JSON literal that appears
    # only in the serialised data payload (the schema description in
    # the system prompt uses `<float 0-1>`, not a numeric literal).
    # With max_pairs=2, exactly 2 entries.
    assert prompt_text.count('"rate": 0.5') == 2


@pytest.mark.asyncio
async def test_analyze_confusion_validates_action_enum(
    m20_dispatcher, fake_sampling_session,
):
    fake_sampling_session.next_response = json.dumps({
        "top_confusion_pairs": [],
        "proposed_actions": [{"action": "rename", "classes": ["a"]}],
        "summary": "x" * 30,
    })
    result = await m20_dispatcher.dispatch(
        tool_name="efe_analyze_confusion_matrix",
        args={
            "catalog": "x",
            "confusion_matrix": {
                "a": {"a": 50, "b": 50},
                "b": {"a": 5, "b": 95},
            },
            "min_confusion_rate": 0.05,
        },
    )
    assert result.is_error is True


@pytest.mark.asyncio
async def test_analyze_confusion_emits_audit(
    m20_dispatcher, fake_sampling_session, audit_emitter,
):
    audit_emitter.enabled = False
    fake_sampling_session.next_response = make_confusion_response()
    before = len(audit_emitter.recent(limit=500))
    await m20_dispatcher.dispatch(
        tool_name="efe_analyze_confusion_matrix",
        args={
            "catalog": "x",
            "confusion_matrix": {
                "a": {"a": 50, "b": 50},
                "b": {"a": 5, "b": 95},
            },
            "min_confusion_rate": 0.05,
        },
    )
    after = len(audit_emitter.recent(limit=500))
    assert after == before + 1


@pytest.mark.asyncio
async def test_analyze_confusion_blocks_when_feature_disabled(
    m20_off_dispatcher, fake_sampling_session,
):
    result = await m20_off_dispatcher.dispatch(
        tool_name="efe_analyze_confusion_matrix",
        args={
            "catalog": "x",
            "confusion_matrix": {"a": {"a": 1}},
        },
    )
    assert result.is_error is True
