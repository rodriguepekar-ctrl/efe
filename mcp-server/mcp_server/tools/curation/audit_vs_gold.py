"""Tool 5 : ``efe_audit_bundle_vs_gold``."""
from __future__ import annotations

from typing import Any

from ..types import Tool, ToolContext, ToolError
from . import _common


_INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeAuditBundleVsGoldInput",
    "description": (
        "Audit a bundle against a gold dataset. Produces a scorecard with "
        "accuracy, precision/recall per class, and detected weak spots. "
        "Structural metrics are always computed locally (no LLM cost) ; "
        "the LLM cost is paid only when ``include_llm_explanation=true``."
    ),
    "properties": {
        "tenant_id": {"type": "string"},
        "catalog_id": {"type": "string"},
        "bundle_id": {"type": "string"},
        "gold_dataset_id": {"type": "string"},
        "max_weak_spots": {"type": "integer", "minimum": 1, "maximum": 50, "default": 10},
        "include_llm_explanation": {"type": "boolean", "default": False},
        "execution_mode": {
            "type": "string",
            "enum": ["SAMPLING_DELEGATION", "INTERNAL_LLM"],
            "default": "SAMPLING_DELEGATION",
        },
        "idempotency_key": {"type": "string"},
    },
    "required": ["tenant_id", "catalog_id", "bundle_id", "gold_dataset_id"],
    "additionalProperties": False,
}


async def _handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    tenant_id = args["tenant_id"]
    _common.assert_tenant_match(ctx, tenant_id)
    max_weak = int(args.get("max_weak_spots", 10))
    if not (1 <= max_weak <= 50):
        raise ToolError("max_weak_spots out of range [1,50]",
                        code="invalid_argument", status_code=400)
    tenant_flags = await _common.fetch_tenant_flags(ctx, tenant_id)
    if not tenant_flags.get("m23_curation_enabled", False):
        raise ToolError(
            f"M23 curation not enabled for tenant '{tenant_id}'",
            code="failed_precondition", status_code=412,
        )

    # Compute structural scorecard via the bench backend (no LLM cost).
    scorecard = await ctx.backend.bench_audit_bundle(
        tenant_id=tenant_id, catalog_id=args["catalog_id"],
        bundle_id=args["bundle_id"], gold_dataset_id=args["gold_dataset_id"],
        max_weak_spots=max_weak,
    )

    if not args.get("include_llm_explanation", False):
        return {
            "scorecard_id": scorecard["scorecard_id"],
            "accuracy": scorecard["accuracy"],
            "macro_f1": scorecard["macro_f1"],
            "weighted_f1": scorecard["weighted_f1"],
            "weak_spots": scorecard.get("weak_spots", []),
            "execution_mode": "STRUCTURAL_ONLY",
            "cost_cents": 0,
        }

    mode = _common.resolve_execution_mode(
        args.get("execution_mode", "SAMPLING_DELEGATION"), tenant_flags,
    )
    rendered = await _common.render_prompt(
        ctx, "curation_audit_vs_gold", 1, tenant_id,
        {
            "tenant_id": tenant_id, "catalog_id": args["catalog_id"],
            "bundle_id": args["bundle_id"], "gold_dataset_id": args["gold_dataset_id"],
            "scorecard_json": scorecard,
            "weak_spots_raw_json": scorecard.get("weak_spots", []),
        },
    )
    if mode == "INTERNAL_LLM":
        llm = await _common.llm_call_via_internal_router(ctx, tenant_id, rendered["body"], max_tokens=2000)
    else:
        llm = await _common.llm_call_via_sampling(ctx, rendered["body"], max_tokens=2000)

    return {
        "scorecard_id": scorecard["scorecard_id"],
        "accuracy": scorecard["accuracy"],
        "macro_f1": scorecard["macro_f1"],
        "weighted_f1": scorecard["weighted_f1"],
        "weak_spots": scorecard.get("weak_spots", []),
        "llm_explanation": llm.content,
        "execution_mode": mode,
        "cost_cents": llm.cost_cents,
        "latency_ms": llm.latency_ms,
    }


TOOL: Tool = Tool(
    name="efe_audit_bundle_vs_gold",
    description=_INPUT_SCHEMA["description"],
    input_schema=_INPUT_SCHEMA,
    handler=_handler,
    is_write=False,  # produces a scorecard row but no taxonomy mutation
    requires_sampling=False,
    profile_tier="agent-native",
    feature_flag="EFE_FEATURE_M23_CURATION_ENABLED",
)
