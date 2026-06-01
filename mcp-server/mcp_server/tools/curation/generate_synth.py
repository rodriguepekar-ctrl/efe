"""Tool 3 : ``efe_generate_synthetic_examples``."""
from __future__ import annotations

from typing import Any

from ..types import Tool, ToolContext, ToolError
from . import _common


_INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeGenerateSyntheticExamplesInput",
    "description": (
        "Generate N synthetic training examples for a class. Useful to "
        "augment training sets or extend gold datasets. Examples are "
        "PII-validated post-generation. Insertion into a gold dataset "
        "requires separate human review approval."
    ),
    "properties": {
        "tenant_id": {"type": "string"},
        "catalog_id": {"type": "string"},
        "class_id": {"type": "string"},
        "n_examples": {"type": "integer", "minimum": 5, "maximum": 100, "default": 20},
        "diversity_hint": {
            "type": "string", "enum": ["HIGH", "MEDIUM", "LOW"], "default": "MEDIUM",
        },
        "include_negative": {"type": "boolean", "default": False},
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
    "required": ["tenant_id", "catalog_id", "class_id", "confirm"],
    "additionalProperties": False,
}


async def _handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not args.get("confirm") is True:
        raise ToolError(
            "confirm=true is required for write tool 'efe_generate_synthetic_examples'",
            code="confirm_required", status_code=400,
        )
    tenant_id = args["tenant_id"]
    _common.assert_tenant_match(ctx, tenant_id)
    n = int(args.get("n_examples", 20))
    if not (5 <= n <= 100):
        raise ToolError("n_examples out of range [5,100]", code="invalid_argument", status_code=400)
    diversity = args.get("diversity_hint", "MEDIUM")
    if diversity not in ("HIGH", "MEDIUM", "LOW"):
        raise ToolError("diversity_hint must be HIGH | MEDIUM | LOW",
                        code="invalid_argument", status_code=400)

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
        ctx, "curation_generate_synth", 1, tenant_id,
        {
            "tenant_id": tenant_id, "catalog_id": args["catalog_id"],
            "class_id": args["class_id"],
            "n_examples": n, "diversity_hint": diversity,
            "include_negative": bool(args.get("include_negative", False)),
        },
    )
    max_tokens = max(2000, n * 100)
    if mode == "INTERNAL_LLM":
        llm = await _common.llm_call_via_internal_router(ctx, tenant_id, rendered["body"], max_tokens=max_tokens)
    else:
        llm = await _common.llm_call_via_sampling(ctx, rendered["body"], max_tokens=max_tokens)

    proposal_id = await _common.insert_proposal(
        ctx, tenant_id=tenant_id, catalog_id=args["catalog_id"], kind="GENERATE_SYNTH",
        llm_output=llm.content, llm_audit_id=llm.audit_id,
        prompt_template_name=rendered["template_name"],
        prompt_template_version=int(rendered["template_version"]),
        execution_mode=mode, idempotency_key=args.get("idempotency_key") or None,
        extra={"class_id": args["class_id"], "n_examples": n, "diversity_hint": diversity},
    )
    return {
        "proposal_id": proposal_id, "execution_mode": mode,
        "cost_cents": llm.cost_cents, "n_examples_requested": n,
    }


TOOL: Tool = Tool(
    name="efe_generate_synthetic_examples",
    description=_INPUT_SCHEMA["description"],
    input_schema=_INPUT_SCHEMA,
    handler=_handler,
    is_write=True,
    requires_sampling=False,
    profile_tier="agent-native",
    feature_flag="EFE_FEATURE_M23_CURATION_ENABLED",
)
