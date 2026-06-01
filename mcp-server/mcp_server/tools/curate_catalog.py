"""efe_curate_catalog (M17 D45) — composite write+sampling+streaming.

Produces a *PLAN* (not an applied mutation). The agent reviews the plan
and can subsequently call evolve/register to apply.

`confirm:true` here gates the *cost* of the LLM sampling rounds, not a
write — the tool is dry-run by design.
"""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeCurateCatalogInput",
    "description": (
        "Build a curation plan for a catalog (dry-run only). Streams "
        "progress events ; consumes sampling rounds for explanations."
    ),
    "properties": {
        "catalog": {"type": "string", "minLength": 1, "maxLength": 100},
        "intent": {
            "type": "string",
            "enum": [
                "balance_class_sizes",
                "remove_ambiguity",
                "augment_underrepresented",
            ],
        },
        "max_changes": {"type": "integer", "minimum": 1, "maximum": 100, "default": 20},
        "confirm": {"type": "boolean"},
    },
    "required": ["catalog", "intent", "confirm"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not args.get("confirm"):
        raise ToolError(
            "curate_catalog requires confirm:true (consumes LLM sampling rounds)",
            code="confirm_required",
        )
    catalog = args["catalog"]
    intent = args["intent"]
    max_changes = int(args.get("max_changes", 20))

    # Phase 1 : diagnose
    if ctx.progress:
        await ctx.progress.emit(
            progress=10, total=100, message="Phase 1/4 : running diagnose_catalog"
        )
    diagnose = await ctx.backend.diagnose_catalog(
        catalog=catalog, depth="standard", include_examples_text=False, auth=ctx.auth,
    )

    # Phase 2 : build candidate actions based on intent
    if ctx.progress:
        ctx.progress.check_cancelled()
        await ctx.progress.emit(
            progress=40, total=100, message="Phase 2/4 : computing candidate actions"
        )
    actions: list[dict[str, Any]] = []
    if intent == "augment_underrepresented":
        for cls in diagnose.get("underrepresented_classes", [])[:max_changes]:
            actions.append({
                "type": "add_examples",
                "class": cls.get("class_name"),
                "examples": [],   # filled by sampling in phase 3 if available
                "reason": f"underrepresented (size={cls.get('size')})",
            })
    elif intent == "remove_ambiguity":
        for amb in diagnose.get("ambiguity_matrix", [])[:max_changes]:
            actions.append({
                "type": "review_boundary",
                "between": [amb.get("class_a"), amb.get("class_b")],
                "reason": f"ambiguity band={amb.get('band')}",
            })
    elif intent == "balance_class_sizes":
        # Pair up under/over-represented based on the diagnostic
        for cls in diagnose.get("underrepresented_classes", [])[:max_changes]:
            actions.append({
                "type": "add_examples",
                "class": cls.get("class_name"),
                "examples": [],
                "reason": "balance_target",
            })

    # Phase 3 : sampling for human-readable rationales (optional)
    if ctx.progress:
        ctx.progress.check_cancelled()
        await ctx.progress.emit(
            progress=70, total=100, message="Phase 3/4 : drafting rationales"
        )
    if ctx.has_sampling() and actions:
        # One sampling round to enrich the plan with a top-level rationale
        prompt = (
            f"Briefly explain (under 60 words) why a curation plan with "
            f"intent={intent!r} and {len(actions)} changes makes sense "
            f"for an EFE catalog with overall_health="
            f"{diagnose.get('overall_health', 'unknown')!r}. "
            f"Reply with a single paragraph."
        )
        try:
            assert ctx.sampling is not None
            rationale = await ctx.sampling.sample(
                tenant_id=ctx.tenant_id, prompt=prompt, max_tokens=200,
            )
        except Exception:  # noqa: BLE001
            rationale = "Plan derived from diagnose_catalog signals."
    else:
        rationale = "Plan derived from diagnose_catalog signals."

    # Phase 4 : assemble
    if ctx.progress:
        ctx.progress.check_cancelled()
        await ctx.progress.emit(
            progress=95, total=100, message="Phase 4/4 : assembling plan"
        )

    plan = {
        "actions": actions[:max_changes],
        "rationale": rationale,
        "estimated_impact": {
            "size_balance_score_after": diagnose.get("scores", {}).get("balance", 0.0),
            "ambiguity_score_after": diagnose.get("scores", {}).get("ambiguity", 0.0),
        },
    }
    if ctx.progress:
        await ctx.progress.emit(progress=100, total=100, message="done")

    return {
        "catalog": catalog,
        "intent": intent,
        "current_state": {
            "version": diagnose.get("version"),
            "fingerprint": diagnose.get("fingerprint"),
            "overall_health": diagnose.get("overall_health"),
        },
        "plan": plan,
        "sampling_usage": (
            {
                "rounds_used": ctx.sampling.usage.rounds_used,
                "cache_hits": ctx.sampling.usage.cache_hits,
            } if ctx.sampling is not None else None
        ),
        "next_action_hint": (
            "Review the plan ; apply by calling efe_evolve_catalog or "
            "efe_register_catalog with confirm:true."
        ),
    }


TOOL = Tool(
    name="efe_curate_catalog",
    description=(
        "Build a multi-step curation plan for a catalog "
        "(balance/ambiguity/augment) — DRY RUN ; the tool only proposes, "
        "the agent applies via separate write tools. Streams progress "
        "events. Requires confirm:true (consumes sampling rounds)."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=False,
    requires_sampling=True,
    streams_progress=True,
    profile_tier="agent-native",
)
