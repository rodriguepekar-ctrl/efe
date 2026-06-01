"""efe_audit_health_check (M17 D44)."""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeAuditHealthCheckInput",
    "description": "Verify D14-bis HMAC chain integrity for the caller's tenant.",
    "properties": {
        "from_timestamp": {"type": "number"},
        "to_timestamp": {"type": "number"},
        "deep": {"type": "boolean", "default": False},
    },
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    return await ctx.backend.audit_health_check(
        from_timestamp=args.get("from_timestamp"),
        to_timestamp=args.get("to_timestamp"),
        deep=bool(args.get("deep", False)),
        auth=ctx.auth,
    )


TOOL = Tool(
    name="efe_audit_health_check",
    description=(
        "Verify the D14-bis HMAC chain integrity for the caller's tenant. "
        "`deep:true` re-verifies the entire chain ; default is incremental. "
        "Returns ok/broken_at/events_verified. Read-only."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    profile_tier="advanced",
)
