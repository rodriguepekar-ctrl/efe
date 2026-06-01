"""efe_export_audit_log (D8) — bulk audit log export (read-only)."""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeExportAuditLogInput",
    "description": (
        "Export audit log rows since a given date. Read-only. The "
        "`format` parameter selects the on-wire shape : `jsonl` (one "
        "JSON object per row) or `csv` (RFC 4180 with header)."
    ),
    "properties": {
        "tenant_id": {"type": "string", "pattern": "^[a-z0-9][a-z0-9-]{1,30}[a-z0-9]$"},
        "since": {"type": "string", "format": "date-time"},
        "until": {"type": ["string", "null"], "format": "date-time"},
        "format": {"type": "string", "enum": ["jsonl", "csv"], "default": "jsonl"},
        "max_rows": {
            "type": "integer", "minimum": 1, "maximum": 1_000_000, "default": 100_000,
        },
    },
    "required": ["tenant_id", "since"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not hasattr(ctx.backend, "export_audit_log"):
        raise ToolError(
            "Backend does not support export_audit_log (EFE 1.6 required)",
            code="not_implemented",
        )
    try:
        result = await ctx.backend.export_audit_log(
            tenant_id=args["tenant_id"],
            since=args["since"],
            until=args.get("until"),
            format=args.get("format", "jsonl"),
            max_rows=int(args.get("max_rows", 100_000)),
            auth=ctx.auth,
        )
    except Exception as e:
        raise ToolError(f"export_audit_log failed: {e}", code="backend_error") from e
    return {
        "rows": result.get("rows", []),
        "row_count": len(result.get("rows", [])),
        "truncated": result.get("truncated", False),
    }


TOOL = Tool(
    name="efe_export_audit_log",
    description="Bulk audit log export (read-only).",
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=False,
    profile_tier="agent-native",
    feature_flag="D8_ADMIN_ENABLED",
)
