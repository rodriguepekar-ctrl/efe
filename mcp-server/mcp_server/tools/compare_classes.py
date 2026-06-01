"""efe_compare_classes (M17 D43)."""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeCompareClassesInput",
    "description": "Compare two classes within a catalog for ambiguity.",
    "properties": {
        "catalog": {"type": "string", "minLength": 1, "maxLength": 100},
        "class_a": {"type": "string", "minLength": 1, "maxLength": 100},
        "class_b": {"type": "string", "minLength": 1, "maxLength": 100},
    },
    "required": ["catalog", "class_a", "class_b"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if args["class_a"] == args["class_b"]:
        raise ToolError(
            "class_a and class_b must differ ; self-comparison has no meaning",
            code="invalid_arguments",
            status_code=400,
        )
    return await ctx.backend.compare_classes(
        catalog=args["catalog"],
        class_a=args["class_a"],
        class_b=args["class_b"],
        auth=ctx.auth,
    )


TOOL = Tool(
    name="efe_compare_classes",
    description=(
        "Compare two classes within an EFE catalog and return centroid "
        "separation + boundary examples. Useful to detect ambiguity "
        "before deploying. Read-only."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    profile_tier="advanced",
)
