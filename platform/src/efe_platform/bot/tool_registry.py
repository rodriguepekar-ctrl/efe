"""Typed registry of tools the LLM may invoke.

Each tool is an async function accepting a dict of args (validated
against a JSON schema) and returning a dict result. The registry:

* enforces **schema validation** at call time,
* collects prometheus-friendly metrics (call counts, latency),
* exposes a JSON tool schema consumable by the Anthropic / OpenAI SDKs
  via :meth:`ToolRegistry.to_anthropic_schema` and
  :meth:`ToolRegistry.to_openai_schema`.

The MVP registers three swarm-backed tools:

* ``swarm.classify`` — run one Execute RPC and return the verdict.
* ``swarm.describe`` — DescribeSpec dry-run.
* ``swarm.list_cones`` — list registered cones for the tenant.
"""

from __future__ import annotations

import time
from collections.abc import Awaitable, Callable, Mapping
from dataclasses import dataclass
from typing import Any

from efe_platform.errors import ToolArgsInvalid, ToolInvocationError, UnknownTool

# ---------------------------------------------------------------------------
#  Value types
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class ToolSchema:
    """Declarative description of a tool's inputs.

    The schema is a subset of JSON Schema sufficient for LLM providers:
    it must be an object with a ``properties`` map and an optional
    ``required`` list.
    """

    name: str
    description: str
    input_schema: Mapping[str, Any]

    def __post_init__(self) -> None:
        if not self.name:
            raise ValueError("tool name must be non-empty")
        if not self.description:
            raise ValueError("tool description must be non-empty")
        if self.input_schema.get("type") != "object":
            raise ValueError("input_schema must describe an object")
        if "properties" not in self.input_schema:
            raise ValueError("input_schema must include properties")


@dataclass(frozen=True, slots=True)
class ToolResult:
    """The outcome of one invocation."""

    tool_name: str
    tool_use_id: str
    ok: bool
    value: Any
    duration_ms: float
    error: str | None = None

    def to_dict(self) -> dict[str, Any]:
        return {
            "tool_name": self.tool_name,
            "tool_use_id": self.tool_use_id,
            "ok": self.ok,
            "value": self.value,
            "duration_ms": round(self.duration_ms, 3),
            "error": self.error,
        }


ToolHandler = Callable[[Mapping[str, Any]], Awaitable[Any]]


@dataclass(slots=True)
class Tool:
    """A registered tool — schema + handler + live metrics."""

    schema: ToolSchema
    handler: ToolHandler
    calls: int = 0
    failures: int = 0
    total_duration_ms: float = 0.0


# ---------------------------------------------------------------------------
#  Registry
# ---------------------------------------------------------------------------


