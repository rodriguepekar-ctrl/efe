"""ToolAdapter — translate tool defs + tool calls across dialects.

The canonical form is :class:`efe_inference.messages.request.ToolDef`
for definitions and :class:`efe_inference.messages.response.ToolCall`
for calls. OpenAI and Anthropic dialects each wrap / unwrap these
shapes slightly differently; this module contains only pure
transformation functions.
"""

from __future__ import annotations

from typing import Any

from efe_inference.errors import ToolSchemaInvalid
from efe_inference.messages.request import ToolDef
from efe_inference.messages.response import ToolCall


class ToolAdapter:
    """Stateless translator between OpenAI and Anthropic tool formats.

    Canonical ↔ OpenAI : near identity (OpenAI wraps in ``function`` key).
    Canonical ↔ Anthropic : ``parameters`` becomes ``input_schema`` and
    tool calls are emitted as ``tool_use`` content blocks.
    """

    # ------------------------------------------------------------ OpenAI

    @staticmethod
    def canonical_to_openai_def(tool: ToolDef) -> dict[str, Any]:
        return {
            "type": "function",
            "function": {
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters_json_schema,
            },
        }

    @staticmethod
    def openai_to_canonical_def(raw: dict[str, Any]) -> ToolDef:
        if raw.get("type") != "function":
            raise ToolSchemaInvalid(
                f"OpenAI tool type must be 'function', got {raw.get('type')!r}",
            )
        fn = raw.get("function") or {}
        return ToolDef(
            name=str(fn.get("name", "")),
            description=str(fn.get("description", "")),
            parameters_json_schema=dict(fn.get("parameters") or {}),
        )

    @staticmethod
    def canonical_to_openai_call(call: ToolCall) -> dict[str, Any]:
        import json

        return {
            "id": call.id,
            "type": "function",
            "function": {
                "name": call.name,
                "arguments": json.dumps(call.arguments, ensure_ascii=False),
            },
        }

    @staticmethod
    def openai_to_canonical_call(raw: dict[str, Any]) -> ToolCall:
        import json

        fn = raw.get("function") or {}
        args_raw = fn.get("arguments", "{}")
        if isinstance(args_raw, str):
            try:
                args = json.loads(args_raw) if args_raw else {}
            except json.JSONDecodeError:
                args = {}
        else:
            args = dict(args_raw)
        return ToolCall(
            id=str(raw.get("id", "")),
            name=str(fn.get("name", "")),
            arguments=args if isinstance(args, dict) else {},
        )

    # ------------------------------------------------------------ Anthropic

    @staticmethod
    def canonical_to_anthropic_def(tool: ToolDef) -> dict[str, Any]:
        return {
            "name": tool.name,
            "description": tool.description,
            "input_schema": tool.parameters_json_schema,
        }

    @staticmethod
    def anthropic_to_canonical_def(raw: dict[str, Any]) -> ToolDef:
        return ToolDef(
            name=str(raw.get("name", "")),
            description=str(raw.get("description", "")),
            parameters_json_schema=dict(raw.get("input_schema") or {}),
        )

    @staticmethod
    def canonical_to_anthropic_call(call: ToolCall) -> dict[str, Any]:
        return {
            "type": "tool_use",
            "id": call.id,
            "name": call.name,
            "input": call.arguments,
        }

    @staticmethod
    def anthropic_to_canonical_call(raw: dict[str, Any]) -> ToolCall:
        return ToolCall(
            id=str(raw.get("id", "")),
            name=str(raw.get("name", "")),
            arguments=dict(raw.get("input") or {}),
        )

    # ------------------------------------------------------------ Round-trip

    @classmethod
    def round_trip_openai_anthropic(cls, tool: ToolDef) -> ToolDef:
        """Convert ToolDef to OpenAI then Anthropic then back.

        Used in test snapshots to assert I2 (round-trip bijective).
        """
        openai_form = cls.canonical_to_openai_def(tool)
        canon_a = cls.openai_to_canonical_def(openai_form)
        anthropic_form = cls.canonical_to_anthropic_def(canon_a)
        return cls.anthropic_to_canonical_def(anthropic_form)


__all__ = ["ToolAdapter"]
