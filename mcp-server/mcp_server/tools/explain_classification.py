"""efe_explain_classification (M17 D43).

Explains why a top-1 classification was returned : contributors,
runner-up, gap, threshold sensitivity, ambiguity signal.

Backend strategy :
  1. Try EFE 1.1 endpoint POST /catalog/<name>/explain.
  2. On 404 / not_supported, fall back to a 2-step composition :
     - call classify(top_k=2) to learn predicted_class + runner-up,
     - locally compute contributors from the version snapshot
       (requires example embeddings — only when backend exposes them).
  3. If neither path is feasible, return a partial explanation with
     `fallback: "minimal"` so the agent knows the limitation.
"""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeExplainClassificationInput",
    "description": "Explain why a given classification was returned.",
    "properties": {
        "catalog": {"type": "string", "minLength": 1, "maxLength": 100},
        "text": {"type": "string", "minLength": 1, "maxLength": 8192},
        "predicted_class": {"type": "string", "minLength": 1, "maxLength": 100},
    },
    "required": ["catalog", "text"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    catalog = args["catalog"]
    text = args["text"]
    predicted_class = args.get("predicted_class")
    return await ctx.backend.explain_classification(
        catalog=catalog,
        text=text,
        predicted_class=predicted_class,
        auth=ctx.auth,
    )


TOOL = Tool(
    name="efe_explain_classification",
    description=(
        "Explain why a classification was returned. Returns the predicted "
        "class, runner-up, gap, top-3 contributor examples, ambiguity "
        "signal, and threshold sensitivity range. Read-only."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=False,
    profile_tier="advanced",
)
