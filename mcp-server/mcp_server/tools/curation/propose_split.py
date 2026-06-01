"""Tool 1 : ``efe_propose_taxonomy_split``.

Propose splitting an over-broad classification class into 2-3 more
precise sub-classes. The LLM analyzes recent misclassifications and
proposes new class IDs + anchors. Output is a taxonomy_change_proposal
in PENDING status that REQUIRES human review (AI Act Art. 14) before
promotion.

Default execution mode is SAMPLING_DELEGATION (the MCP client samples
its own LLM) ; INTERNAL_LLM requires tenant opt-in (M21 dependency).
"""
from __future__ import annotations

from typing import Any

from ..types import Tool, ToolContext, ToolError
from . import _common


_INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeProposeTaxonomySplitInput",
    "description": (
        "Propose splitting an over-broad class. Output is a PENDING "
        "proposal that requires human review (Art. 14)."
    ),
    "properties": {
        "tenant_id": {"type": "string", "description": "Tenant slug. Must match caller's OIDC claim."},
        "catalog_id": {"type": "string"},
        "class_id": {"type": "string", "description": "Class to split."},
        "max_splits": {"type": "integer", "minimum": 2, "maximum": 5, "default": 3},
        "include_examples": {"type": "boolean", "default": True},
        "sample_size": {"type": "integer", "minimum": 5, "maximum": 100, "default": 20},
        "execution_mode": {
            "type": "string",
            "enum": ["SAMPLING_DELEGATION", "INTERNAL_LLM"],
            "default": "SAMPLING_DELEGATION",
        },
        "idempotency_key": {"type": "string"},
        # M16 confirm-required carry-over. Write tool MUST be invoked
        # with `confirm: true` so an LLM client can't trigger it
        # silently. Audit a5 enforced in
        # `tests/test_audit_pass_m18_final.py`.
        "confirm": {
            "type": "boolean",
            "const": True,
            "description": "Must be exactly `true`. Required for every write tool.",
        },
    },
    "required": ["tenant_id", "catalog_id", "class_id", "confirm"],
    "additionalProperties": False,
}


async def _handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    # M16 confirm-required carry-over. Audit a5 enforces this in
    # `tests/test_audit_pass_m18_final.py`. Defensive runtime check
    # even though the schema requires `confirm: true` — some
    # transports could strip the schema enforcement.
    if not args.get("confirm") is True:
        raise ToolError(
            "confirm=true is required for write tool 'efe_propose_taxonomy_split'",
            code="confirm_required", status_code=400,
        )
    tenant_id = args["tenant_id"]
    catalog_id = args["catalog_id"]
    class_id = args["class_id"]
    max_splits = int(args.get("max_splits", 3))
    sample_size = int(args.get("sample_size", 20))
    include_examples = bool(args.get("include_examples", True))
    requested_mode = args.get("execution_mode", "SAMPLING_DELEGATION")
    idempotency_key = args.get("idempotency_key") or None

    _common.assert_tenant_match(ctx, tenant_id)
    if not (2 <= max_splits <= 5):
        raise ToolError("max_splits out of range [2,5]", code="invalid_argument", status_code=400)
    if not (5 <= sample_size <= 100):
        raise ToolError("sample_size out of range [5,100]", code="invalid_argument", status_code=400)

    tenant_flags = await _common.fetch_tenant_flags(ctx, tenant_id)
    if not tenant_flags.get("m23_curation_enabled", False):
        raise ToolError(
            f"M23 curation not enabled for tenant '{tenant_id}'",
            code="failed_precondition", status_code=412,
        )
    mode = _common.resolve_execution_mode(requested_mode, tenant_flags)

    prompt_context = {
        "tenant_id": tenant_id, "catalog_id": catalog_id, "class_id": class_id,
        "max_splits": max_splits, "sample_size": sample_size,
        "include_examples": include_examples,
    }
    rendered = await _common.render_prompt(
        ctx, "curation_propose_split", 1, tenant_id, prompt_context,
    )

    if mode == "INTERNAL_LLM":
        llm = await _common.llm_call_via_internal_router(
            ctx, tenant_id, rendered["body"], max_tokens=1500,
        )
    else:
        llm = await _common.llm_call_via_sampling(ctx, rendered["body"], max_tokens=1500)

    proposal_id = await _common.insert_proposal(
        ctx, tenant_id=tenant_id, catalog_id=catalog_id, kind="SPLIT",
        llm_output=llm.content, llm_audit_id=llm.audit_id,
        prompt_template_name=rendered["template_name"],
        prompt_template_version=int(rendered["template_version"]),
        execution_mode=mode, idempotency_key=idempotency_key,
        extra={"class_id": class_id, "max_splits": max_splits},
    )

    return {
        "proposal_id": proposal_id,
        "execution_mode": mode,
        "llm_provider": llm.provider,
        "llm_model": llm.model,
        "input_tokens": llm.input_tokens,
        "output_tokens": llm.output_tokens,
        "cost_cents": llm.cost_cents,
        "latency_ms": llm.latency_ms,
        "next_step": (
            f"Review at /{tenant_id}/curation/queue/{proposal_id}"
        ),
    }


TOOL: Tool = Tool(
    name="efe_propose_taxonomy_split",
    description=_INPUT_SCHEMA["description"],
    input_schema=_INPUT_SCHEMA,
    handler=_handler,
    is_write=True,
    requires_sampling=False,  # only required when execution_mode=SAMPLING_DELEGATION
    profile_tier="agent-native",
    feature_flag="EFE_FEATURE_M23_CURATION_ENABLED",
)
