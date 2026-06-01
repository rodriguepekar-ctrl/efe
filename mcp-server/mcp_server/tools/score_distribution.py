"""efe_score_distribution (M17 D43)."""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeScoreDistributionInput",
    "description": "Histogram of scores for a class against random / provided texts.",
    "properties": {
        "catalog": {"type": "string", "minLength": 1, "maxLength": 100},
        "class_name": {"type": "string", "minLength": 1, "maxLength": 100},
        "sample_count": {
            "type": "integer", "minimum": 10, "maximum": 1000, "default": 100,
        },
        "texts": {
            "type": "array",
            "items": {"type": "string", "minLength": 1, "maxLength": 8192},
            "maxItems": 1000,
        },
        "include_negatives": {"type": "boolean", "default": True},
    },
    "required": ["catalog", "class_name"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    return await ctx.backend.score_distribution(
        catalog=args["catalog"],
        class_name=args["class_name"],
        sample_count=int(args.get("sample_count", 100)),
        texts=args.get("texts"),
        include_negatives=bool(args.get("include_negatives", True)),
        auth=ctx.auth,
    )


TOOL = Tool(
    name="efe_score_distribution",
    description=(
        "Build a histogram of similarity scores for a class against N "
        "random or provided texts. Returns percentiles + recommended "
        "threshold maximizing F1. Read-only."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    profile_tier="advanced",
)
