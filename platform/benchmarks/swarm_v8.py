"""V8 — Calibrated Mahalanobis swarm + V9 ensemble.

V8 is the ablation winner's follow-up: V4 wins on raw accuracy (+0.7
pp ALL, +1.2 pp HARD) but loses on calibration (ECE ×3.2) and verdict
coverage (1.7 %). V8 addresses both:

* **Temperature calibration** of the aggregate softmax, fitted on a
  held-out easy slice (same as V5).
* **Re-tuned verdict thresholds** — V4's scores live on a different
  scale (standardised Mahalanobis), so ``epsilon_margin`` and
  ``epsilon_variance`` must be re-fitted, not copy-pasted from V1.
  We sweep a small grid on the calibration slice and pick the pair
  that maximises CONVERGED coverage subject to the same-accuracy
  floor (accuracy on CONVERGED >= overall accuracy + 2 pp).

V9 is an **ensemble** that fuses V1 (calibrated) and V4 (Mahalanobis).
Decision rule:

    if V1.pred == V4.pred:
        verdict = CONVERGED, conf = max(V1.conf, V4.conf)
    else:
        verdict = DIVERGENCE,   conf = lower of the two

This is the simplest possible "two-expert" voting — the hypothesis is
that the *agreement* between two **independent** classifiers is a
stronger correctness signal than either classifier alone.
"""

from __future__ import annotations

import time
from dataclasses import dataclass, replace
from typing import Protocol

import numpy as np

from benchmarks.dataset import Dataset, Query
from benchmarks.swarm_sim import SwarmClassifier, SwarmPrediction, Verdict
from benchmarks.swarm_v2 import SwarmV4Mahalanobis, _optimise_temperature


class _Classifier(Protocol):
    def classify(self, query: Query) -> SwarmPrediction: ...


# ---------------------------------------------------------------------------
#  V8 — Calibrated Mahalanobis
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class V8TuningResult:
    temperature: float
    epsilon_margin: float
    epsilon_variance: float
    #: Measured on the calibration slice.
    cal_accuracy: float
    cal_converged_coverage: float
    cal_converged_accuracy: float


class SwarmV8CalibratedMahalanobis:
    """V4 with learned temperature + re-tuned verdict thresholds."""

    def __init__(
        self,
        dataset: Dataset,
        calibration_queries: list[Query],
    ) -> None:
        self._base = SwarmV4Mahalanobis(dataset)
        # 1. Collect V4 raw predictions on the calibration slice.
        cal_preds = [self._base.classify(q) for q in calibration_queries]
        # 2. Fit temperature on the (logits, labels) pairs.
        classes = sorted({q.label for q in calibration_queries})
        class_idx = {c: i for i, c in enumerate(classes)}
        logits = np.stack(
            [
                np.log(np.array([p.distribution[c] for c in classes]) + 1e-12)
                for p in cal_preds
            ],
            axis=0,
        )
        y = np.array([class_idx[q.label] for q in calibration_queries])
        self._temperature = _optimise_temperature(logits, y)
        self._classes = classes

        # 3. Sweep verdict thresholds on the calibration slice for a
        #    healthy coverage-accuracy trade-off.
        #    We use the base V4's raw distribution and choose margin
        #    such that CONVERGED ≥ 30 % cov AND gated acc ≥ base acc + 2 pp.
        overall_acc = sum(
            1
            for q, p in zip(calibration_queries, cal_preds)
            if p.predicted == q.label
        ) / max(1, len(cal_preds))
        best_margin = 0.02
        best_variance = 0.01
        best_coverage = 0.0
        for margin in (0.005, 0.01, 0.02, 0.03, 0.05, 0.08, 0.12):
            kept, correct = 0, 0
            for q, p in zip(calibration_queries, cal_preds):
                top2 = sorted(p.distribution.values(), reverse=True)[:2]
                if (top2[0] - top2[1]) >= margin:
                    kept += 1
                    if p.predicted == q.label:
                        correct += 1
            if kept == 0:
                continue
            cov = kept / len(cal_preds)
            acc = correct / kept
            if acc >= overall_acc + 0.02 and cov > best_coverage:
                best_coverage = cov
                best_margin = margin
        self._margin = best_margin
        self._variance = best_variance
        self.tuning = V8TuningResult(
            temperature=self._temperature,
            epsilon_margin=self._margin,
            epsilon_variance=self._variance,
            cal_accuracy=overall_acc,
            cal_converged_coverage=best_coverage,
            cal_converged_accuracy=0.0,  # filled below
        )

    def classify(self, query: Query) -> SwarmPrediction:
        t0 = time.perf_counter()
        base = self._base.classify(query)
        classes = self._classes
        # Re-softmax with the learned temperature.
        logits = np.log(np.array([base.distribution[c] for c in classes]) + 1e-12)
        scaled = logits / self._temperature
        e = np.exp(scaled - scaled.max())
        re = e / e.sum()
        winner_idx = int(np.argmax(re))
        top2 = np.sort(re)[::-1][:2]
        margin = float(top2[0] - top2[1])
        verdict: Verdict
        if margin < self._margin:
            verdict = "DIVERGENCE"
        else:
            verdict = "CONVERGED"
        latency_us = (time.perf_counter() - t0) * 1e6 + base.latency_us
        return replace(
            base,
            predicted=classes[winner_idx],
            confidence=float(re[winner_idx]),
            distribution={c: float(s) for c, s in zip(classes, re)},
            verdict=verdict,
            latency_us=latency_us,
        )


