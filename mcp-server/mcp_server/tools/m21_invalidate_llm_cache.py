"""efe_invalidate_llm_cache (M21 Phase F) — write, confirm:true."""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeInvalidateLLMCacheInput",
    "description": (
        "Invalidate the LLM cache for a tenant or a specific catalog. "
        "WRITE — requires `confirm:true`. Use after bundle promotion "
        "to ensure stale cached anchors don't bleed into production."
    ),
    "properties": {
        "scope": {
            "type": "string",
            "enum": ["catalog", "tenant"],
        },
        "catalog": {
            "type": "string",
            "minLength": 1,
            "maxLength": 100,
            "description": "Required when scope=catalog.",
        },
        "confirm": {
            "type": "boolean",
            "const": True,
        },
    },
    "required": ["scope", "confirm"],
    "additionalProperties": False,
    "if": {"properties": {"scope": {"const": "catalog"}}, "required": ["scope"]},
    "then": {"required": ["catalog"]},
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not args.get("confirm"):
        raise ToolError(
            "invalidate_llm_cache requires confirm:true",
            code="confirm_required",
        )
    scope = args["scope"]
    if scope == "catalog" and not args.get("catalog"):
        raise ToolError(
            "scope=catalog requires `catalog` arg",
            code="invalid_arguments",
        )
    if not hasattr(ctx.backend, "invalidate_llm_cache"):
        raise ToolError(
            "Backend does not support LLM cache invalidation (EFE 1.4 required)",
            code="not_implemented",
        )

    result = await ctx.backend.invalidate_llm_cache(
        scope=scope,
        catalog=args.get("catalog"),
        auth=ctx.auth,
    )
    return {
        "scope": scope,
        "catalog": args.get("catalog"),
        "deleted_count": result.get("deleted_count", 0),
        "applied_at": result.get("applied_at"),
    }


TOOL = Tool(
    name="efe_invalidate_llm_cache",
    description=(
        "Invalidate the LLM cache for a tenant or catalog. WRITE. "
        "Requires `confirm:true`. Useful after bundle promotion to "
        "ensure stale cached Mode-2 anchors are cleared. Requires "
        "`EFE_FEATURE_M21_MODE2_ENABLED`."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=True,
    profile_tier="agent-native",
    feature_flag="M21_MODE2_ENABLED",
)
