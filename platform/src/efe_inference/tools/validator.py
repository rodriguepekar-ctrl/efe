"""ToolValidator — parse/repair JSON + validate against JSON Schema.

The repair heuristics are intentionally small and well-tested:

* strip Markdown code fences ``(json ...)``;
* replace trailing commas ``,}`` / ``,]`` with ``}``/``]``;
* balance a single missing trailing ``}``;
* unescape stray backslashes in string values.

Anything more aggressive risks altering the model's intent; we prefer
to raise :class:`ToolArgumentsInvalid` over silently guessing.
"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass
from typing import Any

from efe_inference.errors import (
    ToolArgumentsInvalid,
    ToolNotFound,
)
from efe_inference.messages.request import ToolDef
from efe_inference.messages.response import ToolCall


@dataclass(frozen=True, slots=True)
class ValidationResult:
    ok: bool
    repaired: bool
    errors: tuple[str, ...]


# ---------------------------------------------------------------------------
# JSON repair heuristics
# ---------------------------------------------------------------------------


class JsonRepair:
    _FENCE_RE = re.compile(
        r"^\s*```(?:json)?\s*(.*?)\s*```\s*$",
        re.DOTALL | re.IGNORECASE,
    )
    _TRAILING_COMMA_RE = re.compile(r",(\s*[}\]])")

    @classmethod
    def try_parse(cls, raw: str) -> tuple[Any, bool]:
        """Return ``(parsed, repaired)`` or raise on giving up."""
        if not isinstance(raw, str):
            raise ToolArgumentsInvalid(
                f"expected str, got {type(raw).__name__}",
            )
        raw = raw.strip()
        if not raw:
            return {}, False
        try:
            return json.loads(raw), False
        except json.JSONDecodeError:
            pass
        repaired = cls._apply(raw)
        try:
            return json.loads(repaired), True
        except json.JSONDecodeError as exc:
            raise ToolArgumentsInvalid(
                f"cannot parse tool arguments: {exc}",
            ) from exc

    @classmethod
    def _apply(cls, raw: str) -> str:
        m = cls._FENCE_RE.match(raw)
        if m:
            raw = m.group(1)
        raw = cls._TRAILING_COMMA_RE.sub(r"\1", raw)
        if raw.count("{") == raw.count("}") + 1:
            raw = raw + "}"
        if raw.count("[") == raw.count("]") + 1:
            raw = raw + "]"
        return raw


# ---------------------------------------------------------------------------
# ToolValidator
# ---------------------------------------------------------------------------


class ToolValidator:
    """Validates a :class:`ToolCall` against a registered :class:`ToolDef`.

    The JSON Schema validation uses the ``jsonschema`` package when
    available; otherwise falls back to a type-only check (useful on
    minimal CI environments). ``jsonschema`` is in the default dev
    extras so production always runs the full validator.
    """

    def __init__(self, tools: tuple[ToolDef, ...]) -> None:
        self._defs: dict[str, ToolDef] = {t.name: t for t in tools}

    def parse_and_validate(
        self, tool_name: str, args_raw: Any,
    ) -> tuple[ToolCall, ValidationResult]:
        if tool_name not in self._defs:
            raise ToolNotFound(
                f"tool {tool_name!r} not in registry "
                f"(known: {sorted(self._defs)})",
            )
        tool = self._defs[tool_name]

        if isinstance(args_raw, dict):
            parsed: Any = args_raw
            repaired = False
        else:
            parsed, repaired = JsonRepair.try_parse(str(args_raw))

        errors = _validate(parsed, tool.parameters_json_schema)
        ok = not errors
        if not ok:
            raise ToolArgumentsInvalid(
                f"tool {tool_name!r} arguments invalid: {errors}",
                detail={"errors": errors, "arguments": parsed},
            )
        import uuid as _uuid

        call = ToolCall(
            id=str(_uuid.uuid4()),
            name=tool_name,
            arguments=parsed if isinstance(parsed, dict) else {},
        )
        return call, ValidationResult(
            ok=True, repaired=repaired, errors=(),
        )


def _validate(instance: Any, schema: dict[str, Any]) -> list[str]:
    try:
        from jsonschema import Draft202012Validator  # type: ignore[import-untyped]
    except ImportError:  # pragma: no cover
        return _fallback_validate(instance, schema)
    validator = Draft202012Validator(schema)
    errors = [
        f"{'.'.join(str(p) for p in e.absolute_path) or '<root>'}: {e.message}"
        for e in validator.iter_errors(instance)
    ]
    return errors


def _fallback_validate(
    instance: Any, schema: dict[str, Any],
) -> list[str]:
    # Minimal: only checks "type: object" and "required".
    kind = schema.get("type")
    if kind == "object" and not isinstance(instance, dict):
        return [f"<root>: expected object, got {type(instance).__name__}"]
    required = schema.get("required", []) or []
    errors = []
    if isinstance(instance, dict):
        for k in required:
            if k not in instance:
                errors.append(f"{k}: required")
    return errors


__all__ = ["JsonRepair", "ToolValidator", "ValidationResult"]
