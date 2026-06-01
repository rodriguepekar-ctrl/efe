"""efe_classify tool — classify text against a catalog (read)."""
from __future__ import annotations

from typing import Any

from ..audit_emitter import scrub_text
from .types import Tool, ToolContext


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeClassifyInput",
    "description": "Classify a text against a catalog and return top-k classes.",
    "properties": {
        "catalog": {
            "type": "string",
            "minLength": 1,
            "maxLength": 100,
            "description": "Name of the catalog (e.g. 'support-routing'). Caller must have access via tenant.",
        },
        "text": {
            "type": "string",
            "minLength": 1,
            "maxLength": 8192,
            "description": "Text to classify. PII is scrubbed before audit logging (D13).",
        },
        "top_k": {
            "type": "integer",
            "minimum": 1,
            "maximum": 32,
            "default": 3,
            "description": "Number of top-scoring classes to return.",
        },
    },
    "required": ["catalog", "text"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    catalog = args["catalog"]
    text = args["text"]
    top_k = int(args.get("top_k", 3))

    # BackendError subclasses are caught + mapped by the dispatcher so the
    # MCP error code reflects the underlying status (404 → not_found, etc.)
    result = await ctx.backend.classify(
        catalog=catalog, text=text, top_k=top_k, auth=ctx.auth
    )

    # Trim raw text out of the response — the LLM doesn't need a copy of
    # what it just sent. Helps with token budget.
    response = {
        "catalog_name": result.get("catalog_name", catalog),
        "catalog_version": result.get("catalog_version"),
        "fingerprint": result.get("fingerprint"),
        "top_k": result.get("top_k", []),
        "embedding_norm": result.get("embedding_norm"),
        "embedder_info": result.get("embedder_info"),
        "input_text_preview": scrub_text(text[:80]),
    }
    return response


TOOL = Tool(
    name="efe_classify",
    description=(
        "Classify a text against an EFE catalog and return the top-k classes "
        "with cosine similarity scores. Read-only ; safe to call freely."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=False,
)
