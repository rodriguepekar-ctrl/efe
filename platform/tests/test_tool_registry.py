"""Tests for :class:`ToolRegistry` — dispatch, schema validation, metrics."""

from __future__ import annotations

from collections.abc import Mapping
from typing import Any

import pytest

from efe_platform.bot.tool_registry import ToolRegistry, ToolSchema
from efe_platform.errors import ToolArgsInvalid, ToolInvocationError, UnknownTool


def _schema(name: str = "t") -> ToolSchema:
    return ToolSchema(
        name=name,
        description="dummy tool",
        input_schema={
            "type": "object",
            "properties": {
                "text": {"type": "string"},
                "limit": {"type": "integer"},
                "mode": {"type": "string", "enum": ["a", "b"]},
            },
            "required": ["text"],
        },
    )


async def _ok_handler(args: Mapping[str, Any]) -> dict[str, Any]:
    return {"echo": args.get("text"), "limit": args.get("limit")}


async def _raise_handler(_args: Mapping[str, Any]) -> dict[str, Any]:
    raise RuntimeError("boom")


async def _typed_error_handler(_args: Mapping[str, Any]) -> dict[str, Any]:
    raise ToolInvocationError("deliberate", detail={"tool_use_id": "x"})


# ---------------------------------------------------------------------------
#  Registration
# ---------------------------------------------------------------------------


def test_register_and_list() -> None:
    r = ToolRegistry()
    r.register(_schema("alpha"), _ok_handler)
    r.register(_schema("beta"), _ok_handler)
    assert set(r.names()) == {"alpha", "beta"}
    assert "alpha" in r


def test_double_registration_raises() -> None:
    r = ToolRegistry()
    r.register(_schema(), _ok_handler)
    with pytest.raises(ValueError, match="already registered"):
        r.register(_schema(), _ok_handler)


def test_unregister() -> None:
    r = ToolRegistry()
    r.register(_schema(), _ok_handler)
    r.unregister("t")
    assert "t" not in r


def test_unknown_tool_schema() -> None:
    r = ToolRegistry()
    with pytest.raises(UnknownTool):
        r.schema_for("ghost")


# ---------------------------------------------------------------------------
#  Schema validation
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_call_happy_path() -> None:
    r = ToolRegistry()
    r.register(_schema(), _ok_handler)
    result = await r.call(name="t", tool_use_id="u1", args={"text": "hi", "limit": 3})
    assert result.ok
    assert result.value == {"echo": "hi", "limit": 3}
    assert result.duration_ms >= 0


@pytest.mark.asyncio
async def test_missing_required_field() -> None:
    r = ToolRegistry()
    r.register(_schema(), _ok_handler)
    with pytest.raises(ToolArgsInvalid, match="missing required"):
        await r.call(name="t", tool_use_id="u", args={"limit": 1})


@pytest.mark.asyncio
async def test_unknown_field() -> None:
    r = ToolRegistry()
    r.register(_schema(), _ok_handler)
    with pytest.raises(ToolArgsInvalid, match="unknown field"):
        await r.call(name="t", tool_use_id="u", args={"text": "x", "extra": 1})


@pytest.mark.asyncio
async def test_wrong_type() -> None:
    r = ToolRegistry()
    r.register(_schema(), _ok_handler)
    with pytest.raises(ToolArgsInvalid, match="expected type integer"):
        await r.call(name="t", tool_use_id="u", args={"text": "x", "limit": "3"})


@pytest.mark.asyncio
async def test_enum_violation() -> None:
    r = ToolRegistry()
    r.register(_schema(), _ok_handler)
    with pytest.raises(ToolArgsInvalid, match="must be one of"):
        await r.call(name="t", tool_use_id="u", args={"text": "x", "mode": "c"})


@pytest.mark.asyncio
async def test_bool_not_integer() -> None:
    """Regression: ``isinstance(True, int) is True`` — must be rejected."""
    r = ToolRegistry()
    r.register(_schema(), _ok_handler)
    with pytest.raises(ToolArgsInvalid):
        await r.call(name="t", tool_use_id="u", args={"text": "x", "limit": True})


# ---------------------------------------------------------------------------
#  Error handling
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_crash_wrapped_as_tool_invocation_error() -> None:
    r = ToolRegistry()
    r.register(_schema(), _raise_handler)
    with pytest.raises(ToolInvocationError, match="crashed"):
        await r.call(name="t", tool_use_id="u", args={"text": "x"})


@pytest.mark.asyncio
async def test_typed_error_preserved() -> None:
    r = ToolRegistry()
    r.register(_schema(), _typed_error_handler)
    with pytest.raises(ToolInvocationError, match="deliberate"):
        await r.call(name="t", tool_use_id="u", args={"text": "x"})


# ---------------------------------------------------------------------------
#  Metrics + schemas
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_metrics_counted() -> None:
    r = ToolRegistry()
    r.register(_schema(), _ok_handler)
    r.register(_schema("raise"), _raise_handler)
    await r.call(name="t", tool_use_id="u", args={"text": "a"})
    await r.call(name="t", tool_use_id="u", args={"text": "b"})
    with pytest.raises(ToolInvocationError):
        await r.call(name="raise", tool_use_id="u", args={"text": "x"})
    m = r.metrics()
    assert m["t"]["calls"] == 2
    assert m["t"]["failures"] == 0
    assert m["raise"]["calls"] == 1
    assert m["raise"]["failures"] == 1


def test_anthropic_schema_shape() -> None:
    r = ToolRegistry()
    r.register(_schema("alpha"), _ok_handler)
    schemas = r.to_anthropic_schema()
    assert schemas[0]["name"] == "alpha"
    assert schemas[0]["input_schema"]["type"] == "object"


def test_openai_schema_shape() -> None:
    r = ToolRegistry()
    r.register(_schema("alpha"), _ok_handler)
    schemas = r.to_openai_schema()
    assert schemas[0]["type"] == "function"
    assert schemas[0]["function"]["name"] == "alpha"
    assert "parameters" in schemas[0]["function"]
