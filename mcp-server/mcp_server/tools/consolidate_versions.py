"""efe_consolidate_versions (M17 D45) — write."""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeConsolidateVersionsInput",
    "description": "Drop old versions of a catalog, keeping the last N.",
    "properties": {
        "catalog": {"type": "string", "minLength": 1, "maxLength": 100},
        "keep_last_n": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 10},
        "dry_run": {"type": "boolean", "default": True},
        "confirm": {"type": "boolean"},
    },
    "required": ["catalog"],
    "additionalProperties": False,
    # M18-FINAL-A5 fix : schema-level conditional enforcement so the
    # confirm-required pattern is visible at validation time (not just
    # in the handler). When dry_run=false (the actual destructive path)
    # the request MUST carry confirm:true.
    "if": {
        "properties": {"dry_run": {"const": False}},
        "required": ["dry_run"],
    },
    "then": {
        "required": ["confirm"],
        "properties": {"confirm": {"const": True}},
    },
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    catalog = args["catalog"]
    keep_last_n = int(args.get("keep_last_n", 10))
    dry_run = bool(args.get("dry_run", True))

    if not dry_run and not args.get("confirm"):
        raise ToolError(
            "consolidate_versions with dry_run=false requires confirm:true",
            code="confirm_required",
        )

    return await ctx.backend.consolidate_versions(
        catalog=catalog,
        keep_last_n=keep_last_n,
        dry_run=dry_run,
        auth=ctx.auth,
    )


TOOL = Tool(
    name="efe_consolidate_versions",
    description=(
        "Delete old catalog versions, keeping the last N. dry_run=true "
        "by default returns a deletion plan ; pass dry_run=false + "
        "confirm=true to apply. WRITE."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=True,
    profile_tier="agent-native",
)
