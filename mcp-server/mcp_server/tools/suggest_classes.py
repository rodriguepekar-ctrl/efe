"""efe_suggest_classes (M17 D45) — sampling-based.

Bootstraps a catalog from a flat list of texts. Steps :
  1. Fetch embeddings for every example (batch) via backend.encode_batch.
  2. Cluster embeddings with MiniBatchKMeans (or fallback).
  3. For each cluster : if sampling is available, ask the host LLM to
     propose a kebab-case name. Else fallback to top-3 TF-IDF terms.
  4. Compute intra-cluster cohesion + outliers.

The tool is in `agent-native` profile only (sampling required for
optimal naming). It still works in `advanced` profile if forced, but
class names will be heuristic.
"""
from __future__ import annotations

import re
from collections import Counter
from typing import Any

import numpy as np

from ..analysis.clustering import ClusterResult, cluster_kmeans, detect_outliers
from ..sampling import SamplingError, SamplingUnavailable
from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeSuggestClassesInput",
    "description": (
        "Cluster a flat list of example texts into classes ; uses MCP "
        "sampling to name each cluster (with heuristic fallback)."
    ),
    "properties": {
        "examples": {
            "type": "array",
            "minItems": 10,
            "maxItems": 2000,
            "items": {
                "type": "object",
                "properties": {
                    "text": {"type": "string", "minLength": 1, "maxLength": 8192}
                },
                "required": ["text"],
                "additionalProperties": False,
            },
        },
        "target_class_count": {
            "type": "integer", "minimum": 2, "maximum": 50, "default": 5,
        },
        "language_hint": {
            "type": "string", "minLength": 2, "maxLength": 10, "default": "en",
        },
        "confirm": {"type": "boolean"},
    },
    "required": ["examples", "confirm"],
    "additionalProperties": False,
}


_KEBAB_RE = re.compile(r"[^a-z0-9\-]+")


def _kebab_case(name: str) -> str:
    n = name.strip().lower()
    n = _KEBAB_RE.sub("-", n)
    n = re.sub(r"-+", "-", n).strip("-")
    return n[:40] or "class"


def _heuristic_name(texts: list[str], lang: str) -> str:
    """Top-3 TF-IDF-ish terms as a kebab-case name (lang-agnostic)."""
    word_counts: Counter[str] = Counter()
    for t in texts:
        for w in re.findall(r"[a-zA-Z]{4,}", t.lower()):
            word_counts[w] += 1
    common: set[str] = {
        # Tiny stop-list, kept ASCII to avoid lang-specific churn.
        "the", "and", "with", "that", "this", "from", "have", "been",
        "your", "their", "when", "what", "would", "could", "about",
    }
    keywords = [w for w, _ in word_counts.most_common(8) if w not in common][:3]
    return _kebab_case("-".join(keywords) or "class")


def _sampling_prompt(top_examples: list[str], lang: str) -> str:
    bullets = "\n".join(f"  - {ex[:200]}" for ex in top_examples)
    return (
        f"You are naming a cluster of texts. Suggest ONE short, "
        f"kebab-case class name (max 25 characters, lowercase, hyphens "
        f"only, no spaces, no punctuation). Language: {lang}.\n\n"
        f"Cluster examples:\n{bullets}\n\n"
        f"Reply with ONLY the class name, nothing else."
    )


async def _name_cluster(
    cluster: ClusterResult,
    all_texts: list[str],
    *,
    ctx: ToolContext,
    lang: str,
) -> tuple[str, str]:
    """Return (class_name, naming_method).

    `cluster.nearest_examples` are GLOBAL indices into `all_texts`.
    """
    top_examples = [all_texts[i] for i in cluster.nearest_examples
                    if 0 <= i < len(all_texts)]
    if not top_examples:
        return _heuristic_name([], lang), "heuristic_top_terms"
    if ctx.has_sampling():
        prompt = _sampling_prompt(top_examples, lang)
        try:
            assert ctx.sampling is not None
            text = await ctx.sampling.sample(
                tenant_id=ctx.tenant_id, prompt=prompt, max_tokens=50,
            )
            name = _kebab_case(text.split()[0] if text.split() else "class")
            return name, "sampling"
        except (SamplingUnavailable, SamplingError):
            pass  # fall through to heuristic
    return _heuristic_name(top_examples, lang), "heuristic_top_terms"


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not args.get("confirm"):
        raise ToolError(
            "suggest_classes requires confirm:true (uses LLM sampling rounds)",
            code="confirm_required",
        )
    examples_in = args["examples"]
    target = int(args.get("target_class_count", 5))
    lang = args.get("language_hint", "en")
    texts = [e["text"] for e in examples_in]
    if len(texts) < target:
        raise ToolError(
            f"need at least target_class_count={target} examples, got {len(texts)}",
            code="invalid_arguments",
            status_code=400,
        )

    # Encode in one round-trip
    embs = await ctx.backend.encode_batch(texts=texts, auth=ctx.auth)
    embs_arr = np.asarray(embs, dtype=np.float32)

    clusters = cluster_kmeans(embeddings=embs_arr, n_clusters=target)
    outlier_idx = set(detect_outliers(embeddings=embs_arr, clusters=clusters))

    proposed: list[dict[str, Any]] = []
    naming_method = "heuristic_top_terms"
    for c in clusters:
        members = [i for i in c.member_indices if i not in outlier_idx]
        if not members:
            continue
        name, method = await _name_cluster(c, texts, ctx=ctx, lang=lang)
        if method == "sampling":
            naming_method = "sampling"
        proposed.append({
            "name": name,
            "confidence": round(float(c.intra_cohesion), 4),
            "examples": [texts[i] for i in c.nearest_examples
                         if i in c.member_indices and i not in outlier_idx][:5],
            "size": len(members),
            "intra_cluster_cohesion": round(float(c.intra_cohesion), 4),
        })

    outliers = [
        {"text": texts[i], "reason": "low_cohesion_with_all_clusters"}
        for i in sorted(outlier_idx)
    ]

    return {
        "proposed_classes": proposed,
        "outliers": outliers,
        "naming_method": naming_method,
        "sampling_usage": (
            {
                "rounds_used": ctx.sampling.usage.rounds_used,
                "cache_hits": ctx.sampling.usage.cache_hits,
            } if ctx.sampling is not None else None
        ),
        "next_action_hint": (
            "Review proposed_classes ; call efe_register_catalog with "
            "confirm:true once approved."
        ),
    }


TOOL = Tool(
    name="efe_suggest_classes",
    description=(
        "Cluster a flat list of example texts into N classes and propose "
        "kebab-case names. Uses MCP sampling to ask the host LLM for "
        "names ; falls back to heuristic top-terms if sampling is "
        "unavailable. WRITE-style — requires confirm:true (consumes "
        "sampling rounds from the host LLM)."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=False,
    requires_sampling=True,
    profile_tier="agent-native",
)
