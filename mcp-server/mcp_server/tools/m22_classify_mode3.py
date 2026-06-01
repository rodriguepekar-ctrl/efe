"""efe_classify_mode3 (M22 Phase E) — Mode-3 multi-turn classify.

Read-only delegation to backend `ClassifyMode3` RPC. The actual
multi-turn loop happens server-side. Returns final_label +
concept_stack + total_turns + total_cost_usd + converged +
requires_human_review + review_token (if applicable).

Default OFF : gated by `EFE_FEATURE_M22_MODE3_ENABLED`. Requires M21
also active.

Pin tests : tests/test_m22_classify_mode3.py.
"""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeClassifyMode3Input",
    "description": (
        "Run a Mode-3 (multi-turn iterative refinement) classify. "
        "The backend orchestrates N turns (default 3, max 5) of "
        "Mode-2 + swarm consensus. Returns full concept_stack for "
        "GDPR Art. 22 right-to-explanation. Cost is charged to "
        "tenant.monthly_cap_usd as N independent Mode-2 calls. "
        "READ-ONLY. Requires `EFE_FEATURE_M22_MODE3_ENABLED` and "
        "M21 active."
    ),
    "properties": {
        "sentence": {"type": "string", "minLength": 1, "maxLength": 4096},
        "catalog": {"type": "string", "minLength": 1, "maxLength": 100},
        "version": {
            "type": "string",
            "pattern": "^v\\d+\\.\\d+\\.\\d+$",
        },
        "max_turns": {
            "type": "integer",
            "minimum": 1,
            "maximum": 5,
            "description": (
                "Per-request override of tenant default. Hard cap 5."
            ),
        },
        "threshold": {
            "type": "number",
            "minimum": 0.0,
            "maximum": 1.0,
            "description": (
                "Convergence threshold. swarm_confidence >= threshold "
                "stops the loop."
            ),
        },
        "top_k_anchors": {
            "type": "integer",
            "minimum": 3,
            "maximum": 20,
            "default": 10,
        },
    },
    "required": ["sentence", "catalog"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not hasattr(ctx.backend, "classify_mode3"):
        raise ToolError(
            "Backend does not support Mode-3 (EFE 1.5 required)",
            code="not_implemented",
        )

    sentence = args["sentence"]
    catalog = args["catalog"]
    version = args.get("version")
    max_turns = args.get("max_turns")
    threshold = args.get("threshold")
    top_k = int(args.get("top_k_anchors", 10))

    try:
        result = await ctx.backend.classify_mode3(
            sentence=sentence,
            catalog=catalog,
            version=version,
            max_turns=max_turns,
            threshold=threshold,
            top_k_anchors=top_k,
            auth=ctx.auth,
        )
    except Exception as e:
        raise ToolError(
            f"Mode-3 classify failed: {e}",
            code="backend_error",
        ) from e

    return {
        "final_label": result.get("final_label"),
        "final_confidence": result.get("final_confidence"),
        "mode": result.get("mode", "MODE_3"),
        "total_turns": result.get("total_turns"),
        "total_cost_usd": result.get("total_cost_usd"),
        "converged": result.get("converged"),
        "requires_human_review": result.get("requires_human_review", False),
        "review_token": result.get("review_token"),
        "audit_id": result.get("audit_id"),
        "concept_stack": result.get("concept_stack", []),
        "degraded_reason": result.get("degraded_reason"),
    }


TOOL = Tool(
    name="efe_classify_mode3",
    description=(
        "Run a Mode-3 (multi-turn iterative refinement) classify. "
        "Multi-turn LLM-routed loop with convergence check + "
        "concept_stack for GDPR Art. 22 right-to-explanation. "
        "READ-ONLY. Requires `EFE_FEATURE_M22_MODE3_ENABLED` flag "
        "and M21 active."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=False,
    profile_tier="agent-native",
    feature_flag="M22_MODE3_ENABLED",
)
