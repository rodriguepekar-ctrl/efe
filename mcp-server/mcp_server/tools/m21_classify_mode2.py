"""efe_classify_mode2 (M21 Phase F) — Mode-2 LLM-routed classify.

Read-only delegation to backend `ClassifyMode2` RPC. The actual LLM
call happens server-side via `efe-llm-router` microservice — this tool
exposes the capability through MCP for agent clients.

Default OFF : gated by `EFE_FEATURE_M21_MODE2_ENABLED`.

Pin tests : tests/test_m21_classify_mode2.py.
"""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeClassifyMode2Input",
    "description": (
        "Run a Mode-2 (LLM-routed dynamic anchors) classify. The LLM "
        "call is performed server-side by `efe-llm-router` ; cost is "
        "charged to the tenant's monthly_cap_usd. READ-ONLY. Requires "
        "the `EFE_FEATURE_M21_MODE2_ENABLED` flag."
    ),
    "properties": {
        "sentence": {"type": "string", "minLength": 1, "maxLength": 4096},
        "catalog": {"type": "string", "minLength": 1, "maxLength": 100},
        "version": {
            "type": "string",
            "pattern": "^v\\d+\\.\\d+\\.\\d+$",
        },
        "mode": {
            "type": "string",
            "enum": ["MODE_2", "MODE_AUTO_ESC"],
            "default": "MODE_2",
        },
        "top_k_anchors": {
            "type": "integer",
            "minimum": 3,
            "maximum": 20,
            "default": 10,
        },
        "threshold_override": {
            "type": "number",
            "minimum": 0.0,
            "maximum": 1.0,
            "description": (
                "For MODE_AUTO_ESC : override per-catalog threshold. "
                "Mode-1 is escalated to Mode-2 if confidence < this."
            ),
        },
    },
    "required": ["sentence", "catalog"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not hasattr(ctx.backend, "classify_mode2"):
        raise ToolError(
            "Backend does not support Mode-2 (EFE 1.4 required)",
            code="not_implemented",
        )

    sentence = args["sentence"]
    catalog = args["catalog"]
    version = args.get("version")
    mode = args.get("mode", "MODE_2")
    top_k = int(args.get("top_k_anchors", 10))
    threshold = args.get("threshold_override")

    try:
        result = await ctx.backend.classify_mode2(
            sentence=sentence,
            catalog=catalog,
            version=version,
            mode=mode,
            top_k_anchors=top_k,
            threshold_override=threshold,
            auth=ctx.auth,
        )
    except Exception as e:
        # Backend errors are propagated as ToolError so the dispatcher
        # produces a structured isError response.
        raise ToolError(
            f"Mode-2 classify failed: {e}",
            code="backend_error",
        ) from e

    # Surface the structured result to the LLM client.
    return {
        "label": result.get("label"),
        "confidence": result.get("confidence"),
        "anchors_used": result.get("anchors_used", []),
        "mode": result.get("mode", mode),
        "tokens_in": result.get("tokens_in"),
        "tokens_out": result.get("tokens_out"),
        "cost_usd": result.get("cost_usd"),
        "provider_used": result.get("provider_used"),
        "model_used": result.get("model_used"),
        "cache_hit": result.get("cache_hit", False),
        "duration_ms": result.get("duration_ms"),
        "degraded_reason": result.get("degraded_reason"),
    }


TOOL = Tool(
    name="efe_classify_mode2",
    description=(
        "Run a Mode-2 (LLM-routed dynamic anchors) classify. Cost "
        "charged to tenant budget. READ-ONLY. Requires the "
        "`EFE_FEATURE_M21_MODE2_ENABLED` flag and tenant opt-in."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=False,
    profile_tier="agent-native",
    feature_flag="M21_MODE2_ENABLED",
)
