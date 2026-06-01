"""efe_compare_bundles_quality (M20 D55) — LLM-helper, sampling-delegated.

Compare two bundle versions of a catalog on a sample of test sentences.
Process :
  1. Resolve test_sentences (provided OR sample from gold dataset).
  2. Run classify Mode-1 on both versions for all sentences.
  3. Collect divergences (different label OR significant confidence
     gap).
  4. Ask the LLM to analyse the divergences and produce a recommendation.

Default OFF : gated by `EFE_FEATURE_M20_HELPER_TOOLS_ENABLED`.

Pin tests : tests/test_m20_compare_bundles_quality.py.
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
    "title": "EfeCompareBundlesQualityInput",
    "description": (
        "LLM-helper : compare quality of two bundle versions on a sample "
        "of sentences and recommend promote/investigate/do_not_promote."
    ),
    "properties": {
        "catalog": {"type": "string", "minLength": 1, "maxLength": 100},
        "version_a": {
            "type": "string",
            "pattern": "^v\\d+\\.\\d+\\.\\d+$",
            "description": "Current production version.",
        },
        "version_b": {
            "type": "string",
            "pattern": "^v\\d+\\.\\d+\\.\\d+$",
            "description": "Candidate version to evaluate.",
        },
        "test_sentences": {
            "type": "array",
            "items": {"type": "string", "minLength": 1, "maxLength": 4096},
            "minItems": 1,
            "maxItems": 200,
        },
        "sample_size": {
            "type": "integer",
            "minimum": 10,
            "maximum": 500,
            "default": 100,
        },
    },
    "required": ["catalog", "version_a", "version_b"],
    "additionalProperties": False,
    # If neither provided, sample_size default applies.
}

# Hard cap on divergence examples in prompt to avoid context blowup.
_MAX_DIVERGENCES_IN_PROMPT = 50


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not ctx.has_sampling():
        raise ToolError(
            "efe_compare_bundles_quality requires MCP sampling.",
            code="sampling_unavailable",
        )

    catalog = args["catalog"]
    va, vb = args["version_a"], args["version_b"]
    if va == vb:
        raise ToolError(
            f"version_a and version_b must differ ; both = {va!r}",
            code="invalid_arguments",
        )

    # 1. Resolve sentences
    sentences = args.get("test_sentences")
    if sentences is None:
        sample_size = int(args.get("sample_size", 100))
        sentences = await _sample_gold_sentences(ctx, catalog, sample_size)
    if not sentences:
        raise ToolError(
            f"no sentences available to compare for catalog={catalog!r}",
            code="not_found",
        )

    # 2. Classify on both versions (in parallel via gather pattern)
    results_a = await _classify_batch(ctx, catalog, va, sentences)
    results_b = await _classify_batch(ctx, catalog, vb, sentences)

    if len(results_a) != len(sentences) or len(results_b) != len(sentences):
        raise ToolError(
            "classify_batch returned unexpected length",
            code="backend_error",
        )

    # 3. Compute divergences
    divergences = []
    for i, s in enumerate(sentences):
        a, b = results_a[i], results_b[i]
        same_label = a["label"] == b["label"]
        conf_gap = abs(float(a["confidence"]) - float(b["confidence"]))
        if not same_label or conf_gap >= 0.10:
            divergences.append({
                "sentence": redact_for_prompt(s),
                "label_a": a["label"],
                "confidence_a": round(float(a["confidence"]), 3),
                "label_b": b["label"],
                "confidence_b": round(float(b["confidence"]), 3),
            })

    # Sort divergences worst-confidence first (keep most informative)
    divergences.sort(
        key=lambda d: max(d["confidence_a"], d["confidence_b"]),
        reverse=False,
    )
    div_for_prompt = divergences[:_MAX_DIVERGENCES_IN_PROMPT]

    # 4. Render prompt + sampling
    system_prompt, user_prompt = render(
        "compare_bundles_quality_v1",
        catalog=catalog,
        version_a=va,
        version_b=vb,
        n_total=len(sentences),
        n_divergent=len(divergences),
        divergences_json=json.dumps(div_for_prompt, ensure_ascii=False),
    )

    combined_prompt = f"{system_prompt}\n\n{user_prompt}"

    async def _retry_callback(errors: list[str]) -> str:
        retry_prompt = (
            f"{combined_prompt}\n\nValidation errors : {'; '.join(errors)}. "
            f"Produce a strictly schema-conformant JSON."
        )
        return await ctx.sampling.sample(
            tenant_id=ctx.tenant_id,
            prompt=retry_prompt,
            max_tokens=1500,
        )

    raw_text = await ctx.sampling.sample(
        tenant_id=ctx.tenant_id,
        prompt=combined_prompt,
        max_tokens=1500,
    )

    output = await validate_with_retry(
        helper="compare_bundles_quality",
        raw_output=raw_text,
        retry_callback=_retry_callback,
        max_retries=2,
    )

    return {
        "n_sentences_compared": len(sentences),
        "n_divergent": len(divergences),
        "accuracy_delta_pp": output["accuracy_delta_pp"],
        "divergence_examples": output["divergence_examples"],
        "summary": output["summary"],
        "recommendation": output["recommendation"],
        "metadata": {
            "version_a": va,
            "version_b": vb,
            "prompt_template": "compare_bundles_quality_v1",
            "divergences_truncated_in_prompt": (
                len(divergences) > _MAX_DIVERGENCES_IN_PROMPT
            ),
        },
    }


async def _classify_batch(
    ctx: ToolContext, catalog: str, version: str, sentences: list[str],
) -> list[dict[str, Any]]:
    """Best-effort batch classify ; falls back to per-call if backend
    lacks a `classify_batch` method (M19 GA backends do)."""
    if hasattr(ctx.backend, "classify_batch"):
        return await ctx.backend.classify_batch(
            catalog=catalog, version=version, sentences=sentences,
            auth=ctx.auth,
        )
    out: list[dict[str, Any]] = []
    for s in sentences:
        r = await ctx.backend.classify(
            catalog=catalog, version=version, text=s, auth=ctx.auth,
        )
        out.append(r)
    return out


async def _sample_gold_sentences(
    ctx: ToolContext, catalog: str, n: int,
) -> list[str]:
    """Best-effort sample ; M19 GA backend exposes sample_gold_sentences."""
    if hasattr(ctx.backend, "sample_gold_sentences"):
        return await ctx.backend.sample_gold_sentences(
            catalog=catalog, n=n, auth=ctx.auth,
        )
    raise ToolError(
        "no test_sentences provided and backend does not expose "
        "sample_gold_sentences ; provide test_sentences explicitly",
        code="not_found",
    )


TOOL = Tool(
    name="efe_compare_bundles_quality",
    description=(
        "LLM-helper : compare quality of two bundle versions on test "
        "sentences. Returns a recommendation (promote / investigate / "
        "do_not_promote) and divergence examples. READ-ONLY. Requires "
        "the `EFE_FEATURE_M20_HELPER_TOOLS_ENABLED` flag."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=False,
    requires_sampling=True,
    profile_tier="agent-native",
    feature_flag="M20_HELPER_TOOLS_ENABLED",
)
