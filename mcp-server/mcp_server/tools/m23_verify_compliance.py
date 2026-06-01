"""efe_verify_compliance (D8) — read-only compliance status."""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeVerifyComplianceInput",
    "description": (
        "Verify the current compliance state for a tenant. Returns "
        "AI Act Art. 14 + GDPR Art. 22 attestation status, signer, "
        "expiration."
    ),
    "properties": {
        "tenant_id": {"type": "string", "pattern": "^[a-z0-9][a-z0-9-]{1,30}[a-z0-9]$"},
    },
    "required": ["tenant_id"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not hasattr(ctx.backend, "verify_compliance"):
        raise ToolError(
            "Backend does not support verify_compliance (EFE 1.6 required)",
            code="not_implemented",
        )
    tenant = args["tenant_id"]
    try:
        c = await ctx.backend.verify_compliance(tenant_id=tenant, auth=ctx.auth)
    except getattr(ctx.backend, "NotFoundError", Exception) as e:
        raise ToolError(
            f"tenant '{tenant}' not found", code="not_found", status_code=404
        ) from e
    except Exception as e:
        raise ToolError(f"verify_compliance failed: {e}", code="backend_error") from e
    return {
        "tenant_id": tenant,
        "ai_act_art14": c.get("ai_act_art14", {
            "acknowledged": False, "acknowledged_at": None,
            "signed_by": None, "expires_in_days": None,
        }),
        "gdpr_art22": c.get("gdpr_art22", {
            "acknowledged": False, "acknowledged_at": None,
            "signed_by": None, "expires_in_days": None,
        }),
        "soc2_in_scope": c.get("soc2_in_scope", False),
    }


TOOL = Tool(
    name="efe_verify_compliance",
    description="Read-only compliance status (AI Act Art. 14 + GDPR Art. 22).",
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=False,
    profile_tier="agent-native",
    feature_flag="D8_ADMIN_ENABLED",
)
