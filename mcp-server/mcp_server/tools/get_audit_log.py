"""efe_get_audit_log tool — read tenant-scoped audit log entries (read)."""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeGetAuditLogInput",
    "description": "Return the most recent N audit log entries for the caller's tenant.",
    "properties": {
        "limit": {
            "type": "integer",
            "minimum": 1,
            "maximum": 200,
            "default": 50,
            "description": "Number of audit entries to return (most recent first).",
        },
    },
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    limit = int(args.get("limit", 50))
    events = await ctx.backend.get_audit_log(limit=limit, auth=ctx.auth)
    return {
        "count": len(events),
        "events": events,
    }


TOOL = Tool(
    name="efe_get_audit_log",
    description=(
        "Return the most recent audit log entries for the caller's tenant "
        "(D14-bis HMAC-chained). Useful to verify what tools have been called "
        "or to investigate an incident. Read-only."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=False,
)
