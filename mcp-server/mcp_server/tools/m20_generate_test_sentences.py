"""efe_generate_test_sentences (M20 D56) — LLM-helper, sampling-delegated.

Generate N synthetic test sentences for a target class to augment
gold-dataset coverage. Anti-redundancy with existing sentences.

Distinct from M17 `efe_propose_examples` (catalog augmentation, write-
adjacent) — this M20 helper is read-only operator support, intended
for boosting test coverage / eval datasets, not for catalog evolution.

Default OFF : gated by `EFE_FEATURE_M20_HELPER_TOOLS_ENABLED`.

Pin tests : tests/test_m20_generate_test_sentences.py.
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
    "title": "EfeGenerateTestSentencesInput",
    "description": (
        "LLM-helper : generate N test sentences for a class to boost "
        "gold-dataset coverage. Read-only, does NOT mutate the catalog."
    ),
    "properties": {
        "catalog": {"type": "string", "minLength": 1, "maxLength": 100},
        "version": {
            "type": "string",
            "pattern": "^v\\d+\\.\\d+\\.\\d+$",
        },
        "target_class": {"type": "string", "minLength": 1, "maxLength": 100},
        "count": {
            "type": "integer",
            "minimum": 1,
            "maximum": 64,
            "default": 16,
        },
        "difficulty": {
            "type": "string",
            "enum": ["easy", "medium", "hard", "edge", "mixed"],
            "default": "mixed",
        },
        "language": {
            "type": "string",
            "enum": ["en", "fr", "de", "es", "pt", "it"],
            "default": "en",
        },
        "avoid_redundancy_with_existing": {
            "type": "boolean",
            "default": True,
        },
    },
    "required": ["catalog", "target_class"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not ctx.has_sampling():
        raise ToolError(
            "efe_generate_test_sentences requires MCP sampling.",
            code="sampling_unavailable",
        )

    catalog = args["catalog"]
    target_class = args["target_class"]
    version = args.get("version")
    count = int(args.get("count", 16))
    difficulty = args.get("difficulty", "mixed")
    language = args.get("language", "en")
    avoid_redundancy = bool(args.get("avoid_redundancy_with_existing", True))

    # Fetch existing sentences (best-effort) for anti-redundancy
    existing: list[str] = []
    if avoid_redundancy:
        existing = await _fetch_existing_sentences(
            ctx, catalog, version, target_class,
        )
    # Cap at 30 to control prompt context size + redact PII
    existing_for_prompt = [redact_for_prompt(s) for s in existing[:30]]

    system_prompt, user_prompt = render(
        "generate_test_sentences_v1",
        catalog=catalog,
        target_class=target_class,
        count=count,
        difficulty=difficulty,
        language=_language_name(language),
        existing_sentences_json=json.dumps(existing_for_prompt, ensure_ascii=False),
    )

    combined_prompt = f"{system_prompt}\n\n{user_prompt}"

    async def _retry_callback(errors: list[str]) -> str:
        retry_prompt = (
            f"{combined_prompt}\n\nValidation errors : {'; '.join(errors)}. "
            f"Produce strictly schema-conformant JSON with exactly {count} sentences."
        )
        return await ctx.sampling.sample(
            tenant_id=ctx.tenant_id,
            prompt=retry_prompt,
            max_tokens=2000,
        )

    raw_text = await ctx.sampling.sample(
        tenant_id=ctx.tenant_id,
        prompt=combined_prompt,
        max_tokens=2000,
    )

    output = await validate_with_retry(
        helper="generate_test_sentences",
        raw_output=raw_text,
        retry_callback=_retry_callback,
        max_retries=2,
        expected_count=count,
    )

    return {
        "sentences": output["sentences"],
        "difficulty_self_assessment": output["difficulty_self_assessment"],
        "rationale": output["rationale"],
        "metadata": {
            "catalog": catalog,
            "target_class": target_class,
            "count_requested": count,
            "count_returned": len(output["sentences"]),
            "difficulty": difficulty,
            "language": language,
            "n_existing_referenced": len(existing_for_prompt),
            "prompt_template": "generate_test_sentences_v1",
        },
    }


async def _fetch_existing_sentences(
    ctx: ToolContext,
    catalog: str,
    version: str | None,
    class_name: str,
) -> list[str]:
    """Best-effort fetch ; returns empty list on backend miss."""
    if not hasattr(ctx.backend, "get_class_sentences"):
        return []
    try:
        return await ctx.backend.get_class_sentences(
            catalog=catalog, version=version, class_name=class_name,
            auth=ctx.auth,
        )
    except Exception:  # noqa: BLE001
        return []


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
    name="efe_generate_test_sentences",
    description=(
        "LLM-helper : generate N synthetic test sentences for a class "
        "to boost gold-dataset coverage. Avoids redundancy with existing "
        "sentences. READ-ONLY (does not write to catalog). Requires the "
        "`EFE_FEATURE_M20_HELPER_TOOLS_ENABLED` flag."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=False,
    requires_sampling=True,
    profile_tier="agent-native",
    feature_flag="M20_HELPER_TOOLS_ENABLED",
)
