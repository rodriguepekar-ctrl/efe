"""efe_list_catalogs tool — list catalogs visible to the caller (read)."""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeListCatalogsInput",
    "description": "List catalogs visible to the caller's tenant.",
    "properties": {},
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    catalogs = await ctx.backend.list_catalogs(auth=ctx.auth)
    return {
        "count": len(catalogs),
        "catalogs": catalogs,
    }


TOOL = Tool(
    name="efe_list_catalogs",
    description=(
        "List catalogs visible to the caller's tenant (D11 zero-trust isolation). "
        "Returns name, latest_version, fingerprint, class_count and version history."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=False,
)
