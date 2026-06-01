"""efe_rotate_api_key (D8) — rotate an existing API key.

WRITE tool, requires `confirm:true`. Issues a replacement key with the
same scope + (current TTL or default 30d) ; the old key is marked
`pending_revoke` and revoked after a 24-h grace period (cron job).

Returns the NEW plaintext key ONCE.

Pin tests : tests/test_m23_rotate_api_key.py.
"""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeRotateApiKeyInput",
    "description": (
        "Rotate an existing API key. Issues a replacement with same "
        "scope and (current TTL or 30d default). Old key invalidated "
        "after 24-h grace period. Returns NEW plaintext ONCE. WRITE — "
        "requires `confirm:true`."
    ),
    "properties": {
        "tenant_id": {"type": "string", "pattern": "^[a-z0-9][a-z0-9-]{1,30}[a-z0-9]$"},
        "key_id": {"type": "string", "format": "uuid"},
        "confirm": {"type": "boolean", "const": True},
    },
    "required": ["tenant_id", "key_id", "confirm"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not args.get("confirm"):
        raise ToolError(
            "rotate_api_key requires confirm:true",
            code="confirm_required",
        )
    if not hasattr(ctx.backend, "rotate_api_key"):
        raise ToolError(
            "Backend does not support rotate_api_key (EFE 1.6 required)",
            code="not_implemented",
        )

    tenant = args["tenant_id"]
    key_id = args["key_id"]

    try:
        result = await ctx.backend.rotate_api_key(
            tenant_id=tenant,
            key_id=key_id,
            auth=ctx.auth,
        )
    except getattr(ctx.backend, "NotFoundError", Exception) as e:
        raise ToolError(
            f"key_id '{key_id}' not found for tenant '{tenant}'",
            code="not_found",
            status_code=404,
        ) from e
    except Exception as e:
        raise ToolError(
            f"rotate_api_key failed: {e}",
            code="backend_error",
        ) from e

    return {
        "old_key_id": key_id,
        "new_key_id": result.get("new_key_id"),
        # Plaintext returned ONCE.
        "new_plaintext": result.get("new_plaintext"),
        "scope": result.get("scope"),
        "expires_at": result.get("expires_at"),
        "old_key_invalidated_at": result.get("old_key_invalidated_at"),
    }


TOOL = Tool(
    name="efe_rotate_api_key",
    description=(
        "Rotate an existing API key with 24-h grace period. Returns "
        "NEW plaintext ONCE. WRITE — requires `confirm:true`."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=True,
    profile_tier="agent-native-deploy",
    feature_flag="D8_ADMIN_ENABLED",
)
