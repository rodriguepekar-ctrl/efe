"""OpenAI / Anthropic / native normalisers."""

from __future__ import annotations

import pytest

from efe_inference.errors import RequestError, UnsupportedMessageRole
from efe_inference.messages.normalizer import (
    AnthropicNormalizer,
    NativeNormalizer,
    OpenAINormalizer,
)
from efe_inference.messages.request import Role


def test_openai_normalize_basic():
    raw = {
        "model": "gpt-4o",
        "messages": [
            {"role": "system", "content": "Be concise."},
            {"role": "user", "content": "Hello"},
        ],
        "temperature": 0.5,
        "max_tokens": 128,
    }
    req = OpenAINormalizer.normalize(raw, tenant_id="t", task_id="q")
    assert len(req.messages) == 2
    assert req.messages[0].role == Role.SYSTEM
    assert req.sampling.max_tokens == 128
    assert req.model_hint == "gpt-4o"


def test_openai_rejects_missing_messages():
    with pytest.raises(RequestError):
        OpenAINormalizer.normalize({}, tenant_id="t", task_id="q")


def test_openai_rejects_unknown_role():
    raw = {"messages": [{"role": "martian", "content": "x"}]}
    with pytest.raises(UnsupportedMessageRole):
        OpenAINormalizer.normalize(raw, tenant_id="t", task_id="q")


def test_openai_normalize_tool_call():
    raw = {
        "messages": [
            {"role": "user", "content": "What's the weather?"},
            {
                "role": "assistant",
                "content": None,
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "arguments": '{"city": "Paris"}',
                    },
                }],
            },
        ],
    }
    req = OpenAINormalizer.normalize(raw, tenant_id="t", task_id="q")
    tool_blocks = [
        b for m in req.messages for b in m.content
        if b.__class__.__name__ == "ToolUseBlock"
    ]
    assert len(tool_blocks) == 1
    assert tool_blocks[0].tool_name == "get_weather"
    assert tool_blocks[0].tool_args == {"city": "Paris"}


def test_anthropic_normalize_with_system():
    raw = {
        "model": "claude-sonnet-3.5",
        "system": "Be concise.",
        "messages": [{"role": "user", "content": "Hi"}],
        "max_tokens": 256,
    }
    req = AnthropicNormalizer.normalize(raw, tenant_id="t", task_id="q")
    assert req.messages[0].role == Role.SYSTEM
    assert "Be concise" in req.messages[0].text()
    assert req.messages[1].role == Role.USER


def test_anthropic_image_block():
    raw = {
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": "See this"},
                {"type": "image", "source": {
                    "type": "base64", "data": "iVBOR...",
                    "media_type": "image/png",
                }},
            ],
        }],
    }
    req = AnthropicNormalizer.normalize(raw, tenant_id="t", task_id="q")
    blocks = list(req.messages[0].content)
    assert blocks[0].__class__.__name__ == "TextBlock"
    assert blocks[1].__class__.__name__ == "ImageBlock"


def test_native_normalize():
    raw = {
        "task_id": "q",
        "messages": [{
            "role": "user",
            "content": [{"kind": "text", "text": "Hi"}],
        }],
    }
    req = NativeNormalizer.normalize(raw, tenant_id="t", task_id="q")
    assert len(req.messages) == 1
    assert req.messages[0].text() == "Hi"
