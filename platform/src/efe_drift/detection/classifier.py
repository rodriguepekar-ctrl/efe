"""Classifier-based drift detector — multivariate via logistic regression.

Principle (Rabanser et al. 2019) : train a binary classifier to
distinguish **baseline** samples (label 0) from **current** samples
(label 1). If the classifier's test-set AUC is significantly above
0.5, the two distributions differ.

We implement a self-contained logistic-regression solver with batch
gradient descent — no scikit-learn dependency, completely
reproducible under a seeded RNG.

Score reporting :

* ``score`` = ``2 · |auc - 0.5|`` — normalised to ``[0, 1]`` so the
  severity ladder is direction-agnostic (overfit in either direction
  is drift).
* ``details.auc`` = raw AUC.
* ``details.n_train / n_test`` = split sizes.

Features : numeric, binary, or embedding. Categorical is handled by
the caller via one-hot encoding before passing to
:meth:`detect_from_raw`.
"""

from __future__ import annotations

import math
import random
from collections.abc import Sequence
from dataclasses import dataclass, field

from efe_drift.clock import Clock, SystemClock
from efe_drift.detection.base import (
    DetectionContext,
    DriftMethod,
    DriftResult,
)
from efe_drift.detection.severity import (
    DEFAULT_CLASSIFIER_AUC_DIST,
    SeverityThresholds,
)
from efe_drift.detection.stats import rank_auc
from efe_drift.ids import new_uuid
from efe_drift.records import FeatureSnapshot

Vector = Sequence[float]


# ---------------------------------------------------------------------------
# Tiny logistic regression
# ---------------------------------------------------------------------------


def _sigmoid(z: float) -> float:
    # Numerically stable sigmoid.
    if z >= 0:
        ez = math.exp(-z)
        return 1.0 / (1.0 + ez)
    ez = math.exp(z)
    return ez / (1.0 + ez)


@dataclass(slots=True)
class _LogRegModel:
    weights: list[float]
    bias: float

    def predict_proba(self, x: Vector) -> float:
        if len(x) != len(self.weights):
            raise ValueError(
                f"feature dim mismatch: model={len(self.weights)}, "
                f"input={len(x)}",
            )
        z = self.bias + sum(w * v for w, v in zip(self.weights, x, strict=True))
        return _sigmoid(z)


def _train_logreg(
    x: Sequence[Vector],
    y: Sequence[int],
    *,
    max_iter: int = 200,
    learning_rate: float = 0.1,
    l2: float = 1e-3,
    seed: int = 0,
) -> _LogRegModel:
    """Batch-gradient logistic regression with L2 regularisation.

    Early-stop when the loss improvement between iterations drops
    below ``1e-6`` — keeps the runtime bounded for easy-to-classify
    inputs.
    """
    if not x:
        return _LogRegModel(weights=[], bias=0.0)
    n = len(x)
    dim = len(x[0])
    weights = [0.0] * dim
    bias = 0.0
    prev_loss = math.inf
    for _ in range(max_iter):
        total_grad_w = [0.0] * dim
        total_grad_b = 0.0
        loss = 0.0
        for i in range(n):
            z = bias + sum(
                weights[j] * x[i][j] for j in range(dim)
            )
            p = _sigmoid(z)
            err = p - y[i]
            # Cross-entropy loss (clamped).
            eps = 1e-12
            if y[i] == 1:
                loss -= math.log(max(p, eps))
            else:
                loss -= math.log(max(1.0 - p, eps))
            for j in range(dim):
                total_grad_w[j] += err * x[i][j]
            total_grad_b += err
        # L2 contribution.
        for j in range(dim):
            total_grad_w[j] += l2 * weights[j]
            loss += 0.5 * l2 * weights[j] * weights[j]
        loss /= n
        # Update.
        for j in range(dim):
            weights[j] -= learning_rate * total_grad_w[j] / n
        bias -= learning_rate * total_grad_b / n
        if abs(prev_loss - loss) < 1e-6:
            break
        prev_loss = loss
    _ = seed    # reserved for future SGD variants
    return _LogRegModel(weights=weights, bias=bias)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _stratified_split(
    x: Sequence[Vector],
    y: Sequence[int],
    *,
    test_ratio: float,
    rng: random.Random,
) -> tuple[
    list[Vector], list[int], list[Vector], list[int],
]:
    """Stratified random split that preserves class proportions."""
    xs_pos = [xi for xi, yi in zip(x, y, strict=True) if yi == 1]
    xs_neg = [xi for xi, yi in zip(x, y, strict=True) if yi == 0]
    rng.shuffle(xs_pos)
    rng.shuffle(xs_neg)
    n_pos_test = round(test_ratio * len(xs_pos))
    n_neg_test = round(test_ratio * len(xs_neg))
    x_test = xs_pos[:n_pos_test] + xs_neg[:n_neg_test]
    y_test = [1] * n_pos_test + [0] * n_neg_test
    x_train = xs_pos[n_pos_test:] + xs_neg[n_neg_test:]
    y_train = [1] * (len(xs_pos) - n_pos_test) + [0] * (len(xs_neg) - n_neg_test)
    return x_train, y_train, x_test, y_test


