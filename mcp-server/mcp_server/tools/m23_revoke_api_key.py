"""efe_revoke_api_key (D8) — revoke an API key (immediate). WRITE+confirm."""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeRevokeApiKeyInput",
    "description": (
        "Revoke an API key effective immediately. Reason ≥ 10 chars "
        "is required for audit traceability. WRITE — requires "
        "`confirm:true`."
    ),
    "properties": {
        "tenant_id": {"type": "string", "pattern": "^[a-z0-9][a-z0-9-]{1,30}[a-z0-9]$"},
        "key_id": {"type": "string", "format": "uuid"},
        "reason": {"type": "string", "minLength": 10, "maxLength": 1024},
        "confirm": {"type": "boolean", "const": True},
    },
    "required": ["tenant_id", "key_id", "reason", "confirm"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not args.get("confirm"):
        raise ToolError("revoke_api_key requires confirm:true", code="confirm_required")
    if not hasattr(ctx.backend, "revoke_api_key"):
        raise ToolError(
            "Backend does not support revoke_api_key (EFE 1.6 required)",
            code="not_implemented",
        )
    try:
        return await ctx.backend.revoke_api_key(
            tenant_id=args["tenant_id"],
            key_id=args["key_id"],
            reason=args["reason"],
            auth=ctx.auth,
        )
    except getattr(ctx.backend, "NotFoundError", Exception) as e:
        raise ToolError(
            f"key_id '{args['key_id']}' not found", code="not_found", status_code=404
        ) from e
    except Exception as e:
        raise ToolError(f"revoke_api_key failed: {e}", code="backend_error") from e


TOOL = Tool(
    name="efe_revoke_api_key",
    description="Revoke an API key (immediate). WRITE — requires `confirm:true`.",
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=True,
    profile_tier="agent-native-deploy",
    feature_flag="D8_ADMIN_ENABLED",
)
