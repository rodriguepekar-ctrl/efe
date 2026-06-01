"""Metrics layer — per-sample scores, bundles, catalogue, stats tests.

Public surface:

    SampleScores     — per-sample outputs for one metric (numpy array)
    MetricBundle     — one metric's full observation (value + CI + etc.)
    MetricsBundle    — map metric_name → MetricBundle, plus per-group
                       splits for fairness
    MetricRole       — ``primary`` / ``secondary`` / ``budget`` / ``fairness``
    MetricDefinition — catalogue entry describing a metric
    CATALOG          — registered metric definitions
    register_metric  — extension point
"""

from __future__ import annotations

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

__all__ = [
    "CATALOG",
    "MetricBundle",
    "MetricDefinition",
    "MetricRole",
    "MetricsBundle",
    "SampleScores",
    "register_metric",
    "resolve_metric",
]