def classifier_drift_score(
    baseline_vectors: Sequence[Vector],
    current_vectors: Sequence[Vector],
    *,
    test_ratio: float = 0.2,
    max_iter: int = 200,
    learning_rate: float = 0.1,
    l2: float = 1e-3,
    seed: int = 20260420,
) -> tuple[float, float, int, int]:
    """Return ``(auc, drift_score, n_train, n_test)``.

    ``drift_score = 2 · |auc - 0.5|`` — ranges from 0 (cannot
    distinguish) to 1 (perfect separation).
    """
    if len(baseline_vectors) < 2 or len(current_vectors) < 2:
        return float("nan"), float("nan"), 0, 0
    if len(baseline_vectors[0]) != len(current_vectors[0]):
        raise ValueError("baseline and current vectors must have same dim")
    x_all: list[Vector] = list(baseline_vectors) + list(current_vectors)
    y_all: list[int] = (
        [0] * len(baseline_vectors) + [1] * len(current_vectors)
    )
    rng = random.Random(seed)
    x_train, y_train, x_test, y_test = _stratified_split(
        x_all, y_all, test_ratio=test_ratio, rng=rng,
    )
    if not x_test or not x_train:
        return float("nan"), float("nan"), len(x_train), len(x_test)
    model = _train_logreg(
        x_train, y_train,
        max_iter=max_iter, learning_rate=learning_rate, l2=l2,
    )
    scores = [model.predict_proba(v) for v in x_test]
    if 0 in y_test and 1 in y_test:
        auc = rank_auc(scores, y_test)
    else:
        return float("nan"), float("nan"), len(x_train), len(x_test)
    drift_score = 2.0 * abs(auc - 0.5)
    return auc, drift_score, len(x_train), len(x_test)


# ---------------------------------------------------------------------------
# ClassifierDriftDetector
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class ClassifierDriftDetector:
    """Multivariate drift detector based on a trainable classifier."""

    thresholds: SeverityThresholds = DEFAULT_CLASSIFIER_AUC_DIST
    test_ratio: float = 0.2
    max_iter: int = 200
    learning_rate: float = 0.1
    l2: float = 1e-3
    seed: int = 20260420
    _method: DriftMethod = "classifier_based"
    _clock: Clock = field(default_factory=SystemClock)

    @property
    def method(self) -> DriftMethod:
        return self._method

    def supported_kinds(self) -> tuple[str, ...]:
        return ("numeric", "binary", "embedding")

    def detect_from_raw(
        self,
        ctx: DetectionContext,
        current_vectors: Sequence[Vector],
        baseline_vectors: Sequence[Vector],
    ) -> DriftResult:
        auc, score, n_train, n_test = classifier_drift_score(
            baseline_vectors, current_vectors,
            test_ratio=self.test_ratio,
            max_iter=self.max_iter,
            learning_rate=self.learning_rate,
            l2=self.l2,
            seed=self.seed,
        )
        thresholds = ctx.thresholds or self.thresholds
        severity = (
            "none" if math.isnan(score)
            else thresholds.classify(score)
        )
        return DriftResult(
            evaluation_id=new_uuid(),
            tenant_id=ctx.tenant_id,
            task_id=ctx.task_id,
            feature_id=ctx.feature.feature_id,
            feature_name=ctx.feature.name,
            baseline_id=ctx.baseline.baseline_id,
            window_start=ctx.window_start,
            window_end=ctx.window_end,
            method=self._method,
            score=score,
            severity=severity,
            computed_at=self._clock.now(),
            multi_scale_window=ctx.multi_scale_window,
            details={
                "auc": auc,
                "n_train": n_train,
                "n_test": n_test,
            },
            sample_count_current=len(current_vectors),
            sample_count_baseline=len(baseline_vectors),
        )

    def detect_from_snapshots(
        self,
        ctx: DetectionContext,
        current: tuple[FeatureSnapshot, ...],
    ) -> DriftResult:
        # The snapshot-only path cannot reconstruct raw multivariate
        # samples — callers must use detect_from_raw. We return an
        # informational NaN result so the evaluator can skip cleanly.
        return DriftResult(
            evaluation_id=new_uuid(),
            tenant_id=ctx.tenant_id,
            task_id=ctx.task_id,
            feature_id=ctx.feature.feature_id,
            feature_name=ctx.feature.name,
            baseline_id=ctx.baseline.baseline_id,
            window_start=ctx.window_start,
            window_end=ctx.window_end,
            method=self._method,
            score=float("nan"),
            severity="none",
            computed_at=self._clock.now(),
            multi_scale_window=ctx.multi_scale_window,
            details={"mode": "needs_raw_vectors"},
            sample_count_current=sum(s.sample_count for s in current),
            sample_count_baseline=ctx.baseline.spec.sample_count,
        )


__all__ = [
    "ClassifierDriftDetector",
    "classifier_drift_score",
]