class ToolRegistry:
    """Collection of tools keyed by name."""

    def __init__(self) -> None:
        self._tools: dict[str, Tool] = {}

    # --------------------------------------------------------- Registration

    def register(self, schema: ToolSchema, handler: ToolHandler) -> None:
        if schema.name in self._tools:
            raise ValueError(f"tool {schema.name!r} already registered")
        self._tools[schema.name] = Tool(schema=schema, handler=handler)

    def unregister(self, name: str) -> None:
        self._tools.pop(name, None)

    def __contains__(self, name: str) -> bool:
        return name in self._tools

    def names(self) -> tuple[str, ...]:
        return tuple(self._tools.keys())

    # --------------------------------------------------------- Introspection

    def schema_for(self, name: str) -> ToolSchema:
        if name not in self._tools:
            raise UnknownTool(f"unknown tool {name!r}")
        return self._tools[name].schema

    def schemas(self) -> tuple[ToolSchema, ...]:
        """Return all schemas in registration order — provider-neutral.

        The planner passes this tuple to :meth:`LlmProvider.run`; each
        provider adapter is then responsible for translating to its
        own tool schema format (Anthropic vs OpenAI vs …).
        """
        return tuple(t.schema for t in self._tools.values())

    def metrics(self) -> dict[str, dict[str, float | int]]:
        return {
            name: {
                "calls": t.calls,
                "failures": t.failures,
                "total_duration_ms": round(t.total_duration_ms, 3),
                "avg_duration_ms": round(t.total_duration_ms / max(1, t.calls), 3),
            }
            for name, t in self._tools.items()
        }

    # ------------------------------------------------------------- Exports

    def to_anthropic_schema(self) -> list[dict[str, Any]]:
        """Schema list formatted for ``anthropic.messages.create(tools=...)``."""
        return [
            {
                "name": t.schema.name,
                "description": t.schema.description,
                "input_schema": dict(t.schema.input_schema),
            }
            for t in self._tools.values()
        ]

    def to_openai_schema(self) -> list[dict[str, Any]]:
        """Schema list formatted for ``openai.chat.completions.create(tools=...)``."""
        return [
            {
                "type": "function",
                "function": {
                    "name": t.schema.name,
                    "description": t.schema.description,
                    "parameters": dict(t.schema.input_schema),
                },
            }
            for t in self._tools.values()
        ]

    # ------------------------------------------------------------- Dispatch

    async def call(
        self,
        *,
        name: str,
        tool_use_id: str,
        args: Mapping[str, Any],
    ) -> ToolResult:
        """Validate args, dispatch, wrap exceptions into a ToolResult."""
        if name not in self._tools:
            raise UnknownTool(f"unknown tool {name!r}")
        tool = self._tools[name]
        _validate_against(tool.schema.input_schema, args)
        t0 = time.monotonic()
        try:
            value = await tool.handler(args)
        except ToolInvocationError:
            tool.calls += 1
            tool.failures += 1
            tool.total_duration_ms += (time.monotonic() - t0) * 1000
            raise
        except Exception as exc:
            tool.calls += 1
            tool.failures += 1
            tool.total_duration_ms += (time.monotonic() - t0) * 1000
            raise ToolInvocationError(
                f"tool {name!r} crashed: {exc}",
                detail={"tool_use_id": tool_use_id},
            ) from exc
        dt_ms = (time.monotonic() - t0) * 1000
        tool.calls += 1
        tool.total_duration_ms += dt_ms
        return ToolResult(
            tool_name=name,
            tool_use_id=tool_use_id,
            ok=True,
            value=value,
            duration_ms=dt_ms,
        )


# ---------------------------------------------------------------------------
#  Schema validator — tiny subset of JSON Schema
# ---------------------------------------------------------------------------


def _validate_against(schema: Mapping[str, Any], args: Mapping[str, Any]) -> None:
    """Validate that ``args`` fits ``schema``; raise :class:`ToolArgsInvalid`.

    Supports:
      - required properties,
      - primitive ``type`` (string / integer / number / boolean / object / array),
      - ``enum`` on strings.

    Does NOT support oneOf, allOf, $ref — intentionally narrow to keep
    the platform dependency-free. The Rust server re-validates anyway.
    """
    required = schema.get("required", []) or []
    for r in required:
        if r not in args:
            raise ToolArgsInvalid(f"missing required field {r!r}")
    properties = schema.get("properties", {}) or {}
    for key, value in args.items():
        if key not in properties:
            raise ToolArgsInvalid(f"unknown field {key!r}")
        prop = properties[key]
        expected = prop.get("type")
        if expected and not _type_matches(value, expected):
            raise ToolArgsInvalid(
                f"field {key!r} expected type {expected}, got {type(value).__name__}"
            )
        enum = prop.get("enum")
        if enum and value not in enum:
            raise ToolArgsInvalid(
                f"field {key!r} must be one of {enum}, got {value!r}"
            )


def _type_matches(value: Any, expected: str) -> bool:
    mapping: dict[str, type | tuple[type, ...]] = {
        "string": str,
        "integer": int,
        "number": (int, float),
        "boolean": bool,
        "object": dict,
        "array": (list, tuple),
    }
    py_type = mapping.get(expected)
    if py_type is None:  # pragma: no cover — schema author bug
        return True
    # bool is a subclass of int in Python — exclude that false positive.
    if expected == "integer" and isinstance(value, bool):
        return False
    return isinstance(value, py_type)


__all__ = [
    "Tool",
    "ToolHandler",
    "ToolRegistry",
    "ToolResult",
    "ToolSchema",
]
