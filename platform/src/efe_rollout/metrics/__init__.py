"""Exposure + outcome ingestion pipeline."""

from __future__ import annotations

from efe_rollout.metrics.exposure import (
    ExposureContext,
    ExposureRecorder,
    ExposureSink,
    InMemoryExposureSink,
)
from efe_rollout.metrics.outcome import (
    ExposureIndex,
    InMemoryOutcomeSink,
    OutcomeContext,
    OutcomeRecorder,
    OutcomeSink,
)

__all__ = [
    "ExposureContext",
    "ExposureIndex",
    "ExposureRecorder",
    "ExposureSink",
    "InMemoryExposureSink",
    "InMemoryOutcomeSink",
    "OutcomeContext",
    "OutcomeRecorder",
    "OutcomeSink",
]
