"""Canonical ChatRequest / ChatMessage invariants."""

from __future__ import annotations

import pytest

from efe_inference.messages.request import (
    ChatMessage,
    ChatRequest,
    ImageBlock,
    Role,
    SamplingParams,
    TextBlock,
    ToolDef,
)
from efe_inference.messages.response import (
    ChatChoice,
    ChatResponse,
    FinishReason,
    Usage,
)


def test_chat_request_requires_messages():
    with pytest.raises(ValueError):
        ChatRequest(
            tenant_id="t", task_id="q", request_id="r",
            messages=(),
        )


def test_chat_request_requires_tenant():
    with pytest.raises(ValueError):
        ChatRequest(
            tenant_id="", task_id="q", request_id="r",
            messages=(ChatMessage(role=Role.USER, content=(TextBlock("hi"),)),),
        )


def test_sampling_params_reject_bad_temperature():
    with pytest.raises(ValueError):
        SamplingParams(temperature=5.0)


def test_tool_def_validates_identifier():
    with pytest.raises(ValueError):
        ToolDef(name="bad-name!", description="x")
    ToolDef(name="get_weather", description="x")  # ok


def test_total_text_concatenates_messages():
    req = ChatRequest(
        tenant_id="t", task_id="q", request_id="r",
        messages=(
            ChatMessage(role=Role.SYSTEM, content=(TextBlock("be concise"),)),
            ChatMessage(role=Role.USER,   content=(TextBlock("hi"),)),
        ),
    )
    assert "be concise" in req.total_text()
    assert "hi" in req.total_text()


def test_usage_rejects_inconsistent_total():
    with pytest.raises(ValueError):
        Usage(prompt_tokens=3, completion_tokens=2, total_tokens=10)


def test_image_block_requires_url():
    with pytest.raises(ValueError):
        ImageBlock(image_url="")


def test_chat_response_shape():
    resp = ChatResponse(
        request_id="r",
        model="mock",
        choices=(
            ChatChoice(
                index=0,
                message=ChatMessage(
                    role=Role.ASSISTANT,
                    content=(TextBlock("hello"),),
                ),
                finish_reason=FinishReason.STOP,
            ),
        ),
        usage=Usage(prompt_tokens=1, completion_tokens=2, total_tokens=3),
        backend_kind="mock",
        backend_latency_ms=1.0,
        adapter_overhead_ms=0.5,
    )
    assert resp.choices[0].message.text() == "hello"
    assert resp.usage.total_tokens == 3
