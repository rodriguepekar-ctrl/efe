"""Tool : ``efe_request_gdpr_erasure`` — submit Art. 17 erasure."""
from __future__ import annotations

from typing import Any

from ..types import Tool, ToolContext, ToolError
from . import _common


_INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeRequestGdprErasureInput",
    "description": (
        "Submit a GDPR Art. 17 (right to erasure) request. Tombstones the "
        "matching gold_example rows + creates a bundle_rebuild_request "
        "for cascade. Tenant admin role required. Affects bundle accuracy "
        "until next retune."
    ),
    "properties": {
        "tenant_id": {"type": "string"},
        "catalog_id": {"type": "string"},
        "urgency": {
            "type": "string",
            "enum": ["STANDARD", "REGULATORY", "CATASTROPHIC"],
            "default": "STANDARD",
        },
        "content_hashes": {
            "type": "array",
            "items": {"type": "string", "pattern": "^[a-f0-9]{64}$"},
            "minItems": 1, "maxItems": 1000,
            "description": "SHA-256 hex hashes of sentences to erase.",
        },
        "requester_subject_id": {
            "type": "string",
            "description": "PII-safe subject identifier (anonymized handle).",
        },
        "reason": {
            "type": "string", "minLength": 10, "maxLength": 2048,
            "description": "Reason for erasure (audit trail).",
        },
        "confirm": {
            "type": "boolean", "const": True,
            "description": "Must be exactly `true`. Required for every write tool.",
        },
    },
    "required": ["tenant_id", "catalog_id", "content_hashes", "reason", "confirm"],
    "additionalProperties": False,
}


async def _handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not args.get("confirm") is True:
        raise ToolError(
            "confirm=true is required for write tool 'efe_request_gdpr_erasure'",
            code="confirm_required", status_code=400,
        )
    _common.assert_m24_enabled(ctx)
    _common.assert_tenant_match(ctx, args["tenant_id"])

    # tenant_admin role required (server enforces ; this is early hint)
    role = getattr(ctx.auth, "role", "viewer") if ctx.auth else "viewer"
    if role != "tenant_admin":
        raise ToolError(
            "GDPR Art. 17 erasure requires tenant_admin role",
            code="permission_denied", status_code=403,
        )

    resp = await _common.call_grpc(
        ctx, "submit_gdpr_erasure",
        tenant_id=args["tenant_id"],
        catalog_id=args["catalog_id"],
        urgency=args.get("urgency", "STANDARD"),
        content_hashes=args["content_hashes"],
        requester_subject_id=args.get("requester_subject_id"),
        reason=args["reason"],
    )
    return {
        "rebuild_request_id": resp["rebuild_request_id"],
        "sla_deadline": resp["sla_deadline"],
        "affected_count": resp["affected_count"],
    }


TOOL: Tool = Tool(
    name="efe_request_gdpr_erasure",
    description=_INPUT_SCHEMA["description"],
    input_schema=_INPUT_SCHEMA,
    handler=_handler,
    is_write=True,
    requires_sampling=False,
    profile_tier="agent-native",
    feature_flag="EFE_FEATURE_M24_ACTIVE_LEARNING_ENABLED",
)
