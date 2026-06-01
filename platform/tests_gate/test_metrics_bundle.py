"""Tests for MetricsBundle + catalog."""

from __future__ import annotations

import numpy as np
import pytest

from efe_gate.errors import UnknownMetric
from efe_gate.metrics.bundle import (
    MetricBundle,
    MetricRole,
    MetricsBundle,
    SampleScores,
)
from efe_gate.metrics.catalog import (
    CATALOG,
    MetricDefinition,
    register_metric,
    resolve_metric,
)


def test_sample_scores_mean() -> None:
    s = SampleScores(metric_name="m", values=np.array([1, 0, 1, 1]))
    assert s.mean() == pytest.approx(0.75)
    assert s.n == 4


def test_sample_scores_group_mask() -> None:
    s = SampleScores(
        metric_name="m",
        values=np.array([1, 0, 1, 0]),
        groups=np.array(["a", "a", "b", "b"]),
    )
    sub = s.with_group_mask("a")
    assert sub.n == 2
    assert list(s.group_keys()) == ["a", "b"]


def test_sample_scores_no_groups() -> None:
    s = SampleScores(metric_name="m", values=np.array([1, 0]))
    with pytest.raises(ValueError):
        s.with_group_mask("a")


def test_sample_scores_shape_invariant() -> None:
    with pytest.raises(ValueError):
        SampleScores(metric_name="m", values=np.ones((3, 4)))


def test_sample_scores_group_len_mismatch() -> None:
    with pytest.raises(ValueError):
        SampleScores(
            metric_name="m",
            values=np.array([1.0, 0.0]),
            groups=np.array(["a"]),
        )


def test_metric_bundle_dict() -> None:
    b = MetricBundle(
        name="accuracy", role=MetricRole.PRIMARY,
        value=0.9, ci_lower=0.85, ci_upper=0.95, sample_size=100,
    )
    d = b.as_dict()
    assert d["role"] == "primary"
    assert d["value"] == 0.9


def test_metrics_bundle_add_and_filter() -> None:
    b = MetricsBundle()
    b.add(MetricBundle("accuracy", MetricRole.PRIMARY, 0.9))
    b.add(MetricBundle("p99_latency_ms", MetricRole.BUDGET, 40.0,
                        higher_is_better=False))
    assert b.has("accuracy")
    assert len(b.filter_role(MetricRole.PRIMARY)) == 1
    assert b.role("accuracy") is MetricRole.PRIMARY


def test_catalog_has_classif() -> None:
    md = resolve_metric("accuracy")
    assert md.role is MetricRole.PRIMARY
    assert "classif" in md.task_types


def test_catalog_unknown() -> None:
    with pytest.raises(UnknownMetric):
        resolve_metric("galaxy_brain")


def test_register_custom_metric() -> None:
    md = MetricDefinition(
        name="custom_score", role=MetricRole.SECONDARY,
        higher_is_better=True, default_test="bootstrap",
    )
    register_metric(md)
    assert resolve_metric("custom_score").name == "custom_score"
    # Cleanup so we don't pollute the global catalog.
    del CATALOG["custom_score"]


def test_register_empty_name_rejected() -> None:
    with pytest.raises(ValueError):
        register_metric(MetricDefinition(
            name="", role=MetricRole.PRIMARY,
            higher_is_better=True, default_test="bootstrap",
        ))
