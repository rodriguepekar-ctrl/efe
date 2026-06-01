"""Scoring primitives for shadow diff analysis.

Pure-python implementations (no numpy) so unit tests stay light. The
production pipeline can optionally swap in a vectorised version, but
the semantics are fixed here.
"""

from __future__ import annotations

import math
from collections.abc import Iterable, Sequence
from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class ShadowScore:
    cosine: float
    bleu4: float
    latency_delta_ms: float
    n_samples: int


def cosine_similarity(a: Sequence[float], b: Sequence[float]) -> float:
    if len(a) != len(b):
        raise ValueError(
            f"cosine_similarity: length mismatch ({len(a)} vs {len(b)})",
        )
    if not a:
        return 0.0
    dot = sum(x * y for x, y in zip(a, b, strict=True))
    na = math.sqrt(sum(x * x for x in a))
    nb = math.sqrt(sum(y * y for y in b))
    if na == 0.0 or nb == 0.0:
        return 0.0
    return dot / (na * nb)


def bleu4_on_tokens(
    reference: Sequence[str], candidate: Sequence[str],
) -> float:
    """Corpus-level BLEU-4 on pre-tokenised sequences.

    Implementation follows the standard definition (geometric mean of
    modified n-gram precision for n=1..4, with brevity penalty).
    Returns 0.0 on empty inputs rather than raising — shadow scoring
    should never crash the router.
    """
    if not reference or not candidate:
        return 0.0
    weights = (0.25, 0.25, 0.25, 0.25)
    precisions: list[float] = []
    for n in range(1, 5):
        ref_ngrams = _ngram_counts(reference, n)
        cand_ngrams = _ngram_counts(candidate, n)
        if not cand_ngrams:
            precisions.append(0.0)
            continue
        overlap = 0
        total = 0
        for ngram, count in cand_ngrams.items():
            clipped = min(count, ref_ngrams.get(ngram, 0))
            overlap += clipped
            total += count
        precisions.append(overlap / total if total > 0 else 0.0)
    if min(precisions) == 0.0:
        # Smoothing method 1: replace 0 with 1/(2*N).
        precisions = [
            p if p > 0 else 1.0 / (2.0 * max(1, len(candidate)))
            for p in precisions
        ]
    log_p = sum(w * math.log(p) for w, p in zip(weights, precisions, strict=True))
    geom = math.exp(log_p)
    # Brevity penalty.
    bp = 1.0
    if len(candidate) < len(reference):
        bp = math.exp(1.0 - len(reference) / max(1, len(candidate)))
    return bp * geom


def latency_delta_ms(
    baseline_ms: Iterable[float], candidate_ms: Iterable[float],
) -> float:
    b = list(baseline_ms)
    c = list(candidate_ms)
    if not b or not c:
        return 0.0
    return (sum(c) / len(c)) - (sum(b) / len(b))


def _ngram_counts(seq: Sequence[str], n: int) -> dict[tuple[str, ...], int]:
    counts: dict[tuple[str, ...], int] = {}
    for i in range(len(seq) - n + 1):
        ng = tuple(seq[i:i + n])
        counts[ng] = counts.get(ng, 0) + 1
    return counts


__all__ = [
    "ShadowScore",
    "bleu4_on_tokens",
    "cosine_similarity",
    "latency_delta_ms",
]
