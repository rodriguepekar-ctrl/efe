"""Tests for the decision engine + rule parser."""

from __future__ import annotations

from typing import Any

import numpy as np

from efe_gate.comparator import ComparisonEngine
from efe_gate.decision import (
    DecisionEngine,
    DecisionOutcome,
    FairnessRuleSpec,
    MetricRuleSpec,
    PromotionRule,
    ReviewPolicy,
)
from efe_gate.metrics.bundle import (
    MetricBundle,
    MetricRole,
    MetricsBundle,
    SampleScores,
)


def _add_metric(
    bundle: MetricsBundle, *, name: str, role: MetricRole,
    value: float, scores: np.ndarray | None = None,
    higher_is_better: bool = True,
) -> None:
    bundle.add(MetricBundle(
        name=name, role=role, value=value,
        sample_size=scores.size if scores is not None else 0,
        higher_is_better=higher_is_better,
    ))
    if scores is not None:
        bundle.add_raw(SampleScores(name, scores))


def _engine() -> DecisionEngine:
    return DecisionEngine(
        comparator=ComparisonEngine(bootstrap_resamples=500),
    )


def _rule(**kw: Any) -> PromotionRule:
    base = dict(
        primary=(MetricRuleSpec("accuracy", min_equivalence_margin=0.02),),
        budget=(MetricRuleSpec("p99_latency_ms", hard_cap=100.0),),
        fairness=FairnessRuleSpec(
            groups=(), metric="accuracy",
            max_per_group_drop=0.05, min_samples_per_group=10,
        ),
        review_policy=ReviewPolicy(
            require_dual_signoff=False, timeout_hours=1,
        ),
    )
    base.update(kw)
    return PromotionRule(**base)


def test_budget_reject(bern: Any) -> None:
    champ, cand = MetricsBundle(), MetricsBundle()
    scores = bern(0.9, 500, 1)
    _add_metric(champ, name="accuracy",
                 role=MetricRole.PRIMARY, value=0.9, scores=scores)
    _add_metric(cand, name="accuracy",
                 role=MetricRole.PRIMARY, value=0.9, scores=scores)
    _add_metric(cand, name="p99_latency_ms",
                 role=MetricRole.BUDGET, value=200.0,
                 higher_is_better=False)
    decision = _engine().decide(
        champion=champ, candidate=cand, rule=_rule(),
    )
    assert decision.outcome is DecisionOutcome.REJECT


def test_primary_regression_reject(bern: Any) -> None:
    champ, cand = MetricsBundle(), MetricsBundle()
    _add_metric(
        champ, name="accuracy", role=MetricRole.PRIMARY,
        value=0.9, scores=bern(0.9, 1000, 1),
    )
    _add_metric(
        cand, name="accuracy", role=MetricRole.PRIMARY,
        value=0.7, scores=bern(0.7, 1000, 1),
    )
    decision = _engine().decide(
        champion=champ, candidate=cand, rule=_rule(),
    )
    assert decision.outcome is DecisionOutcome.REJECT
    assert "primary" in decision.reason


def test_promote_on_clear_improvement(bern: Any) -> None:
    champ, cand = MetricsBundle(), MetricsBundle()
    _add_metric(
        champ, name="accuracy", role=MetricRole.PRIMARY,
        value=0.85, scores=bern(0.85, 2000, 1),
    )
    _add_metric(
        cand, name="accuracy", role=MetricRole.PRIMARY,
        value=0.90, scores=bern(0.90, 2000, 1),
    )
    decision = _engine().decide(
        champion=champ, candidate=cand, rule=_rule(),
    )
    assert decision.outcome is DecisionOutcome.PROMOTE


def test_missing_primary_goes_to_review(bern: Any) -> None:
    champ, cand = MetricsBundle(), MetricsBundle()
    _add_metric(
        champ, name="accuracy", role=MetricRole.PRIMARY,
        value=0.9, scores=bern(0.9, 500, 1),
    )
    # candidate lacks "accuracy"
    decision = _engine().decide(
        champion=champ, candidate=cand, rule=_rule(),
    )
    assert decision.outcome is DecisionOutcome.HUMAN_REVIEW


def test_fairness_violation_triggers_review(bern: Any) -> None:
    champ, cand = MetricsBundle(), MetricsBundle()
    _add_metric(
        champ, name="accuracy", role=MetricRole.PRIMARY,
        value=0.9, scores=bern(0.9, 2000, 1),
    )
    _add_metric(
        cand, name="accuracy", role=MetricRole.PRIMARY,
        value=0.90, scores=bern(0.90, 2000, 1),
    )
    # By-group: cand drops by 0.15 on group "fr".
    champ.add_group("fr", MetricBundle(
        "accuracy", MetricRole.PRIMARY, 0.9, sample_size=500,
    ))
    cand.add_group("fr", MetricBundle(
        "accuracy", MetricRole.PRIMARY, 0.75, sample_size=500,
    ))
    champ.add_group("en", MetricBundle(
        "accuracy", MetricRole.PRIMARY, 0.9, sample_size=500,
    ))
    cand.add_group("en", MetricBundle(
        "accuracy", MetricRole.PRIMARY, 0.91, sample_size=500,
    ))
    rule = _rule(
        fairness=FairnessRuleSpec(
            groups=("fr", "en"), metric="accuracy",
            max_per_group_drop=0.05, min_samples_per_group=10,
        ),
    )
    decision = _engine().decide(
        champion=champ, candidate=cand, rule=rule,
    )
    assert decision.outcome is DecisionOutcome.HUMAN_REVIEW
    assert any(g == "fr" for g, _d in decision.fairness_violations)


def test_rule_roundtrip() -> None:
    rule = _rule()
    d = rule.as_dict()
    rebuilt = PromotionRule.from_dict(d)
    assert rebuilt.primary_metrics() == ["accuracy"]
    assert rebuilt.budget_metrics() == ["p99_latency_ms"]


def test_rule_parse_rejects_garbage() -> None:
    import pytest
    from efe_gate.errors import RuleParseError

    with pytest.raises(RuleParseError):
        PromotionRule.from_dict({"primary": [{"no_metric_key": 1}]})
