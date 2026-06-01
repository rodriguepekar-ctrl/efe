"""Base types for the drift-detection plane.

* :class:`DriftMethod` — Literal of the seven supported methods.
* :class:`DetectionContext` — tenant + task + feature + baseline +
  window scope shared by all detectors.
* :class:`DriftResult` — canonical detector output, persisted into the
  ``drift_evaluations`` table and published as
  ``efe.drift.detected`` NATS events.
* :class:`DriftDetector` :class:`~typing.Protocol` — the single
  entry-point every detector implements.

Detectors are **pure** : they take a ``DetectionContext`` + current
samples (or snapshots) and return a ``DriftResult``. No I/O, no state.
The orchestration layer (:mod:`efe_drift.detection.evaluator`) composes
them against a :class:`BaselineStore` / snapshot-fetching shim.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime
from typing import Any, Literal, Protocol

from efe_drift.detection.severity import Severity, SeverityThresholds
from efe_drift.records import Baseline, FeatureEntry, FeatureSnapshot

DriftMethod = Literal[
    "psi",
    "ks",
    "chi_squared",
    "wasserstein",
    "js_divergence",
    "mmd",
    "classifier_based",
]

MultiScaleWindow = Literal["1h", "1d", "7d"]


# ---------------------------------------------------------------------------
# DetectionContext — inputs common to all detectors
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class DetectionContext:
    """Everything the detectors need except the data itself."""

    tenant_id: str
    task_id: str
    feature: FeatureEntry
    baseline: Baseline
    window_start: datetime
    window_end: datetime
    multi_scale_window: MultiScaleWindow | None = None
    thresholds: SeverityThresholds | None = None


# ---------------------------------------------------------------------------
# DriftResult — canonical output
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class DriftResult:
    """Output of one detector run, ready to persist.

    ``score`` is the primary numeric drift measurement (PSI value,
    KS-D, MMD², …). ``p_value`` is set for detectors that have one
    (``ks``, ``chi_squared``, ``mmd``). ``details`` carries
    method-specific extras (histogram-level contributions, per-bin
    deltas, AUC, …) — these never land in a metric label but are
    surfaced in the UI and dashboards.
    """

    evaluation_id: str
    tenant_id: str
    task_id: str
    feature_id: str
    feature_name: str
    baseline_id: str
    window_start: datetime
    window_end: datetime
    method: DriftMethod
    score: float
    severity: Severity
    computed_at: datetime
    p_value: float | None = None
    multi_scale_window: MultiScaleWindow | None = None
    details: dict[str, Any] = field(default_factory=dict)
    sample_count_current: int = 0
    sample_count_baseline: int = 0

    def is_breach(self) -> bool:
        return self.severity in ("warning", "major", "critical")


# ---------------------------------------------------------------------------
# DriftDetector Protocol
# ---------------------------------------------------------------------------


class DriftDetector(Protocol):
    """Every detector in :mod:`efe_drift.detection.*` implements this."""

    @property
    def method(self) -> DriftMethod:  # pragma: no cover
        ...

    def supported_kinds(
        self,
    ) -> tuple[str, ...]:  # pragma: no cover
        """Return the :class:`FeatureKind` values this detector
        accepts (e.g. numeric, categorical, embedding)."""
        ...

    def detect_from_snapshots(
        self,
        ctx: DetectionContext,
        current: tuple[FeatureSnapshot, ...],
    ) -> DriftResult:  # pragma: no cover
        """Main entry-point from an aggregated window.

        For detectors that can't run against aggregated snapshots
        (e.g. KS which needs raw samples), implementations call
        ``super().detect_from_snapshots`` which raises
        :class:`NotImplementedError`.
        """
        ...


__all__ = [
    "DetectionContext",
    "DriftDetector",
    "DriftMethod",
    "DriftResult",
    "MultiScaleWindow",
]
