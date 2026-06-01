"""efe_calibrate_thresholds (M17 D45) — streaming."""
from __future__ import annotations

from typing import Any

from ..analysis.thresholds import calibrate_per_class
from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeCalibrateThresholdsInput",
    "description": "Per-class threshold optimisation toward a target metric.",
    "properties": {
        "catalog": {"type": "string", "minLength": 1, "maxLength": 100},
        "target_metric": {
            "type": "string",
            "enum": ["precision", "recall", "f1"],
            "default": "f1",
        },
        "target_value": {"type": "number", "minimum": 0.0, "maximum": 1.0, "default": 0.9},
        "validation_set": {
            "type": "array",
            "maxItems": 1000,
            "items": {
                "type": "object",
                "properties": {
                    "text": {"type": "string", "maxLength": 8192},
                    "true_class": {"type": "string", "maxLength": 100},
                },
                "required": ["text", "true_class"],
                "additionalProperties": False,
            },
        },
        "min_per_class_samples": {
            "type": "integer", "minimum": 5, "maximum": 1000, "default": 30,
        },
    },
    "required": ["catalog"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    catalog = args["catalog"]
    target_metric = args.get("target_metric", "f1")
    target_value = float(args.get("target_value", 0.9))
    min_samples = int(args.get("min_per_class_samples", 30))
    validation = args.get("validation_set")

    classes = await ctx.backend.list_catalog_classes(catalog=catalog, auth=ctx.auth)
    if not classes:
        raise ToolError(
            f"catalog {catalog!r} has no classes ; nothing to calibrate",
            code="invalid_arguments",
            status_code=400,
        )

    # Audit fix M17-A1 (HIGH) — when `validation_set` is provided we
    # MUST honour ground-truth labels. Previous impl forwarded pos+neg
    # mixed to score_distribution(include_negatives=False) which lost
    # the labels, silently miscalibrating. Now we classify each
    # validation entry directly and partition by `true_class`.
    score_distributions: dict[str, dict] = {}
    method = "internal_holdout"
    n = len(classes)

    if validation:
        method = "provided_validation"
        # Pre-classify every validation entry once ; reuse the score
        # against each class.
        per_entry_scores: list[dict[str, Any]] = []
        for entry in validation:
            r = await ctx.backend.classify(
                catalog=catalog, text=entry["text"], top_k=32, auth=ctx.auth,
            )
            scores_by_class: dict[str, float] = {}
            for s in r.get("top_k", []):
                scores_by_class[s["class_name"]] = float(s["score"])
            per_entry_scores.append({
                "true_class": entry["true_class"],
                "scores_by_class": scores_by_class,
            })

        for i, cls in enumerate(classes):
            cn = cls["name"]
            if ctx.progress:
                ctx.progress.check_cancelled()
                await ctx.progress.emit(
                    progress=int(((i + 1) * 100) / max(1, n)),
                    total=100,
                    message=f"Calibrating class {cn} ({i + 1}/{n})",
                )
            pos_scores: list[float] = []
            neg_scores: list[float] = []
            for entry in per_entry_scores:
                score = entry["scores_by_class"].get(cn, 0.0)
                if entry["true_class"] == cn:
                    pos_scores.append(score)
                else:
                    neg_scores.append(score)
            score_distributions[cn] = {
                "positives": pos_scores,
                "negatives": neg_scores,
            }
    else:
        for i, cls in enumerate(classes):
            if ctx.progress:
                ctx.progress.check_cancelled()
                await ctx.progress.emit(
                    progress=int(((i + 1) * 100) / max(1, n)),
                    total=100,
                    message=f"Calibrating class {cls['name']} ({i + 1}/{n})",
                )
            res = await ctx.backend.score_distribution(
                catalog=catalog,
                class_name=cls["name"],
                sample_count=min_samples,
                texts=None,
                include_negatives=True,
                auth=ctx.auth,
            )
            score_distributions[cls["name"]] = {
                "positives": res.get("positive_scores", []),
                "negatives": res.get("negative_scores", []),
            }

    calibrations = calibrate_per_class(
        classes=classes,
        score_distributions=score_distributions,
        target_metric=target_metric,
        target_value=target_value,
    )
    per_class_results = [
        {
            "class_name": c.class_name,
            "current_threshold": c.current_threshold,
            "recommended_threshold": c.recommended_threshold,
            "expected_precision": c.expected_precision,
            "expected_recall": c.expected_recall,
            "expected_f1": c.expected_f1,
            "samples_used": c.samples_used,
        }
        for c in calibrations
    ]

    avg_p = (
        sum(c["expected_precision"] for c in per_class_results) / len(per_class_results)
        if per_class_results else 0.0
    )
    avg_r = (
        sum(c["expected_recall"] for c in per_class_results) / len(per_class_results)
        if per_class_results else 0.0
    )

    return {
        "catalog": catalog,
        "calibration_method": method,
        "target_metric": target_metric,
        "target_value": target_value,
        "per_class": per_class_results,
        "global_summary": {
            "avg_expected_precision": round(avg_p, 4),
            "avg_expected_recall": round(avg_r, 4),
        },
        "next_action_hint": (
            "Apply via efe_evolve_catalog : pass the new "
            "confidence_threshold per class (confirm:true required)."
        ),
    }


TOOL = Tool(
    name="efe_calibrate_thresholds",
    description=(
        "Compute per-class confidence thresholds optimised toward a "
        "target metric (precision/recall/F1). Streams progress. "
        "Returns recommendations only — apply via efe_evolve_catalog."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    streams_progress=True,
    profile_tier="agent-native",
)
