"""efe_evolve_catalog tool — add examples to an existing catalog (WRITE).

Mirrors `playground/backend/catalog.py::evolve` (M15 audit fix M15-A1).
Requires `confirm: true` (M16 confirm-required pattern).
"""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeEvolveCatalogInput",
    "description": (
        "Add example sentences to existing classes (or create new classes) of "
        "an existing catalog. Centroids are recomputed and a new immutable "
        "version is registered."
    ),
    "properties": {
        "name": {
            "type": "string",
            "minLength": 1,
            "maxLength": 100,
            "description": "Catalog name. Must already exist for the caller's tenant.",
        },
        "additional_examples_per_class": {
            "type": "object",
            "description": (
                "Map class_name → list of new example sentences. New classes "
                "are appended ; existing class examples are merged."
            ),
            "minProperties": 1,
            "maxProperties": 256,
            "additionalProperties": {
                "type": "array",
                "minItems": 1,
                "maxItems": 64,
                "items": {"type": "string", "minLength": 1, "maxLength": 1024},
            },
        },
        "confirm": {
            "type": "boolean",
            "description": (
                "MUST be set to true to actually evolve the catalog. "
                "M16 confirm-required pattern (TM-M16-T-01 mitigation)."
            ),
        },
    },
    "required": ["name", "additional_examples_per_class", "confirm"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not args.get("confirm"):
        raise ToolError(
            "evolve_catalog requires confirm:true ; user must approve the "
            "write in the MCP client.",
            code="confirm_required",
        )

    name = args["name"]
    additional = args["additional_examples_per_class"]
    result = await ctx.backend.evolve_catalog(
        name=name,
        additional_examples_per_class=additional,
        auth=ctx.auth,
    )
    return {
        "evolved": True,
        "name": result.get("name", name),
        "version": result.get("version"),
        "fingerprint": result.get("fingerprint"),
        "class_count": result.get("class_count"),
    }


TOOL = Tool(
    name="efe_evolve_catalog",
    description=(
        "Add example sentences to existing classes (or new classes) of an "
        "existing EFE catalog. Recomputes centroids and registers a new "
        "immutable version. WRITE — requires confirm:true. D12 quotas "
        "enforced server-side."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=True,
)
