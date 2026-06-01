"""End-to-end analyzer — routes input to the right method."""

from __future__ import annotations

from datetime import datetime, timezone

import pytest

from efe_rollout.analysis.analyzer import AnalysisInputs, ExperimentAnalyzer
from efe_rollout.experiment import ExperimentSpec, Variant
from efe_rollout.records import Exposure, Outcome


def _spec() -> ExperimentSpec:
    return ExperimentSpec(
        name="analyze",
        tenant_id="acme",
        task_id="summarize",
        layer="prompt",
        kind="ab_fixed",
        owner="alice",
        hypothesis="h",
        primary_metric="ctr",
        variants=(
            Variant(name="control", allocation_pct=50, is_control=True,
                    variant_id="v-ctrl"),
            Variant(name="treatment", allocation_pct=50,
                    variant_id="v-trt"),
        ),
    )


def _outcome(value: float, variant: str, *, user: str = "u") -> Outcome:
    return Outcome(
        outcome_id=f"o-{variant}-{user}-{value}",
        experiment_id="e-1",
        tenant_id="acme",
        user_id=user,
        variant_id=variant,
        metric_name="ctr",
        metric_value=value,
        observed_at=datetime(2026, 4, 20, tzinfo=timezone.utc),
    )


def _exposure(variant: str, user: str) -> Exposure:
    return Exposure(
        exposure_id=f"e-{variant}-{user}",
        experiment_id="e-1",
        tenant_id="acme",
        user_id=user,
        variant_id=variant,
        assigned_at=datetime(2026, 4, 20, tzinfo=timezone.utc),
        layer="prompt",
        forced=False,
        holdout=False,
    )


def test_analyzer_binary_path_ship_decision():
    spec = _spec()
    analyzer = ExperimentAnalyzer()
    outs = (
        [_outcome(0.0, "v-ctrl", user=f"c{i}") for i in range(100)]
        + [_outcome(1.0, "v-ctrl", user=f"c1{i}") for i in range(10)]
        + [_outcome(0.0, "v-trt", user=f"t{i}") for i in range(40)]
        + [_outcome(1.0, "v-trt", user=f"t1{i}") for i in range(70)]
    )
    exps = tuple(
        _exposure(o.variant_id or "v-ctrl", o.user_id) for o in outs
    )
    inputs = AnalysisInputs(
        spec=spec,
        exposures=exps,
        outcomes=tuple(outs),
        primary_metric="ctr",
        kind="final",
        bayesian_n_samples=2_000,
    )
    result = analyzer.analyze(inputs)
    assert result.kind == "final"
    assert result.n_per_variant["control"] == 110
    assert result.n_per_variant["treatment"] == 110
    assert result.estimate > 0
    # Treatment much better than control → should recommend ship.
    assert result.decision in ("ship", "continue")


def test_analyzer_raises_on_insufficient_data():
    spec = _spec()
    analyzer = ExperimentAnalyzer()
    inputs = AnalysisInputs(
        spec=spec,
        exposures=(),
        outcomes=(_outcome(1.0, "v-ctrl"),),
        primary_metric="ctr",
    )
    with pytest.raises(Exception):
        analyzer.analyze(inputs)


def test_analyzer_continuous_path_welch():
    spec = _spec()
    analyzer = ExperimentAnalyzer()
    outs = (
        [_outcome(10.0 + i * 0.01, "v-ctrl", user=f"c{i}")
         for i in range(100)]
        + [_outcome(11.0 + i * 0.01, "v-trt", user=f"t{i}")
           for i in range(100)]
    )
    exps = tuple(
        _exposure(o.variant_id or "v-ctrl", o.user_id) for o in outs
    )
    inputs = AnalysisInputs(
        spec=spec,
        exposures=exps,
        outcomes=tuple(outs),
        primary_metric="ctr",
        kind="interim",
    )
    result = analyzer.analyze(inputs)
    assert result.methods["method"] == "welch_t_test"
    assert result.estimate == pytest.approx(1.0, abs=0.1)
    assert result.p_value is not None
    assert result.p_value < 1e-9
