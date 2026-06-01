"""Segment / HTE subgroup analysis."""

from __future__ import annotations

from datetime import datetime, timezone

from efe_rollout.analysis.hte import subgroup_analysis
from efe_rollout.records import Outcome


def _outcome(
    value: float, variant: str, *, country: str, user: str,
) -> Outcome:
    return Outcome(
        outcome_id=f"o-{user}",
        experiment_id="e",
        tenant_id="acme",
        user_id=user,
        variant_id=variant,
        metric_name="ctr",
        metric_value=value,
        observed_at=datetime(2026, 4, 20, tzinfo=timezone.utc),
        covariate={"country": country},
    )


def test_subgroup_analysis_detects_country_hte():
    outs = []
    # FR segment: treatment much better
    for i in range(50):
        outs.append(_outcome(0.3, "ctrl", country="FR", user=f"ctrl-fr-{i}"))
        outs.append(_outcome(0.8, "trt", country="FR", user=f"trt-fr-{i}"))
    # US segment: no effect
    for i in range(50):
        outs.append(_outcome(0.4, "ctrl", country="US", user=f"ctrl-us-{i}"))
        outs.append(_outcome(0.4, "trt", country="US", user=f"trt-us-{i}"))
    result = subgroup_analysis(
        outs,
        segment_key="country",
        control_variant_id="ctrl",
        treatment_variant_id="trt",
    )
    assert set(result.keys()) == {"FR", "US"}
    assert result["FR"].significant
    assert not result["US"].significant


def test_subgroup_analysis_empty_returns_empty():
    result = subgroup_analysis(
        [], segment_key="country",
        control_variant_id="a", treatment_variant_id="b",
    )
    assert result == {}
