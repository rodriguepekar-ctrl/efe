"""Tool 4 : ``efe_suggest_class_merge``."""
from __future__ import annotations

from typing import Any

from ..types import Tool, ToolContext, ToolError
from . import _common


_INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeSuggestClassMergeInput",
    "description": (
        "Suggest merging 2-3 classes that are too similar (high cosine "
        "similarity OR high co-confusion). Output is a PENDING proposal."
    ),
    "properties": {
        "tenant_id": {"type": "string"},
        "catalog_id": {"type": "string"},
        "class_ids": {
            "type": "array",
            "items": {"type": "string"},
            "minItems": 2, "maxItems": 3, "uniqueItems": True,
        },
        "execution_mode": {
            "type": "string",
            "enum": ["SAMPLING_DELEGATION", "INTERNAL_LLM"],
            "default": "SAMPLING_DELEGATION",
        },
        "idempotency_key": {"type": "string"},
        "confirm": {
            "type": "boolean", "const": True,
            "description": "Must be exactly `true`. Required for every write tool.",
        },
    },
    "required": ["tenant_id", "catalog_id", "class_ids", "confirm"],
    "additionalProperties": False,
}


async def _handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not args.get("confirm") is True:
        raise ToolError(
            "confirm=true is required for write tool 'efe_suggest_class_merge'",
            code="confirm_required", status_code=400,
        )
    tenant_id = args["tenant_id"]
    _common.assert_tenant_match(ctx, tenant_id)
    class_ids: list[str] = list(args["class_ids"])
    if not (2 <= len(class_ids) <= 3):
        raise ToolError("class_ids must have 2 or 3 entries",
                        code="invalid_argument", status_code=400)
    if len(set(class_ids)) != len(class_ids):
        raise ToolError("duplicate class_ids", code="invalid_argument", status_code=400)

    tenant_flags = await _common.fetch_tenant_flags(ctx, tenant_id)
    if not tenant_flags.get("m23_curation_enabled", False):
        raise ToolError(
            f"M23 curation not enabled for tenant '{tenant_id}'",
            code="failed_precondition", status_code=412,
        )
    mode = _common.resolve_execution_mode(
        args.get("execution_mode", "SAMPLING_DELEGATION"), tenant_flags,
    )

    rendered = await _common.render_prompt(
        ctx, "curation_suggest_merge", 1, tenant_id,
        {"tenant_id": tenant_id, "catalog_id": args["catalog_id"], "class_ids": class_ids},
    )
    if mode == "INTERNAL_LLM":
        llm = await _common.llm_call_via_internal_router(ctx, tenant_id, rendered["body"], max_tokens=1200)
    else:
        llm = await _common.llm_call_via_sampling(ctx, rendered["body"], max_tokens=1200)

    proposal_id = await _common.insert_proposal(
        ctx, tenant_id=tenant_id, catalog_id=args["catalog_id"], kind="MERGE",
        llm_output=llm.content, llm_audit_id=llm.audit_id,
        prompt_template_name=rendered["template_name"],
        prompt_template_version=int(rendered["template_version"]),
        execution_mode=mode, idempotency_key=args.get("idempotency_key") or None,
        extra={"class_ids": class_ids},
    )
    return {
        "proposal_id": proposal_id, "execution_mode": mode,
        "cost_cents": llm.cost_cents, "merging_classes": class_ids,
    }


TOOL: Tool = Tool(
    name="efe_suggest_class_merge",
    description=_INPUT_SCHEMA["description"],
    input_schema=_INPUT_SCHEMA,
    handler=_handler,
    is_write=True,
    requires_sampling=False,
    profile_tier="agent-native",
    feature_flag="EFE_FEATURE_M23_CURATION_ENABLED",
)
