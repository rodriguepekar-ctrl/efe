"""efe_get_llm_budget (M21 Phase F) — read tenant budget state."""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeGetLLMBudgetInput",
    "description": (
        "Read the current tenant's LLM budget state (cap, spent, "
        "pct_used, fail_open, soft_threshold, last_rollover). "
        "Tenant_id is derived from the JWT, never user-supplied."
    ),
    "properties": {},
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not hasattr(ctx.backend, "get_llm_budget"):
        raise ToolError(
            "Backend does not support LLM budget queries (EFE 1.4 required)",
            code="not_implemented",
        )
    state = await ctx.backend.get_llm_budget(auth=ctx.auth)
    return {
        "monthly_cap_usd": state.get("monthly_cap_usd"),
        "spent_this_month_usd": state.get("spent_this_month_usd"),
        "pct_used": state.get("pct_used"),
        "soft_threshold_pct": state.get("soft_threshold_pct"),
        "fail_open": state.get("fail_open"),
        "rollover_dom": state.get("rollover_dom"),
        "last_rollover": state.get("last_rollover"),
        "warning": state.get("pct_used", 0) >= state.get("soft_threshold_pct", 80),
    }


TOOL = Tool(
    name="efe_get_llm_budget",
    description=(
        "Read the current tenant's LLM budget state. Returns cap, "
        "spent_this_month, pct_used, fail_open mode, soft threshold "
        "warning. READ-ONLY. Requires the `EFE_FEATURE_M21_MODE2_ENABLED` "
        "flag."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=False,
    profile_tier="core",
    feature_flag="M21_MODE2_ENABLED",
)
