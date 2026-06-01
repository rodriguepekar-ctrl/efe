"""efe_get_concept_stack (M22 Phase E) — right-to-explanation API.

Retrieve the full concept_stack (Mode-3 reasoning trace) for a
specific audit_id. Supports DPO export with PII unredaction (admin
only).

Default OFF : gated by `EFE_FEATURE_M22_MODE3_ENABLED`.

Pin tests : tests/test_m22_get_concept_stack.py.
"""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeGetConceptStackInput",
    "description": (
        "Retrieve the full concept_stack reasoning trace for a "
        "specific Mode-3 audit_id. Supports DPO export procedure for "
        "GDPR Art. 22(3) right-to-explanation. PII unredaction is "
        "admin-only (`include_pii_unredacted=true` requires "
        "agent-native-deploy profile)."
    ),
    "properties": {
        "audit_id": {
            "type": "integer",
            "minimum": 1,
        },
        "include_pii_unredacted": {
            "type": "boolean",
            "default": False,
            "description": (
                "Admin only ; for DPO export. Requires "
                "agent-native-deploy profile."
            ),
        },
    },
    "required": ["audit_id"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not hasattr(ctx.backend, "get_concept_stack"):
        raise ToolError(
            "Backend does not support concept_stack retrieval (EFE 1.5 required)",
            code="not_implemented",
        )

    audit_id = int(args["audit_id"])
    include_pii_unredacted = bool(args.get("include_pii_unredacted", False))

    # Privilege check : PII unredaction only in admin profile.
    if include_pii_unredacted:
        # ToolContext.profile is set at dispatch time ; we check
        # via getattr with safe default.
        profile = getattr(ctx, "profile", "agent-native")
        if profile not in ("agent-native-deploy", "experimental"):
            raise ToolError(
                "include_pii_unredacted=true requires agent-native-deploy profile",
                code="forbidden",
            )

    try:
        result = await ctx.backend.get_concept_stack(
            audit_id=audit_id,
            include_pii_unredacted=include_pii_unredacted,
            auth=ctx.auth,
        )
    except Exception as e:
        raise ToolError(
            f"get_concept_stack failed: {e}",
            code="backend_error",
        ) from e

    return {
        "audit_id": audit_id,
        "tenant_id_hash": result.get("tenant_id_hash"),
        "catalog": result.get("catalog"),
        "bundle_version": result.get("bundle_version"),
        "mode": result.get("mode"),
        "total_turns": result.get("total_turns"),
        "total_cost_usd": result.get("total_cost_usd"),
        "converged": result.get("converged"),
        "final_label": result.get("final_label"),
        "final_confidence": result.get("final_confidence"),
        "concept_stack": result.get("concept_stack", []),
        "pii_unredacted": include_pii_unredacted,
        "created_at": result.get("created_at"),
    }


TOOL = Tool(
    name="efe_get_concept_stack",
    description=(
        "Retrieve full concept_stack reasoning trace for a Mode-3 "
        "audit_id (GDPR Art. 22(3) right-to-explanation API). PII "
        "unredaction is admin-only. READ-ONLY. Requires "
        "`EFE_FEATURE_M22_MODE3_ENABLED` flag."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=False,
    profile_tier="agent-native",
    feature_flag="M22_MODE3_ENABLED",
)
