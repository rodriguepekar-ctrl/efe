"""efe_detect_drift (M17 D43)."""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeDetectDriftInput",
    "description": "Compare two versions of a catalog and report drift.",
    "properties": {
        "catalog": {"type": "string", "minLength": 1, "maxLength": 100},
        "version_a": {"type": "integer", "minimum": 1},
        "version_b": {"type": "integer", "minimum": 1},
    },
    "required": ["catalog"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    return await ctx.backend.detect_drift(
        catalog=args["catalog"],
        version_a=args.get("version_a"),
        version_b=args.get("version_b"),
        auth=ctx.auth,
    )


TOOL = Tool(
    name="efe_detect_drift",
    description=(
        "Compare two versions of a catalog and report per-class centroid "
        "shift, examples added/removed, new/removed classes, and a plain-"
        "English recommendation. Default = latest vs previous. Read-only."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    profile_tier="advanced",
)
