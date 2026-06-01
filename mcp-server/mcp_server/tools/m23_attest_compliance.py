"""efe_attest_compliance (D8) — attest AI Act Art. 14 + GDPR Art. 22.

WRITE tool, requires `confirm:true`. Inserts a new row in
`tenant_compliance` (versioned). The migration's
`sync_tenant_compliance_to_mode3` AFTER trigger automatically
propagates the AI Act timestamp into `tenant_feature_mode3`, which
unblocks the Mode-3 compliance gate (M22).

Pin tests : tests/test_m23_attest_compliance.py.
"""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeAttestComplianceInput",
    "description": (
        "Attest AI Act Art. 14 (high-risk human oversight) and/or "
        "GDPR Art. 22 (right not to be subject to solely automated "
        "decisions) for a tenant. At least one attestation must be "
        "set. Inserts a new versioned row ; trigger propagates the "
        "AI Act timestamp into tenant_feature_mode3 (unblocks M22 "
        "compliance gate). WRITE — requires `confirm:true`."
    ),
    "properties": {
        "tenant_id": {"type": "string", "pattern": "^[a-z0-9][a-z0-9-]{1,30}[a-z0-9]$"},
        "ai_act_art14": {"type": "boolean", "default": False},
        "gdpr_art22": {"type": "boolean", "default": False},
        "signed_by": {
            "type": "string",
            "format": "email",
            "description": "Email of the natural person attesting.",
        },
        "dpo_contact": {
            "type": ["string", "null"],
            "format": "email",
            "description": "Required when gdpr_art22=true (GDPR Art. 37).",
        },
        "confirm": {"type": "boolean", "const": True},
    },
    "required": ["tenant_id", "signed_by", "confirm"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not args.get("confirm"):
        raise ToolError(
            "attest_compliance requires confirm:true",
            code="confirm_required",
        )
    if not hasattr(ctx.backend, "attest_compliance"):
        raise ToolError(
            "Backend does not support attest_compliance (EFE 1.6 required)",
            code="not_implemented",
        )

    tenant = args["tenant_id"]
    ai_act = bool(args.get("ai_act_art14", False))
    gdpr = bool(args.get("gdpr_art22", False))
    signed_by = args["signed_by"]
    dpo_contact = args.get("dpo_contact")

    if not ai_act and not gdpr:
        raise ToolError(
            "at least one of ai_act_art14 or gdpr_art22 must be true",
            code="invalid_input",
        )
    if gdpr and not dpo_contact:
        raise ToolError(
            "dpo_contact is required when gdpr_art22=true (GDPR Art. 37)",
            code="invalid_input",
        )

    try:
        result = await ctx.backend.attest_compliance(
            tenant_id=tenant,
            ai_act_art14=ai_act,
            gdpr_art22=gdpr,
            signed_by=signed_by,
            dpo_contact=dpo_contact,
            auth=ctx.auth,
        )
    except Exception as e:
        raise ToolError(
            f"attest_compliance failed: {e}",
            code="backend_error",
        ) from e

    return {
        "tenant_id": tenant,
        "ai_act_art14_acknowledged": ai_act,
        "gdpr_art22_acknowledged": gdpr,
        "signed_by": signed_by,
        "dpo_contact": dpo_contact,
        "attested_at": result.get("attested_at"),
    }


TOOL = Tool(
    name="efe_attest_compliance",
    description=(
        "Attest AI Act Art. 14 + GDPR Art. 22 for a tenant. Inserts a "
        "new versioned row. Trigger propagates AI Act timestamp into "
        "tenant_feature_mode3 (unblocks M22 compliance gate). WRITE — "
        "requires `confirm:true`."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=True,
    profile_tier="agent-native-deploy",
    feature_flag="D8_ADMIN_ENABLED",
)
