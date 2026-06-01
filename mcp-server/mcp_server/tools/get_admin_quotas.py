"""efe_get_admin_quotas tool — D12 quota usage (read)."""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeGetAdminQuotasInput",
    "description": "Return D12 quota usage for the caller's tenant.",
    "properties": {},
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    quotas = await ctx.backend.get_admin_quotas(auth=ctx.auth)
    return quotas


TOOL = Tool(
    name="efe_get_admin_quotas",
    description=(
        "Return D12 catalog quota usage for the caller's tenant : "
        "MAX_NAMES_PER_TENANT, MAX_VERSIONS_PER_NAME, MAX_CLASSES_PER_CATALOG, "
        "MAX_SENTENCES_PER_CLASS — current vs configured limits. Read-only."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=False,
)
