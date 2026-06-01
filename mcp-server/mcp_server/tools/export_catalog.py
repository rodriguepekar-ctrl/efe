"""efe_export_catalog (M17 D44)."""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeExportCatalogInput",
    "description": "Serialize a catalog (full or single version) to JSON or YAML.",
    "properties": {
        "catalog": {"type": "string", "minLength": 1, "maxLength": 100},
        "version": {"type": "integer", "minimum": 1},
        "format": {"type": "string", "enum": ["json", "yaml"], "default": "json"},
        "include_centroids": {"type": "boolean", "default": False},
        "include_audit_excerpt": {"type": "boolean", "default": False},
    },
    "required": ["catalog"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    return await ctx.backend.export_catalog(
        catalog=args["catalog"],
        version=args.get("version"),
        format=args.get("format", "json"),
        include_centroids=bool(args.get("include_centroids", False)),
        include_audit_excerpt=bool(args.get("include_audit_excerpt", False)),
        auth=ctx.auth,
    )


TOOL = Tool(
    name="efe_export_catalog",
    description=(
        "Serialize a catalog snapshot (full or single version) to JSON "
        "or YAML for backup, diff, or audit. Read-only."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    profile_tier="advanced",
)
