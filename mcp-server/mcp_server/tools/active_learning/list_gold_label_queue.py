"""Tool : ``efe_list_gold_label_queue`` — read pending labels."""
from __future__ import annotations

from typing import Any

from ..types import Tool, ToolContext, ToolError
from . import _common


_INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeListGoldLabelQueueInput",
    "description": "List items in the gold label queue (read-only).",
    "properties": {
        "tenant_id": {"type": "string"},
        "catalog_id": {"type": "string"},
        "status_filter": {
            "type": "string",
            "enum": [
                "ENQUEUED", "LABELING", "LLM_LABELED",
                "AWAITING_REVIEW", "FAILED",
            ],
        },
        "limit": {"type": "integer", "minimum": 1, "maximum": 500, "default": 50},
    },
    "required": ["tenant_id"],
    "additionalProperties": False,
}


async def _handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    _common.assert_m24_enabled(ctx)
    _common.assert_tenant_match(ctx, args["tenant_id"])
    flags = await _common.fetch_tenant_flags(ctx, args["tenant_id"])
    if not flags.get("m24_active_learning_enabled", False):
        raise ToolError(
            f"M24 not enabled for tenant '{args['tenant_id']}'",
            code="failed_precondition", status_code=412,
        )
    items = await _common.call_grpc(
        ctx, "list_queue",
        tenant_id=args["tenant_id"],
        catalog_id=args.get("catalog_id"),
        status_filter=args.get("status_filter"),
        limit=int(args.get("limit", 50)),
    )
    return {"items": items, "count": len(items)}


TOOL: Tool = Tool(
    name="efe_list_gold_label_queue",
    description=_INPUT_SCHEMA["description"],
    input_schema=_INPUT_SCHEMA,
    handler=_handler,
    is_write=False,
    requires_sampling=False,
    profile_tier="agent-native",
    feature_flag="EFE_FEATURE_M24_ACTIVE_LEARNING_ENABLED",
)
