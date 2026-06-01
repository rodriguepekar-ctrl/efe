"""Prompt templater — stability + family dispatch."""

from __future__ import annotations

import pytest

from efe_inference.messages.request import (
    ChatMessage,
    ChatRequest,
    Role,
    TextBlock,
)
from efe_inference.templating.families import ModelFamily
from efe_inference.templating.templater import PromptTemplater


def _req(text: str) -> ChatRequest:
    return ChatRequest(
        tenant_id="t", task_id="q", request_id="r",
        messages=(
            ChatMessage(role=Role.SYSTEM, content=(TextBlock("be concise"),)),
            ChatMessage(role=Role.USER, content=(TextBlock(text),)),
        ),
    )


def test_render_structured_family_returns_messages():
    t = PromptTemplater()
    rendered = t.render(_req("hi"), family=ModelFamily.CLAUDE)
    assert rendered.messages is not None
    assert rendered.textual is None
    assert rendered.family == ModelFamily.CLAUDE


def test_render_llama3_contains_begin_of_text():
    t = PromptTemplater()
    rendered = t.render(_req("hi"), family=ModelFamily.LLAMA3)
    assert rendered.textual is not None
    assert "<|begin_of_text|>" in rendered.textual
    assert "<|start_header_id|>user<|end_header_id|>" in rendered.textual


def test_render_chatml_format():
    t = PromptTemplater()
    rendered = t.render(_req("hi"), family=ModelFamily.CHATML)
    assert rendered.textual is not None
    assert "<|im_start|>user" in rendered.textual
    assert "<|im_end|>" in rendered.textual


def test_render_hash_stable_same_input():
    t = PromptTemplater()
    a = t.render(_req("hello"), family=ModelFamily.LLAMA3)
    b = t.render(_req("hello"), family=ModelFamily.LLAMA3)
    assert a.hash_hex == b.hash_hex


def test_render_hash_changes_on_content_change():
    t = PromptTemplater()
    a = t.render(_req("hello"), family=ModelFamily.LLAMA3)
    b = t.render(_req("world"), family=ModelFamily.LLAMA3)
    assert a.hash_hex != b.hash_hex


def test_render_unknown_version_raises():
    t = PromptTemplater()
    with pytest.raises(KeyError):
        t.render(_req("hi"), family=ModelFamily.LLAMA3, version="99.9")
