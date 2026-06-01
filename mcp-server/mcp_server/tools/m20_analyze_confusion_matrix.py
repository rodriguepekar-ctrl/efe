"""efe_analyze_confusion_matrix (M20 D57) — LLM-helper, sampling-delegated.

Take a confusion matrix (true_class → {predicted_class → count}) and
ask the LLM to identify the top-K most concerning confusion pairs and
propose actions (split, merge, reword, investigate) per pair.

Default OFF : gated by `EFE_FEATURE_M20_HELPER_TOOLS_ENABLED`.

Pin tests : tests/test_m20_analyze_confusion_matrix.py.
"""
from __future__ import annotations

import json
from typing import Any

from .m20_prompts import render
from .m20_validation import validate_with_retry
from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeAnalyzeConfusionMatrixInput",
    "description": (
        "LLM-helper : analyze a confusion matrix and propose actions "
        "(split / merge / reword / investigate) per confusion pair."
    ),
    "properties": {
        "catalog": {"type": "string", "minLength": 1, "maxLength": 100},
        "version": {
            "type": "string",
            "pattern": "^v\\d+\\.\\d+\\.\\d+$",
        },
        "confusion_matrix": {
            "type": "object",
            "description": "Map of true_class → {predicted_class → count}.",
            "additionalProperties": {
                "type": "object",
                "additionalProperties": {"type": "integer", "minimum": 0},
            },
            "minProperties": 2,
        },
        "min_confusion_rate": {
            "type": "number",
            "minimum": 0.0,
            "maximum": 1.0,
            "default": 0.05,
        },
        "max_pairs": {
            "type": "integer",
            "minimum": 1,
            "maximum": 20,
            "default": 5,
        },
    },
    "required": ["catalog", "confusion_matrix"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not ctx.has_sampling():
        raise ToolError(
            "efe_analyze_confusion_matrix requires MCP sampling.",
            code="sampling_unavailable",
        )

    catalog = args["catalog"]
    version = args.get("version", "")
    matrix: dict[str, dict[str, int]] = args["confusion_matrix"]
    min_rate = float(args.get("min_confusion_rate", 0.05))
    max_pairs = int(args.get("max_pairs", 5))

    if not (0.0 <= min_rate <= 1.0):
        raise ToolError(
            "min_confusion_rate must be in [0, 1]",
            code="invalid_arguments",
        )

    # Pre-process : compute rates client-side ; the LLM gets a pre-filtered
    # short list rather than the raw counts (smaller prompt, less hallucination
    # risk). The LLM is responsible for the qualitative analysis only.
    pre_filtered_pairs = _extract_top_pairs(
        matrix, min_rate=min_rate, max_pairs=max_pairs,
    )
    if not pre_filtered_pairs:
        return {
            "top_confusion_pairs": [],
            "proposed_actions": [],
            "summary": (
                f"No confusion pairs above min_confusion_rate={min_rate} "
                f"in catalog {catalog!r}. The classifier appears well-separated."
            ),
            "metadata": {
                "catalog": catalog,
                "n_classes": len(matrix),
                "n_pairs_above_threshold": 0,
                "prompt_template": "analyze_confusion_matrix_v1",
            },
        }

    system_prompt, user_prompt = render(
        "analyze_confusion_matrix_v1",
        catalog=catalog,
        version=version or "(unspecified)",
        confusion_matrix_json=json.dumps(pre_filtered_pairs, ensure_ascii=False),
        min_confusion_rate=min_rate,
        max_pairs=max_pairs,
    )

    combined_prompt = f"{system_prompt}\n\n{user_prompt}"

    async def _retry_callback(errors: list[str]) -> str:
        retry_prompt = (
            f"{combined_prompt}\n\nValidation errors : {'; '.join(errors)}. "
            f"Produce strictly schema-conformant JSON."
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
        helper="analyze_confusion_matrix",
        raw_output=raw_text,
        retry_callback=_retry_callback,
        max_retries=2,
    )

    return {
        "top_confusion_pairs": output["top_confusion_pairs"],
        "proposed_actions": output["proposed_actions"],
        "summary": output["summary"],
        "metadata": {
            "catalog": catalog,
            "version": version,
            "n_classes": len(matrix),
            "n_pairs_above_threshold": len(pre_filtered_pairs),
            "min_confusion_rate": min_rate,
            "max_pairs": max_pairs,
            "prompt_template": "analyze_confusion_matrix_v1",
        },
    }


def _extract_top_pairs(
    matrix: dict[str, dict[str, int]],
    *,
    min_rate: float,
    max_pairs: int,
) -> list[dict[str, Any]]:
    """Compute confusion rates and return top pairs above threshold.

    Returns a list of dicts with per-pair rate. Self-confusions
    (diagonal) are excluded. Sorted by rate descending.
    """
    pairs: list[dict[str, Any]] = []
    for true_class, row in matrix.items():
        if not isinstance(row, dict):
            continue
        total = sum(int(v) for v in row.values() if isinstance(v, int))
        if total == 0:
            continue
        for predicted, count in row.items():
            if predicted == true_class:
                continue
            rate = float(count) / float(total)
            if rate >= min_rate:
                pairs.append({
                    "true_class": true_class,
                    "predicted_class": predicted,
                    "count": int(count),
                    "row_total": total,
                    "rate": round(rate, 4),
                })
    pairs.sort(key=lambda p: p["rate"], reverse=True)
    return pairs[:max_pairs]


TOOL = Tool(
    name="efe_analyze_confusion_matrix",
    description=(
        "LLM-helper : analyse a confusion matrix and propose qualitative "
        "actions (split / merge / reword / investigate) for the worst "
        "confusion pairs. READ-ONLY. Requires the "
        "`EFE_FEATURE_M20_HELPER_TOOLS_ENABLED` flag."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=False,
    requires_sampling=True,
    profile_tier="agent-native",
    feature_flag="M20_HELPER_TOOLS_ENABLED",
)
