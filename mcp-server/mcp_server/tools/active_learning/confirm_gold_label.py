"""Tool : ``efe_confirm_gold_label`` — operator confirms an LLM-proposed label."""
from __future__ import annotations

from typing import Any

from ..types import Tool, ToolContext, ToolError
from . import _common


_INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeConfirmGoldLabelInput",
    "description": (
        "Confirm an LLM-proposed gold label. The proposal is inserted into "
        "gold_dataset (M23) and used in the next bundle retune. WRITE — "
        "requires `confirm:true` and a rationale ≥ 10 chars (Art. 14)."
    ),
    "properties": {
        "tenant_id": {"type": "string"},
        "proposal_id": {"type": "string"},
        "rationale": {"type": "string", "minLength": 10, "maxLength": 2048},
        "edit_class_id": {
            "type": "string",
            "description": "If provided, replaces LLM-proposed class (EditAndConfirm).",
        },
        "confirm": {
            "type": "boolean", "const": True,
            "description": "Must be exactly `true`. Required for every write tool.",
        },
    },
    "required": ["tenant_id", "proposal_id", "rationale", "confirm"],
    "additionalProperties": False,
}


async def _handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not args.get("confirm") is True:
        raise ToolError(
            "confirm=true is required for write tool 'efe_confirm_gold_label'",
            code="confirm_required", status_code=400,
        )
    _common.assert_m24_enabled(ctx)
    _common.assert_tenant_match(ctx, args["tenant_id"])
    flags = await _common.fetch_tenant_flags(ctx, args["tenant_id"])
    if not flags.get("m24_active_learning_enabled", False):
        raise ToolError(
            f"M24 not enabled for tenant '{args['tenant_id']}'",
            code="failed_precondition", status_code=412,
        )

    if "edit_class_id" in args and args["edit_class_id"]:
        review_id = await _common.call_grpc(
            ctx, "edit_and_confirm",
            tenant_id=args["tenant_id"],
            proposal_id=args["proposal_id"],
            final_class_id=args["edit_class_id"],
            rationale=args["rationale"],
        )
        return {"review_id": review_id, "decision": "EDIT_AND_CONFIRM"}
    else:
        review_id = await _common.call_grpc(
            ctx, "confirm",
            tenant_id=args["tenant_id"],
            proposal_id=args["proposal_id"],
            rationale=args["rationale"],
        )
        return {"review_id": review_id, "decision": "CONFIRM"}


TOOL: Tool = Tool(
    name="efe_confirm_gold_label",
    description=_INPUT_SCHEMA["description"],
    input_schema=_INPUT_SCHEMA,
    handler=_handler,
    is_write=True,
    requires_sampling=False,
    profile_tier="agent-native",
    feature_flag="EFE_FEATURE_M24_ACTIVE_LEARNING_ENABLED",
)
