"""Tests for :class:`ConversationMemory`."""

from __future__ import annotations

import pytest

from efe_platform.bot.memory import ConversationMemory, MemoryMessage


def test_add_user_and_assistant() -> None:
    mem = ConversationMemory()
    mem.add_user("Bonjour")
    mem.add_assistant("Salut !")
    snap = mem.snapshot()
    assert len(snap) == 2
    assert snap[0].role == "user"
    assert snap[0].content == "Bonjour"
    assert snap[1].role == "assistant"
    assert snap[1].approx_tokens > 0


def test_add_tool_result_encodes_json() -> None:
    mem = ConversationMemory()
    mem.add_tool_result(
        tool_use_id="abc",
        tool_name="swarm.classify",
        result={"verdict": "CONVERGED", "confidence": 0.9},
    )
    snap = mem.snapshot()
    assert snap[0].role == "tool"
    assert "CONVERGED" in snap[0].content
    assert snap[0].tool_use_id == "abc"
    assert snap[0].tool_name == "swarm.classify"


def test_max_messages_respected() -> None:
    mem = ConversationMemory(max_messages=3, max_approx_tokens=10_000)
    for i in range(10):
        mem.add_user(f"msg {i}")
    assert len(mem) == 3
    # Oldest dropped first.
    assert "msg 9" in mem.snapshot()[-1].content


def test_max_tokens_respected() -> None:
    mem = ConversationMemory(max_messages=1000, max_approx_tokens=20)
    for _ in range(20):
        mem.add_user("x" * 40)  # ~10 tokens each
    assert mem.approx_token_count <= 20


def test_tool_message_requires_ids() -> None:
    with pytest.raises(ValueError, match="tool_use_id"):
        MemoryMessage(role="tool", content="x")


def test_clear() -> None:
    mem = ConversationMemory()
    mem.add_user("hi")
    mem.add_assistant("hello")
    mem.clear()
    assert len(mem) == 0
    assert mem.approx_token_count == 0


def test_to_dict_roundtrips() -> None:
    mem = ConversationMemory()
    mem.add_user("bonjour")
    snap = mem.snapshot()
    d = snap[0].to_dict()
    assert d["role"] == "user"
    assert d["content"] == "bonjour"
    assert "approx_tokens" in d


def test_empty_constructor_args_rejected() -> None:
    with pytest.raises(ValueError):
        ConversationMemory(max_messages=0)
    with pytest.raises(ValueError):
        ConversationMemory(max_approx_tokens=0)
