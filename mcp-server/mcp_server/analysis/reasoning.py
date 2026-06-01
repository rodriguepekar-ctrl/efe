"""Reasoning helpers — explain a classification, compare classes,
threshold sensitivity. Pure numpy, no I/O.
"""
from __future__ import annotations

from dataclasses import dataclass
from typing import Sequence

import numpy as np


@dataclass(frozen=True)
class ContributorExample:
    example: str
    similarity_to_input: float
    weight_in_centroid: float


@dataclass(frozen=True)
class ExplainResult:
    predicted_class: str
    score: float
    above_threshold: bool
    threshold: float
    runner_up_class: str | None
    runner_up_score: float
    gap: float
    contributors: list[ContributorExample]
    is_ambiguous: bool
    threshold_sensitivity_low: float
    threshold_sensitivity_high: float


def _cosine(a: np.ndarray, b: np.ndarray) -> float:
    """Cosine similarity for L2-normalized vectors = simple dot product.

    Falls back to manual normalization when norms are not unit ; this
    keeps the helper safe even if the caller passes unnormalized data.
    """
    na = float(np.linalg.norm(a))
    nb = float(np.linalg.norm(b))
    if na == 0 or nb == 0:
        return 0.0
    return float(np.dot(a, b) / (na * nb))


def explain_top1(
    *,
    text_embedding: np.ndarray,
    classes: Sequence[dict],
    predicted_class: str | None = None,
    ambiguity_gap_threshold: float = 0.05,
) -> ExplainResult:
    """Compute a structured explanation for the top-1 classification.

    `classes` is a list of dicts with keys :
      - name (str)
      - centroid (list[float] | np.ndarray, 384-dim)
      - examples (list[str])
      - example_embeddings (list[list[float]] | np.ndarray) — optional,
        used only to compute the contributor ranking. If absent, the
        contributors list is left empty (caller signals fallback mode).
      - confidence_threshold (float)
    """
    if not classes:
        raise ValueError("explain_top1: classes is empty")

    # Score every class
    scored: list[tuple[str, float, dict]] = []
    for cls in classes:
        centroid = np.asarray(cls["centroid"], dtype=np.float32)
        score = _cosine(text_embedding, centroid)
        scored.append((cls["name"], score, cls))
    scored.sort(key=lambda x: x[1], reverse=True)

    # Pick predicted_class (default to top-1)
    top_name, top_score, top_cls = scored[0]
    if predicted_class is not None:
        # Find the user-supplied class in the scored list
        match = next((s for s in scored if s[0] == predicted_class), None)
        if match is None:
            raise ValueError(
                f"explain_top1: predicted_class {predicted_class!r} not found in catalog"
            )
        top_name, top_score, top_cls = match

    threshold = float(top_cls.get("confidence_threshold", 0.5))
    above_threshold = top_score >= threshold

    runner_up_name: str | None = None
    runner_up_score = 0.0
    if len(scored) >= 2:
        # Pick the highest-scoring class that ISN'T predicted_class
        for name, score, _ in scored:
            if name != top_name:
                runner_up_name = name
                runner_up_score = score
                break
    gap = float(top_score - runner_up_score)
    is_ambiguous = gap < ambiguity_gap_threshold

    # Contributors : if example_embeddings available, rank by similarity
    contributors: list[ContributorExample] = []
    embs = top_cls.get("example_embeddings")
    examples: list[str] = top_cls.get("examples", []) or []
    if embs is not None and len(examples) > 0:
        embs_arr = np.asarray(embs, dtype=np.float32)
        sims = np.array(
            [_cosine(text_embedding, embs_arr[i]) for i in range(len(examples))],
            dtype=np.float32,
        )
        # Top-3 contributors
        order = np.argsort(-sims)[:3]
        n = max(1, len(examples))
        weight = 1.0 / n
        for idx in order:
            contributors.append(
                ContributorExample(
                    example=examples[int(idx)],
                    similarity_to_input=float(sims[int(idx)]),
                    weight_in_centroid=weight,
                )
            )

    # Threshold sensitivity : the range of thresholds where the top
    # decision is still `top_name`. Since we just compare against
    # `confidence_threshold`, the range where decision == above_threshold
    # is the band [0, top_score] for "above" and [top_score, 1] for "below".
    if above_threshold:
        sens_low = 0.0
        sens_high = top_score
    else:
        sens_low = top_score
        sens_high = 1.0

    return ExplainResult(
        predicted_class=top_name,
        score=top_score,
        above_threshold=above_threshold,
        threshold=threshold,
        runner_up_class=runner_up_name,
        runner_up_score=runner_up_score,
        gap=gap,
        contributors=contributors,
        is_ambiguous=is_ambiguous,
        threshold_sensitivity_low=sens_low,
        threshold_sensitivity_high=sens_high,
    )


