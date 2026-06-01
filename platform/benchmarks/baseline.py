"""Naïve **no-swarm** baseline classifier.

Decision rule
-------------

For a query embedding ``q`` and, per class ``c``, a set of anchor
embeddings ``A_c``, the baseline computes::

    score(c) = (1 / |A_c|) * Σ_{a ∈ A_c} cos(q, a)

and returns the class with the highest score. Confidence is the raw
(cosine-mean) score clamped to ``[0, 1]`` via a softmax across classes.

This is the obvious, honest, zero-dependency reference implementation
every ML engineer would reach for first. It has **no** consensus, **no**
multi-agent voting, **no** verdict reasoning.
"""

from __future__ import annotations

import time
from dataclasses import dataclass

import numpy as np

from benchmarks.dataset import Dataset, Query


@dataclass(frozen=True, slots=True)
class BaselinePrediction:
    """One inference result from the baseline."""

    qid: str
    predicted: str
    confidence: float
    #: Latency in microseconds (single-query inference).
    latency_us: float
    #: Full softmax distribution across all classes — useful for
    #: calibration and confusion-matrix analysis.
    distribution: dict[str, float]


class NaiveCosineBaseline:
    """Classifier: ``argmax_c mean_a cos(q, a_c)``."""

    def __init__(self, dataset: Dataset) -> None:
        self._classes: list[str] = sorted(dataset.anchors.keys())
        # Stack anchors as shape (n_classes, anchors_per_class, dim).
        self._anchors_mat = np.stack(
            [dataset.anchors[c] for c in self._classes], axis=0
        )

    def classify(self, query: Query) -> BaselinePrediction:
        t0 = time.perf_counter()
        q = query.embedding  # shape (dim,)
        # Assume anchors are already L2-normalised (see dataset.py).
        # cosine = dot product.  shape (n_classes, anchors_per_class)
        sim = self._anchors_mat @ q  # (n_classes, anchors_per_class)
        per_class_mean = sim.mean(axis=1)  # (n_classes,)
        softmax = _softmax(per_class_mean * 10.0)  # temperature 0.1
        best_idx = int(np.argmax(per_class_mean))
        latency_us = (time.perf_counter() - t0) * 1e6
        distribution = {c: float(s) for c, s in zip(self._classes, softmax)}
        return BaselinePrediction(
            qid=query.qid,
            predicted=self._classes[best_idx],
            confidence=float(softmax[best_idx]),
            latency_us=latency_us,
            distribution=distribution,
        )


def _softmax(x: np.ndarray) -> np.ndarray:
    e = np.exp(x - x.max())
    return e / e.sum()


__all__ = ["BaselinePrediction", "NaiveCosineBaseline"]
