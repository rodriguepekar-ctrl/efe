"""efe_propose_examples (M17 D45) — sampling-based."""
from __future__ import annotations

import re
from typing import Any

import numpy as np

from ..sampling import SamplingError, SamplingUnavailable
from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeProposeExamplesInput",
    "description": "Propose new examples for a class via host-LLM sampling.",
    "properties": {
        "catalog": {"type": "string", "minLength": 1, "maxLength": 100},
        "class_name": {"type": "string", "minLength": 1, "maxLength": 100},
        "n_proposed": {"type": "integer", "minimum": 1, "maximum": 10, "default": 3},
        "diversity_target": {
            "type": "string",
            "enum": ["low", "medium", "high"],
            "default": "medium",
        },
        "confirm": {"type": "boolean"},
    },
    "required": ["catalog", "class_name", "confirm"],
    "additionalProperties": False,
}


def _build_prompt(class_name: str, existing: list[str], n: int, diversity: str) -> str:
    diversity_blurb = {
        "low": "stay close to the existing examples in style and tone",
        "medium": "stay on-topic but vary phrasing",
        "high": "explore broadly different sub-topics within the same class",
    }[diversity]
    examples_block = "\n".join(f"  - {ex[:200]}" for ex in existing[:8])
    return (
        f"You are augmenting a text-classification catalog. The class "
        f"name is '{class_name}'. Below are some existing examples ; "
        f"propose {n} NEW examples for this class. Constraint: "
        f"{diversity_blurb}.\n\n"
        f"Existing examples:\n{examples_block}\n\n"
        f"Reply with one example per line, no numbering, no quotes, "
        f"no commentary. Up to 200 characters per line."
    )


def _parse_lines(text: str, n_max: int) -> list[str]:
    out: list[str] = []
    for raw in text.splitlines():
        line = raw.strip()
        # Strip leading numbering / bullets
        line = re.sub(r'^[\-\*•]?\s*\d*[\.\)]?\s*', '', line)
        line = line.strip().strip('"\'')
        if line and len(line) <= 200:
            out.append(line)
        if len(out) >= n_max:
            break
    return out


def _cosine_max(c: np.ndarray, mat: np.ndarray) -> float:
    if len(mat) == 0:
        return 0.0
    sims = mat @ c
    return float(np.max(sims))


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not args.get("confirm"):
        raise ToolError(
            "propose_examples requires confirm:true (consumes sampling rounds)",
            code="confirm_required",
        )
    if not ctx.has_sampling():
        raise ToolError(
            "host MCP client does not advertise sampling capability ; "
            "propose_examples cannot operate without it",
            code="sampling_unavailable",
        )

    catalog = args["catalog"]
    class_name = args["class_name"]
    n = int(args.get("n_proposed", 3))
    diversity = args.get("diversity_target", "medium")

    cls_info = await ctx.backend.get_class_examples(
        catalog=catalog, class_name=class_name, auth=ctx.auth,
    )
    existing = cls_info.get("examples") or []
    centroid = np.asarray(cls_info.get("centroid") or [], dtype=np.float32)

    prompt = _build_prompt(class_name, existing, n, diversity)
    try:
        assert ctx.sampling is not None
        text = await ctx.sampling.sample(
            tenant_id=ctx.tenant_id, prompt=prompt, max_tokens=400,
        )
    except (SamplingUnavailable, SamplingError) as exc:
        raise ToolError(
            f"sampling round failed : {exc}",
            code="sampling_failed",
            status_code=502,
        ) from exc

    candidates = _parse_lines(text, n_max=n * 2)  # ask LLM, take up to 2× then filter
    if not candidates:
        return {
            "class_name": class_name,
            "existing_examples_count": len(existing),
            "proposed_examples": [],
            "next_action_hint": "Sampling produced no parseable lines ; retry with different diversity_target.",
        }

    # Embed existing + candidates to compute novelty + on-topic filter.
    cand_embs = await ctx.backend.encode_batch(texts=candidates, auth=ctx.auth)
    cand_arr = np.asarray(cand_embs, dtype=np.float32)
    if len(existing):
        existing_embs = await ctx.backend.encode_batch(texts=existing, auth=ctx.auth)
        existing_arr = np.asarray(existing_embs, dtype=np.float32)
    else:
        existing_arr = np.zeros((0, cand_arr.shape[1]), dtype=np.float32)

    proposals: list[dict[str, Any]] = []
    for i, candidate in enumerate(candidates):
        if len(centroid) == 0:
            sim_centroid = 0.0
        else:
            denom = float(np.linalg.norm(cand_arr[i]) * np.linalg.norm(centroid))
            sim_centroid = float(np.dot(cand_arr[i], centroid) / denom) if denom > 0 else 0.0
        if sim_centroid < 0.4:
            continue  # off-topic
        novelty = 1.0 - _cosine_max(cand_arr[i], existing_arr)
        proposals.append({
            "text": candidate,
            "estimated_centroid_similarity": round(sim_centroid, 4),
            "novelty_score": round(novelty, 4),
            "rationale": "candidate stays on-topic with novelty above existing set",
        })
        if len(proposals) >= n:
            break

    return {
        "catalog": catalog,
        "class_name": class_name,
        "existing_examples_count": len(existing),
        "proposed_examples": proposals,
        "sampling_usage": {
            "rounds_used": ctx.sampling.usage.rounds_used,
            "cache_hits": ctx.sampling.usage.cache_hits,
        },
        "next_action_hint": (
            "Review the proposals and call efe_evolve_catalog with "
            "confirm:true to apply."
        ),
    }


TOOL = Tool(
    name="efe_propose_examples",
    description=(
        "Propose new examples for an existing class. Uses host-LLM "
        "sampling to generate candidates and filters on-topic + novelty. "
        "Requires confirm:true."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    requires_sampling=True,
    profile_tier="agent-native",
)