# ─────────────────────────────────────────────────────────────────────
# Compare two classes
# ─────────────────────────────────────────────────────────────────────


@dataclass(frozen=True)
class CompareResult:
    centroid_cosine: float
    centroid_distance_l2: float
    separation_score: float       # 1 - cosine, clamped to [0, 1]
    ambiguity_band: str           # none | moderate | high | critical
    boundary_examples: list[dict]


def _ambiguity_band(separation: float) -> str:
    if separation > 0.7:
        return "none"
    if separation > 0.4:
        return "moderate"
    if separation > 0.2:
        return "high"
    return "critical"


def compare_centroids(
    *,
    class_a_name: str,
    class_a_centroid: np.ndarray,
    class_a_examples: list[str],
    class_a_example_embeddings: np.ndarray | None,
    class_b_name: str,
    class_b_centroid: np.ndarray,
    class_b_examples: list[str],
    class_b_example_embeddings: np.ndarray | None,
    boundary_threshold: float = 0.1,
    max_boundary_examples: int = 10,
) -> CompareResult:
    """Compute centroid separation + boundary examples between two classes."""
    cos = _cosine(class_a_centroid, class_b_centroid)
    l2 = float(np.linalg.norm(class_a_centroid - class_b_centroid))
    sep = max(0.0, min(1.0, 1.0 - cos))
    band = _ambiguity_band(sep)

    boundary: list[dict] = []
    # For each class' examples, score them against BOTH centroids.
    # Examples whose |score_a - score_b| < boundary_threshold are
    # boundary.
    def _scan(
        from_class: str,
        examples: list[str],
        embs: np.ndarray | None,
    ) -> None:
        if embs is None:
            return
        for i, ex in enumerate(examples):
            sa = _cosine(embs[i], class_a_centroid)
            sb = _cosine(embs[i], class_b_centroid)
            if abs(sa - sb) < boundary_threshold:
                boundary.append({
                    "example": ex,
                    "from_class": from_class,
                    "score_a": float(sa),
                    "score_b": float(sb),
                })

    _scan(class_a_name, class_a_examples, class_a_example_embeddings)
    _scan(class_b_name, class_b_examples, class_b_example_embeddings)
    boundary.sort(key=lambda x: abs(x["score_a"] - x["score_b"]))
    boundary = boundary[:max_boundary_examples]

    return CompareResult(
        centroid_cosine=cos,
        centroid_distance_l2=l2,
        separation_score=sep,
        ambiguity_band=band,
        boundary_examples=boundary,
    )


# ─────────────────────────────────────────────────────────────────────
# Threshold sensitivity (binary search)
# ─────────────────────────────────────────────────────────────────────


def threshold_sensitivity_range(
    *,
    score: float,
    runner_up_score: float,
) -> tuple[float, float]:
    """Return [low, high] threshold range where the top-1 decision is
    unchanged.

    Decision = "score >= threshold AND score > runner_up_score".
    The top-1 winner stays the same as long as `threshold <= score`.
    The classification flips below `runner_up_score` only if we drop
    below the runner-up's score (where another class would be predicted).
    """
    low = max(0.0, runner_up_score)
    high = max(low, score)
    return (round(low, 4), round(high, 4))
