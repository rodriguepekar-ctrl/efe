"""efe_list_llm_providers (M21 Phase F) — list available providers + health."""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeListLLMProvidersInput",
    "description": (
        "List the LLM providers configured server-side, with health "
        "snapshot (available, latency_p50, latency_p99, error_rate_30d) "
        "and circuit breaker state. READ-ONLY."
    ),
    "properties": {
        "include_health": {
            "type": "boolean",
            "default": True,
        },
    },
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    include_health = bool(args.get("include_health", True))

    if not hasattr(ctx.backend, "list_llm_providers"):
        raise ToolError(
            "Backend does not support LLM provider listing (EFE 1.4 required)",
            code="not_implemented",
        )

    providers = await ctx.backend.list_llm_providers(
        include_health=include_health,
        auth=ctx.auth,
    )
    return {
        "providers": providers,
        "count": len(providers),
    }


TOOL = Tool(
    name="efe_list_llm_providers",
    description=(
        "List configured LLM providers with health + circuit breaker "
        "state. READ-ONLY. Requires `EFE_FEATURE_M21_MODE2_ENABLED`."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=False,
    profile_tier="advanced",
    feature_flag="M21_MODE2_ENABLED",
)
