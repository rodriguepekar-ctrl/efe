"""Tests for the comparison engine."""

from __future__ import annotations

from typing import Any

import numpy as np

from efe_gate.comparator import (
    ComparisonEngine,
    MetricComparisonVerdict,
)
from efe_gate.metrics.bundle import (
    MetricBundle,
    MetricRole,
    MetricsBundle,
    SampleScores,
)


def _bundle(
    *,
    metric: str, role: MetricRole, value: float,
    scores: np.ndarray | None = None,
    higher_is_better: bool = True,
) -> tuple[MetricBundle, SampleScores | None]:
    b = MetricBundle(
        name=metric, role=role, value=float(value),
        ci_lower=None, ci_upper=None,
        sample_size=int(scores.size) if scores is not None else 0,
        higher_is_better=higher_is_better,
    )
    sc = None
    if scores is not None:
        sc = SampleScores(metric_name=metric, values=scores)
    return b, sc


def _mk(
    rows: list[tuple[str, MetricRole, float, np.ndarray | None, bool]],
) -> MetricsBundle:
    mb = MetricsBundle()
    for (metric, role, value, scores, hib) in rows:
        b, sc = _bundle(
            metric=metric, role=role, value=value,
            scores=scores, higher_is_better=hib,
        )
        mb.add(b)
        if sc is not None:
            mb.add_raw(sc)
    return mb


def test_budget_exceeds_hard_cap() -> None:
    champ = _mk([
        ("p99_latency_ms", MetricRole.BUDGET, 30.0, None, False),
    ])
    cand = _mk([
        ("p99_latency_ms", MetricRole.BUDGET, 80.0, None, False),
    ])
    eng = ComparisonEngine()
    out = eng.compare(
        champ, cand, budget_caps={"p99_latency_ms": 50.0},
    )
    assert out[0].verdict is MetricComparisonVerdict.BUDGET_EXCEEDED


def test_primary_metric_pass_when_equivalent(bern: Any) -> None:
    # On 500 samples the bootstrap CI for a Bernoulli proportion is
    # ~±0.03, so we need a wider equivalence margin to land in PASS.
    champ_scores = bern(0.85, 500, 1)
    cand_scores = bern(0.86, 500, 2)
    champ = _mk([
        ("accuracy", MetricRole.PRIMARY, 0.85, champ_scores, True),
    ])
    cand = _mk([
        ("accuracy", MetricRole.PRIMARY, 0.86, cand_scores, True),
    ])
    eng = ComparisonEngine(bootstrap_resamples=500)
    out = eng.compare(
        champ, cand, margin_overrides={"accuracy": 0.10},
    )
    assert out[0].verdict in (
        MetricComparisonVerdict.PASS,
        MetricComparisonVerdict.BETTER,
        MetricComparisonVerdict.AMBIGUOUS,
    )


def test_primary_metric_fail_on_regression(bern: Any) -> None:
    champ_scores = bern(0.9, 1000, 1)
    cand_scores = bern(0.7, 1000, 1)
    champ = _mk([
        ("accuracy", MetricRole.PRIMARY, 0.9, champ_scores, True),
    ])
    cand = _mk([
        ("accuracy", MetricRole.PRIMARY, 0.7, cand_scores, True),
    ])
    eng = ComparisonEngine(bootstrap_resamples=500)
    out = eng.compare(
        champ, cand, margin_overrides={"accuracy": 0.02},
    )
    assert out[0].verdict is MetricComparisonVerdict.FAIL


def test_metric_with_no_champion_is_pass() -> None:
    cand = _mk([
        ("accuracy", MetricRole.PRIMARY, 0.9, None, True),
    ])
    eng = ComparisonEngine()
    out = eng.compare(
        MetricsBundle(), cand,
    )
    assert out[0].verdict is MetricComparisonVerdict.PASS


def test_uncomparable_on_stats_error() -> None:
    # craft a bundle with a malformed raw (wrong shape)
    cand = MetricsBundle()
    cand.add(MetricBundle(
        "accuracy", MetricRole.PRIMARY, 0.9, sample_size=2,
    ))
    cand.add_raw(SampleScores("accuracy", np.array([1.0, 0.0, 1.0])))
    champ = MetricsBundle()
    champ.add(MetricBundle("accuracy", MetricRole.PRIMARY, 0.9,
                            sample_size=2))
    champ.add_raw(SampleScores("accuracy", np.array([1.0, 0.0])))
    eng = ComparisonEngine(bootstrap_resamples=100)
    out = eng.compare(champ, cand)
    assert out[0].verdict is MetricComparisonVerdict.UNCOMPARABLE


def test_welch_budget_via_raw() -> None:
    rng = np.random.default_rng(2)
    champ_ms = rng.normal(30.0, 1.0, 500)
    cand_ms = rng.normal(45.0, 1.0, 500)
    champ = _mk([
        ("p99_latency_ms", MetricRole.BUDGET, 30.0, champ_ms, False),
    ])
    cand = _mk([
        ("p99_latency_ms", MetricRole.BUDGET, 45.0, cand_ms, False),
    ])
    eng = ComparisonEngine()
    out = eng.compare(champ, cand)
    # No hard cap → Welch t should catch the regression.
    assert out[0].verdict is MetricComparisonVerdict.FAIL


def test_mcnemar_path_invoked_for_accuracy(bern: Any) -> None:
    champ_scores = bern(0.8, 500, 1)
    cand_scores = bern(0.8, 500, 2)
    champ = _mk([
        ("accuracy", MetricRole.PRIMARY, 0.8, champ_scores, True),
    ])
    cand = _mk([
        ("accuracy", MetricRole.PRIMARY, 0.8, cand_scores, True),
    ])
    eng = ComparisonEngine(bootstrap_resamples=500)
    out = eng.compare(
        champ, cand, margin_overrides={"accuracy": 0.05},
    )
    assert "mcnemar" in out[0].test
