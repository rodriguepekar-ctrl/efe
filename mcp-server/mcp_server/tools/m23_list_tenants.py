"""efe_list_tenants (D8) — read-only tenant listing.

Optionally filter by status. Pagination is server-side (cursor),
but the v1.0.0 CLI fetches everything in one call (≤ 10k tenants).
"""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeListTenantsInput",
    "description": "Read-only tenant listing. Optional status filter.",
    "properties": {
        "status": {"type": "string", "enum": ["active", "suspended", "any"]},
        "limit": {"type": "integer", "minimum": 1, "maximum": 10_000, "default": 1_000},
    },
    "required": [],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not hasattr(ctx.backend, "list_tenants"):
        raise ToolError(
            "Backend does not support list_tenants (EFE 1.6 required)",
            code="not_implemented",
        )
    try:
        result = await ctx.backend.list_tenants(
            status=args.get("status", "any"),
            limit=int(args.get("limit", 1_000)),
            auth=ctx.auth,
        )
    except Exception as e:
        raise ToolError(f"list_tenants failed: {e}", code="backend_error") from e
    return {"tenants": result.get("tenants", [])}


TOOL = Tool(
    name="efe_list_tenants",
    description="Read-only tenant listing with optional status filter.",
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=False,
    profile_tier="agent-native",
    feature_flag="D8_ADMIN_ENABLED",
)
