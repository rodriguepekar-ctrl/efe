"""efe_find_similar_examples (M17 D43)."""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeFindSimilarExamplesInput",
    "description": "Find catalog examples nearest to a query text.",
    "properties": {
        "catalog": {"type": "string", "minLength": 1, "maxLength": 100},
        "text": {"type": "string", "minLength": 1, "maxLength": 8192},
        "top_k": {"type": "integer", "minimum": 1, "maximum": 32, "default": 5},
        "scope": {
            "type": "string",
            "enum": ["all_classes", "top_class"],
            "default": "all_classes",
        },
    },
    "required": ["catalog", "text"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    return await ctx.backend.find_similar_examples(
        catalog=args["catalog"],
        text=args["text"],
        top_k=int(args.get("top_k", 5)),
        scope=args.get("scope", "all_classes"),
        auth=ctx.auth,
    )


TOOL = Tool(
    name="efe_find_similar_examples",
    description=(
        "Return the top-k catalog examples most similar to a query text. "
        "Useful for 'what is this most similar to in my catalog' lookups. "
        "Read-only."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    profile_tier="advanced",
)
