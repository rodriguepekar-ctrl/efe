"""efe_diagnose_catalog (M17 D43)."""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeDiagnoseCatalogInput",
    "description": "Full health report for a catalog : ambiguity, coverage, balance, drift.",
    "properties": {
        "catalog": {"type": "string", "minLength": 1, "maxLength": 100},
        "depth": {
            "type": "string",
            "enum": ["quick", "standard", "full"],
            "default": "standard",
        },
        "include_examples_text": {"type": "boolean", "default": False},
    },
    "required": ["catalog"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    return await ctx.backend.diagnose_catalog(
        catalog=args["catalog"],
        depth=args.get("depth", "standard"),
        include_examples_text=bool(args.get("include_examples_text", False)),
        auth=ctx.auth,
    )


TOOL = Tool(
    name="efe_diagnose_catalog",
    description=(
        "Run a full health diagnostic on a catalog : ambiguity matrix, "
        "coverage gaps, class balance, threshold quality, drift signals, "
        "and recommended actions. PII in examples is scrubbed by default. "
        "Read-only."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    profile_tier="advanced",
)
