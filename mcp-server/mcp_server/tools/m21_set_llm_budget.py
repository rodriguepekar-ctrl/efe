"""efe_set_llm_budget (M21 Phase F) — write tool, confirm:true required.

Profile : `agent-native-deploy` only (admin tier).
"""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeSetLLMBudgetInput",
    "description": (
        "Set the tenant's monthly LLM budget cap. WRITE — requires "
        "`confirm:true`. Cap = 0 disables Mode-2 entirely. Default "
        "soft_threshold = 80%, fail_open = false (hard cap)."
    ),
    "properties": {
        "monthly_cap_usd": {
            "type": "number",
            "minimum": 0,
            "maximum": 100_000,
            "description": "Hard monthly cap in USD. 0 → all calls rejected.",
        },
        "soft_threshold_pct": {
            "type": "integer",
            "minimum": 0,
            "maximum": 100,
            "default": 80,
        },
        "fail_open": {
            "type": "boolean",
            "default": False,
            "description": (
                "If true, calls over cap fall back to Mode-1 silently. "
                "If false, return 429 BUDGET_EXHAUSTED."
            ),
        },
        "rollover_dom": {
            "type": "integer",
            "minimum": 1,
            "maximum": 28,
            "default": 1,
        },
        "confirm": {
            "type": "boolean",
            "const": True,
            "description": "Required for write tool (M16 carry-over).",
        },
    },
    "required": ["monthly_cap_usd", "confirm"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not args.get("confirm"):
        raise ToolError(
            "set_llm_budget requires confirm:true",
            code="confirm_required",
        )
    if not hasattr(ctx.backend, "set_llm_budget"):
        raise ToolError(
            "Backend does not support LLM budget management (EFE 1.4 required)",
            code="not_implemented",
        )

    monthly_cap = float(args["monthly_cap_usd"])
    soft_threshold = int(args.get("soft_threshold_pct", 80))
    fail_open = bool(args.get("fail_open", False))
    rollover_dom = int(args.get("rollover_dom", 1))

    result = await ctx.backend.set_llm_budget(
        monthly_cap_usd=monthly_cap,
        soft_threshold_pct=soft_threshold,
        fail_open=fail_open,
        rollover_dom=rollover_dom,
        auth=ctx.auth,
    )
    return {
        "monthly_cap_usd": result.get("monthly_cap_usd", monthly_cap),
        "soft_threshold_pct": result.get("soft_threshold_pct", soft_threshold),
        "fail_open": result.get("fail_open", fail_open),
        "rollover_dom": result.get("rollover_dom", rollover_dom),
        "applied_at": result.get("applied_at"),
    }


TOOL = Tool(
    name="efe_set_llm_budget",
    description=(
        "Set the tenant's monthly LLM budget cap. WRITE. Requires "
        "`confirm:true` and `agent-native-deploy` profile. Cap=0 "
        "disables Mode-2. Requires `EFE_FEATURE_M21_MODE2_ENABLED` flag."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=True,
    profile_tier="agent-native-deploy",
    feature_flag="M21_MODE2_ENABLED",
)
