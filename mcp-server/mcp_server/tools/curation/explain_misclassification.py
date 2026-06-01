"""Tool 2 : ``efe_explain_misclassification``."""
from __future__ import annotations

from typing import Any

from ..types import Tool, ToolContext, ToolError
from . import _common


_INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeExplainMisclassificationInput",
    "description": (
        "Explain why a sentence was classified incorrectly and propose 1-3 "
        "actionable fixes (add anchor, remove ambiguous anchor, split class, "
        "or generate examples)."
    ),
    "properties": {
        "tenant_id": {"type": "string"},
        "catalog_id": {"type": "string"},
        "sentence_id": {"type": "string", "description": "Reference to a row in llm_call_audit."},
        "actual_class_id": {"type": "string"},
        "expected_class_id": {"type": "string", "description": "Optional ; LLM suggests if absent."},
        "execution_mode": {
            "type": "string",
            "enum": ["SAMPLING_DELEGATION", "INTERNAL_LLM"],
            "default": "SAMPLING_DELEGATION",
        },
        "idempotency_key": {"type": "string"},
        # M16 confirm-required carry-over. Audit a5 enforced.
        "confirm": {
            "type": "boolean", "const": True,
            "description": "Must be exactly `true`. Required for every write tool.",
        },
    },
    "required": ["tenant_id", "catalog_id", "sentence_id", "actual_class_id", "confirm"],
    "additionalProperties": False,
}


async def _handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not args.get("confirm") is True:
        raise ToolError(
            "confirm=true is required for write tool 'efe_explain_misclassification'",
            code="confirm_required", status_code=400,
        )
    tenant_id = args["tenant_id"]
    _common.assert_tenant_match(ctx, tenant_id)
    tenant_flags = await _common.fetch_tenant_flags(ctx, tenant_id)
    if not tenant_flags.get("m23_curation_enabled", False):
        raise ToolError(
            f"M23 curation not enabled for tenant '{tenant_id}'",
            code="failed_precondition", status_code=412,
        )
    mode = _common.resolve_execution_mode(
        args.get("execution_mode", "SAMPLING_DELEGATION"), tenant_flags,
    )

    prompt_context = {
        "tenant_id": tenant_id,
        "catalog_id": args["catalog_id"],
        "sentence_id": args["sentence_id"],
        "actual_class_id": args["actual_class_id"],
        "expected_class_id_or_unknown":
            args.get("expected_class_id") or "(unknown — LLM to suggest)",
    }
    rendered = await _common.render_prompt(
        ctx, "curation_explain_miss", 1, tenant_id, prompt_context,
    )

    if mode == "INTERNAL_LLM":
        llm = await _common.llm_call_via_internal_router(ctx, tenant_id, rendered["body"], max_tokens=600)
    else:
        llm = await _common.llm_call_via_sampling(ctx, rendered["body"], max_tokens=600)

    proposal_id = await _common.insert_proposal(
        ctx, tenant_id=tenant_id, catalog_id=args["catalog_id"], kind="EXPLAIN",
        llm_output=llm.content, llm_audit_id=llm.audit_id,
        prompt_template_name=rendered["template_name"],
        prompt_template_version=int(rendered["template_version"]),
        execution_mode=mode, idempotency_key=args.get("idempotency_key") or None,
        extra={"sentence_id": args["sentence_id"], "actual_class_id": args["actual_class_id"]},
    )
    return {
        "proposal_id": proposal_id,
        "execution_mode": mode,
        "cost_cents": llm.cost_cents,
        "latency_ms": llm.latency_ms,
        "raw_llm_output": llm.content,
    }


TOOL: Tool = Tool(
    name="efe_explain_misclassification",
    description=_INPUT_SCHEMA["description"],
    input_schema=_INPUT_SCHEMA,
    handler=_handler,
    is_write=True,
    requires_sampling=False,
    profile_tier="agent-native",
    feature_flag="EFE_FEATURE_M23_CURATION_ENABLED",
)