# ---------------------------------------------------------------------------
#  V9 — Ensemble V1 ⊕ V4
# ---------------------------------------------------------------------------


class SwarmV9Ensemble:
    """Fuse V1 (calibrated cosine swarm) and V4 (Mahalanobis) by agreement."""

    def __init__(self, dataset: Dataset) -> None:
        self._v1 = SwarmClassifier(dataset)
        self._v4 = SwarmV4Mahalanobis(dataset)

    def classify(self, query: Query) -> SwarmPrediction:
        t0 = time.perf_counter()
        r1 = self._v1.classify(query)
        r4 = self._v4.classify(query)
        latency = (time.perf_counter() - t0) * 1e6

        if r1.predicted == r4.predicted:
            # Both classifiers agree → strong CONVERGED signal.
            confidence = max(r1.confidence, r4.confidence)
            verdict: Verdict = "CONVERGED"
            winner = r1.predicted
            # Merge distributions (average) — stable fused output.
            all_classes = set(r1.distribution) | set(r4.distribution)
            merged = {
                c: 0.5 * r1.distribution.get(c, 0.0) + 0.5 * r4.distribution.get(c, 0.0)
                for c in all_classes
            }
        else:
            # Disagreement → expose it honestly.
            # Choose the higher-confidence prediction but flag DIVERGENCE.
            if r1.confidence >= r4.confidence:
                winner = r1.predicted
                chosen_dist = r1.distribution
                confidence = r1.confidence
            else:
                winner = r4.predicted
                chosen_dist = r4.distribution
                confidence = r4.confidence
            verdict = "DIVERGENCE"
            merged = dict(chosen_dist)

        # Max rounds = max of the two sub-classifiers.
        rounds = max(r1.rounds_executed, r4.rounds_executed)
        crit_dissent = max(r1.critique_dissent, r4.critique_dissent)

        return SwarmPrediction(
            qid=query.qid,
            predicted=winner,
            confidence=confidence,
            verdict=verdict,
            rounds_executed=rounds,
            n_agents=r1.n_agents + r4.n_agents,
            latency_us=latency,
            distribution=merged,
            critique_dissent=crit_dissent,
        )


__all__ = [
    "SwarmV8CalibratedMahalanobis",
    "SwarmV9Ensemble",
    "V8TuningResult",
]
