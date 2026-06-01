"""efe_register_catalog tool — create a catalog version (WRITE).

Write tools require an explicit `confirm: true` argument. This is the
M16 confirm-required pattern (mirrors STRIDE TM-M16-T-01 mitigation) :
the LLM has to deliberately set `confirm` true, which means the user
saw the tool call card with the argument before approving in their
MCP client. Defends against an LLM "auto-piloting" through writes.
"""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeRegisterCatalogInput",
    "description": (
        "Create a new catalog (or new version of an existing catalog) from "
        "a list of classes + example sentences. Quotas D12 enforced server-side."
    ),
    "properties": {
        "name": {
            "type": "string",
            "minLength": 1,
            "maxLength": 100,
            "pattern": r"^[a-zA-Z0-9][a-zA-Z0-9_\-]{0,99}$",
            "description": "Catalog name. Must be unique per tenant.",
        },
        "classes": {
            "type": "array",
            "minItems": 1,
            "maxItems": 256,
            "description": "Classes that make up the catalog.",
            "items": {
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": 100,
                    },
                    "examples": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 64,
                        "items": {"type": "string", "minLength": 1, "maxLength": 1024},
                    },
                    "confidence_threshold": {
                        "type": "number",
                        "minimum": 0.0,
                        "maximum": 1.0,
                        "default": 0.5,
                    },
                },
                "required": ["name", "examples"],
                "additionalProperties": False,
            },
        },
        "confirm": {
            "type": "boolean",
            "description": (
                "MUST be set to true to actually create the catalog. "
                "M16 confirm-required pattern (TM-M16-T-01 mitigation)."
            ),
        },
    },
    "required": ["name", "classes", "confirm"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not args.get("confirm"):
        raise ToolError(
            "register_catalog requires confirm:true ; user must approve the "
            "write in the MCP client.",
            code="confirm_required",
        )

    name = args["name"]
    classes = args["classes"]
    result = await ctx.backend.register_catalog(
        name=name, classes=classes, auth=ctx.auth
    )
    return {
        "registered": True,
        "name": result.get("name", name),
        "version": result.get("version"),
        "fingerprint": result.get("fingerprint"),
        "class_count": result.get("class_count", len(classes)),
    }


TOOL = Tool(
    name="efe_register_catalog",
    description=(
        "Create a new EFE catalog (or new version of an existing one) from a "
        "list of classes + example sentences. WRITE — requires confirm:true. "
        "D12 quotas enforced server-side : "
        "MAX_NAMES_PER_TENANT=100, MAX_VERSIONS_PER_NAME=1000, "
        "MAX_CLASSES_PER_CATALOG=256, MAX_SENTENCES_PER_CLASS=64."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=True,
)
