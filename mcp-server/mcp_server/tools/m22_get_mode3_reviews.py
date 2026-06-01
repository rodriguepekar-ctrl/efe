"""efe_get_mode3_reviews (M22 Phase E) — list pending reviews."""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeGetMode3ReviewsInput",
    "description": (
        "List Mode-3 reviews for the current tenant (filtered by "
        "status). Tenant is derived from JWT claim, never user-supplied."
    ),
    "properties": {
        "status": {
            "type": "string",
            "enum": ["pending", "approved", "rejected", "expired", "all"],
            "default": "pending",
        },
        "limit": {
            "type": "integer",
            "minimum": 1,
            "maximum": 100,
            "default": 20,
        },
        "offset": {
            "type": "integer",
            "minimum": 0,
            "default": 0,
        },
    },
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not hasattr(ctx.backend, "list_mode3_reviews"):
        raise ToolError(
            "Backend does not support Mode-3 reviews (EFE 1.5 required)",
            code="not_implemented",
        )
    status = args.get("status", "pending")
    limit = int(args.get("limit", 20))
    offset = int(args.get("offset", 0))

    result = await ctx.backend.list_mode3_reviews(
        status=status,
        limit=limit,
        offset=offset,
        auth=ctx.auth,
    )
    reviews = result.get("reviews", [])
    return {
        "reviews": reviews,
        "count": len(reviews),
        "offset": offset,
        "has_more": result.get("has_more", False),
    }


TOOL = Tool(
    name="efe_get_mode3_reviews",
    description=(
        "List pending Mode-3 reviews for the current tenant. "
        "READ-ONLY. Requires `EFE_FEATURE_M22_MODE3_ENABLED`."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=False,
    profile_tier="agent-native",
    feature_flag="M22_MODE3_ENABLED",
)
