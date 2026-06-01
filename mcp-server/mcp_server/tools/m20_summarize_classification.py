"""efe_summarize_classification (M20 D55) — LLM-helper, sampling-delegated.

Given a Mode-1 classification result (label, confidence, anchors_used),
ask the LLM (via MCP sampling) to produce a structured human-readable
explanation : 20-1000 char summary + 1-5 key factors + confidence
self-assessment + 0-3 alternative classes.

Differs from M17 `efe_explain_classification` (backend deterministic)
by being LLM-driven and prose-focused. Both can co-exist : the M17
tool gives algorithmic contributors, the M20 tool gives narrative.

Default OFF : gated by `EFE_FEATURE_M20_HELPER_TOOLS_ENABLED`.

Pin tests : tests/test_m20_summarize_classification.py.
"""
from __future__ import annotations

import json
from typing import Any

from .m20_prompts import render
from .m20_redaction import redact_for_prompt
from .m20_validation import validate_with_retry
from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeSummarizeClassificationInput",
    "description": (
        "LLM-helper : produce a structured natural-language summary of a "
        "Mode-1 classification (label + confidence + anchors). Uses MCP "
        "sampling delegation."
    ),
    "properties": {
        "catalog": {"type": "string", "minLength": 1, "maxLength": 100},
        "version": {
            "type": "string",
            "pattern": "^v\\d+\\.\\d+\\.\\d+$",
        },
        "sentence": {"type": "string", "minLength": 1, "maxLength": 4096},
        "classification_result": {
            "type": "object",
            "properties": {
                "label": {"type": "string", "minLength": 1, "maxLength": 100},
                "confidence": {"type": "number", "minimum": 0, "maximum": 1},
                "anchors_used": {
                    "type": "array",
                    "items": {"type": "string", "maxLength": 200},
                    "maxItems": 10,
                },
            },
            "required": ["label", "confidence"],
            "additionalProperties": False,
        },
        "language_hint": {
            "type": "string",
            "enum": ["en", "fr", "de", "es", "pt", "it"],
            "default": "en",
        },
    },
    "required": ["catalog", "sentence", "classification_result"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not ctx.has_sampling():
        raise ToolError(
            "efe_summarize_classification requires MCP sampling capability ; "
            "the connected client did not declare it.",
            code="sampling_unavailable",
        )

    catalog = args["catalog"]
    sentence_raw = args["sentence"]
    cls_result = args["classification_result"]
    version = args.get("version")
    language = args.get("language_hint", "en")

    # 1. Redact PII before prompt inclusion
    sentence = redact_for_prompt(sentence_raw)

    # 2. Try to fetch class definitions for richer context (best-effort)
    class_definitions: dict[str, Any] = {}
    if version:
        try:
            class_definitions = await _fetch_class_definitions(
                ctx, catalog, version,
            )
        except Exception:  # noqa: BLE001 - non-fatal
            class_definitions = {}

    # 3. Render prompt template
    system_prompt, user_prompt = render(
        "summarize_classification_v1",
        catalog=catalog,
        sentence=sentence,
        classification_result_json=json.dumps(cls_result, ensure_ascii=False),
        class_definitions_json=json.dumps(class_definitions, ensure_ascii=False),
        language=_language_name(language),
    )

    combined_prompt = f"{system_prompt}\n\n{user_prompt}"

    # 4. Issue sampling request via MCP, with retry on validation failure
    async def _retry_callback(errors: list[str]) -> str:
        retry_prompt = (
            f"{combined_prompt}\n\n"
            f"Validation errors : {'; '.join(errors)}. "
            f"Produce a corrected JSON output that strictly matches the schema."
        )
        return await ctx.sampling.sample(
            tenant_id=ctx.tenant_id,
            prompt=retry_prompt,
            max_tokens=600,
        )

    raw_text = await ctx.sampling.sample(
        tenant_id=ctx.tenant_id,
        prompt=combined_prompt,
        max_tokens=600,
    )

    output = await validate_with_retry(
        helper="summarize_classification",
        raw_output=raw_text,
        retry_callback=_retry_callback,
        max_retries=2,
    )

    return {
        "explanation": output["explanation"],
        "key_factors": output["key_factors"],
        "confidence_assessment": output["confidence_assessment"],
        "suggested_alternatives": output["suggested_alternatives"],
        "metadata": {
            "language": language,
            "prompt_template": "summarize_classification_v1",
        },
    }


async def _fetch_class_definitions(
    ctx: ToolContext, catalog: str, version: str,
) -> dict[str, Any]:
    """Fetch class definitions for the given catalog/version.

    Best-effort : returns `{}` on any backend error (the helper still
    works without class definitions, with degraded explanations).
    """
    if not hasattr(ctx.backend, "get_class_definitions"):
        return {}
    try:
        return await ctx.backend.get_class_definitions(
            catalog=catalog, version=version, auth=ctx.auth,
        )
    except Exception:  # noqa: BLE001
        return {}


def _language_name(code: str) -> str:
    return {
        "en": "English",
        "fr": "français",
        "de": "Deutsch",
        "es": "español",
        "pt": "português",
        "it": "italiano",
    }.get(code, "English")


TOOL = Tool(
    name="efe_summarize_classification",
    description=(
        "LLM-helper : produce a structured natural-language summary of a "
        "Mode-1 classification result. Uses MCP sampling delegation — the "
        "LLM call is performed by the connected client. READ-ONLY. "
        "Requires the `EFE_FEATURE_M20_HELPER_TOOLS_ENABLED` flag."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=False,
    requires_sampling=True,
    profile_tier="agent-native",
    feature_flag="M20_HELPER_TOOLS_ENABLED",
)
