"""Tests for :class:`MockProvider`."""

from __future__ import annotations

import pytest

from efe_platform.bot.providers.mock import MockProvider, _coerce_value, _parse_mock_plan
from efe_platform.bot.tool_registry import ToolSchema


def _schema(name: str = "swarm.classify") -> ToolSchema:
    return ToolSchema(
        name=name,
        description="t",
        input_schema={
            "type": "object",
            "properties": {
                "tenant": {"type": "string"},
                "text": {"type": "string"},
                "n": {"type": "integer"},
                "v": {"type": "array"},
                "flag": {"type": "boolean"},
            },
        },
    )


# ---------------------------------------------------------------------------
#  Plan phase
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_plan_conversational_no_prefix() -> None:
    p = MockProvider()
    turn = await p.run(
        phase="plan",
        system="",
        messages=[{"role": "user", "content": "Bonjour"}],
        tool_schemas=(_schema(),),
        max_tokens=256,
    )
    assert turn.tool_uses == ()
    assert "EFE" in turn.text


@pytest.mark.asyncio
async def test_plan_with_prefix_produces_call() -> None:
    p = MockProvider()
    turn = await p.run(
        phase="plan",
        system="",
        messages=[
            {
                "role": "user",
                "content": "plan: swarm.classify tenant=acme n=3 flag=true v=[1.0,2.0]",
            }
        ],
        tool_schemas=(_schema(),),
        max_tokens=256,
    )
    assert len(turn.tool_uses) == 1
    call = turn.tool_uses[0]
    assert call.tool_name == "swarm.classify"
    assert call.arguments["tenant"] == "acme"
    assert call.arguments["n"] == 3
    assert call.arguments["flag"] is True
    assert call.arguments["v"] == [1.0, 2.0]


@pytest.mark.asyncio
async def test_plan_unknown_tool_ignored() -> None:
    p = MockProvider()
    turn = await p.run(
        phase="plan",
        system="",
        messages=[{"role": "user", "content": "plan: ghost.tool x=1"}],
        tool_schemas=(_schema(),),
        max_tokens=256,
    )
    assert turn.tool_uses == ()  # unknown → dropped


@pytest.mark.asyncio
async def test_force_kind_conversational() -> None:
    p = MockProvider(force_kind="conversational")
    turn = await p.run(
        phase="plan",
        system="",
        messages=[
            {
                "role": "user",
                "content": "plan: swarm.classify tenant=acme",  # would normally fire
            }
        ],
        tool_schemas=(_schema(),),
        max_tokens=256,
    )
    assert turn.tool_uses == ()


@pytest.mark.asyncio
async def test_force_kind_tool_uses_synthesizes_call() -> None:
    """Regression: ``force_kind="tool_uses"`` without a ``plan:`` prefix
    must still produce a call, else the bot loop is stuck."""
    p = MockProvider(force_kind="tool_uses")
    turn = await p.run(
        phase="plan",
        system="",
        messages=[{"role": "user", "content": "Classe ce texte."}],
        tool_schemas=(_schema("swarm.classify"),),
        max_tokens=256,
    )
    assert len(turn.tool_uses) == 1
    assert turn.tool_uses[0].tool_name == "swarm.classify"


@pytest.mark.asyncio
async def test_force_kind_tool_uses_no_tools_falls_back() -> None:
    p = MockProvider(force_kind="tool_uses")
    turn = await p.run(
        phase="plan",
        system="",
        messages=[{"role": "user", "content": "Classe ce texte."}],
        tool_schemas=(),
        max_tokens=256,
    )
    # No tools available → conversational fallback.
    assert turn.tool_uses == ()


# ---------------------------------------------------------------------------
#  Verify phase
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_verify_with_tool_results() -> None:
    p = MockProvider()
    messages = [
        {"role": "user", "content": "plan: swarm.classify"},
        {"role": "assistant", "content": "[plan]"},
        {
            "role": "tool",
            "tool_use_id": "u1",
            "tool_name": "swarm.classify",
            "content": '{"verdict":"CONVERGED","confidence":0.85}',
        },
    ]
    turn = await p.run(
        phase="verify",
        system="",
        messages=messages,
        tool_schemas=(),
        max_tokens=256,
    )
    assert "swarm.classify" in turn.text
    assert "<json>" in turn.text


@pytest.mark.asyncio
async def test_verify_flags_divergence() -> None:
    p = MockProvider()
    messages = [
        {"role": "user", "content": "x"},
        {"role": "assistant", "content": "[plan]"},
        {
            "role": "tool",
            "tool_use_id": "u1",
            "tool_name": "swarm.classify",
            "content": '{"verdict":"DIVERGENCE","confidence":0.8}',
        },
    ]
    turn = await p.run(
        phase="verify",
        system="",
        messages=messages,
        tool_schemas=(),
        max_tokens=256,
    )
    assert "uncertain" in turn.text


@pytest.mark.asyncio
async def test_verify_without_tools_is_permissive() -> None:
    p = MockProvider()
    turn = await p.run(
        phase="verify",
        system="",
        messages=[{"role": "user", "content": "x"}],
        tool_schemas=(),
        max_tokens=256,
    )
    # Returns a default unknown verdict instead of crashing.
    assert "unknown" in turn.text


# ---------------------------------------------------------------------------
#  Phase dispatch
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_unknown_phase_raises() -> None:
    p = MockProvider()
    with pytest.raises(ValueError, match="unknown phase"):
        await p.run(
            phase="mystery",  # type: ignore[arg-type]
            system="",
            messages=[],
            tool_schemas=(),
            max_tokens=1,
        )


# ---------------------------------------------------------------------------
#  Helpers
# ---------------------------------------------------------------------------


def test_coerce_value_variants() -> None:
    assert _coerce_value("true") is True
    assert _coerce_value("false") is False
    assert _coerce_value("42") == 42
    assert _coerce_value("3.14") == 3.14
    assert _coerce_value("hello") == "hello"
    assert _coerce_value('"quoted"') == "quoted"
    assert _coerce_value("[1,2,3]") == [1, 2, 3]


def test_parse_mock_plan_empty_body_returns_empty() -> None:
    # "plan:" with nothing after should parse to no calls.
    calls = _parse_mock_plan("plan:", tool_schemas=(_schema(),))
    assert calls == ()
