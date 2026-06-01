"""Drift evaluator — orchestrates detectors against a baseline store.

The :class:`DriftEvaluator` is the service-level entry point that
combines :

* :class:`BaselineManager` — to fetch the active baseline for a
  feature
* :class:`DetectorRegistry` — to enumerate applicable detectors
* A snapshot fetch shim (:class:`SnapshotSource` Protocol) —
  InMemory for tests, DB-backed in production
* :func:`compute_scale_windows` — one call yields up to three scales

The evaluator emits a :class:`DriftResult` per (detector × scale)
combination and returns them all — callers persist them to the
``drift_evaluations`` table and/or feed the alerting router.

Errors from individual detectors are **contained** : if one detector
crashes (e.g. insufficient data), the evaluator logs and skips, not
aborting the whole sweep. Other detectors still run.
"""

from __future__ import annotations

from collections.abc import Iterable, Sequence
from dataclasses import dataclass, field
from datetime import datetime
from typing import Protocol

from efe_drift.baseline.manager import BaselineManager
from efe_drift.catalog.registry import FeatureCatalog
from efe_drift.clock import Clock, SystemClock
from efe_drift.detection.base import (
    DetectionContext,
    DriftMethod,
    DriftResult,
    MultiScaleWindow,
)
from efe_drift.detection.registry import (
    DetectorRegistry,
    build_default_registry,
)
from efe_drift.detection.sequential import ScaleWindow, compute_scale_windows
from efe_drift.detection.severity import SeverityThresholds
from efe_drift.errors import NoActiveBaseline
from efe_drift.records import Baseline, FeatureEntry, FeatureSnapshot

# ---------------------------------------------------------------------------
# Snapshot source (tests + production)
# ---------------------------------------------------------------------------


class SnapshotSource(Protocol):
    async def fetch(
        self,
        *,
        tenant_id: str,
        task_id: str,
        feature_id: str,
        bucket_kind: str,
        start: datetime,
        end: datetime,
    ) -> tuple[FeatureSnapshot, ...]:  # pragma: no cover
        ...


class InMemorySnapshotSource:
    """Test double — callers populate it with :meth:`push`."""

    __slots__ = ("_snapshots",)

    def __init__(self) -> None:
        self._snapshots: list[FeatureSnapshot] = []

    def push(self, snapshot: FeatureSnapshot) -> None:
        self._snapshots.append(snapshot)

    def push_many(self, snapshots: Iterable[FeatureSnapshot]) -> None:
        for s in snapshots:
            self.push(s)

    async def fetch(
        self,
        *,
        tenant_id: str,
        task_id: str,
        feature_id: str,
        bucket_kind: str,
        start: datetime,
        end: datetime,
    ) -> tuple[FeatureSnapshot, ...]:
        out = []
        for s in self._snapshots:
            if (
                s.tenant_id == tenant_id
                and s.task_id == task_id
                and s.feature_id == feature_id
                and s.bucket_kind == bucket_kind
                and s.bucket_start >= start
                and s.bucket_end <= end
            ):
                out.append(s)
        return tuple(sorted(out, key=lambda s: s.bucket_start))

    def clear(self) -> None:
        self._snapshots.clear()


# ---------------------------------------------------------------------------
# Evaluation summary
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class EvaluationSweep:
    """All drift results produced by one evaluator invocation.

    ``skipped`` carries error messages for detectors that could not
    run (e.g. feature kind unsupported, zero samples) — useful for
    the UI's "why didn't this feature fire?" column.
    """

    results: tuple[DriftResult, ...] = field(default_factory=tuple)
    skipped: tuple[tuple[str, str], ...] = field(default_factory=tuple)

    def by_method(
        self, method: DriftMethod,
    ) -> tuple[DriftResult, ...]:
        return tuple(r for r in self.results if r.method == method)

    def by_scale(
        self, scale: MultiScaleWindow,
    ) -> tuple[DriftResult, ...]:
        return tuple(
            r for r in self.results if r.multi_scale_window == scale
        )

    def breaches(self) -> tuple[DriftResult, ...]:
        return tuple(r for r in self.results if r.is_breach())


# ---------------------------------------------------------------------------
# DriftEvaluator
# ---------------------------------------------------------------------------


@dataclass
class DriftEvaluator:
    """Orchestrates detectors against the configured baselines."""

    catalog: FeatureCatalog
    baseline_manager: BaselineManager
    snapshots: SnapshotSource
    detectors: DetectorRegistry = field(default_factory=build_default_registry)
    clock: Clock = field(default_factory=SystemClock)
    # Restrict the methods that run automatically (e.g. classifier
    # is raw-only, so skip in the snapshot-driven sweep).
    snapshot_methods: tuple[DriftMethod, ...] = (
        "psi", "js_divergence", "chi_squared",
        "ks", "wasserstein", "mmd",
    )

    async def evaluate_feature(
        self,
        feature: FeatureEntry,
        *,
        now: datetime | None = None,
        scales: tuple[MultiScaleWindow, ...] = ("1h", "1d", "7d"),
        thresholds: SeverityThresholds | None = None,
    ) -> EvaluationSweep:
        """Run every applicable detector × every enabled scale."""
        effective_now = now or self.clock.now()
        baseline = await self.baseline_manager.active_for(
            tenant_id=feature.tenant_id,
            task_id=feature.task_id,
            feature_name=feature.name,
        )
        if baseline is None:
            raise NoActiveBaseline(
                f"no ACTIVE baseline for feature {feature.name!r} "
                f"(tenant={feature.tenant_id!r}, task={feature.task_id!r})",
            )
        windows = compute_scale_windows(effective_now, enabled=scales)
        results: list[DriftResult] = []
        skipped: list[tuple[str, str]] = []
        for window in windows:
            per_window = await self._evaluate_single_window(
                feature=feature,
                baseline=baseline,
                window=window,
                thresholds=thresholds,
                skipped=skipped,
            )
            results.extend(per_window)
        return EvaluationSweep(
            results=tuple(results), skipped=tuple(skipped),
        )

    async def evaluate_many(
        self,
        features: Sequence[FeatureEntry],
        *,
        now: datetime | None = None,
        scales: tuple[MultiScaleWindow, ...] = ("1h", "1d", "7d"),
        thresholds: SeverityThresholds | None = None,
    ) -> tuple[tuple[FeatureEntry, EvaluationSweep], ...]:
        out: list[tuple[FeatureEntry, EvaluationSweep]] = []
        for feature in features:
            try:
                sweep = await self.evaluate_feature(
                    feature, now=now, scales=scales, thresholds=thresholds,
                )
            except NoActiveBaseline:
                sweep = EvaluationSweep(
                    results=(),
                    skipped=(("no_active_baseline", feature.name),),
                )
            out.append((feature, sweep))
        return tuple(out)

    # ---- internals -----------------------------------------------------

    async def _evaluate_single_window(
        self,
        *,
        feature: FeatureEntry,
        baseline: Baseline,
        window: ScaleWindow,
        thresholds: SeverityThresholds | None,
        skipped: list[tuple[str, str]],
    ) -> tuple[DriftResult, ...]:
        bucket_kind = _bucket_kind_for_scale(window.label)
        snapshots = await self.snapshots.fetch(
            tenant_id=feature.tenant_id,
            task_id=feature.task_id,
            feature_id=feature.feature_id,
            bucket_kind=bucket_kind,
            start=window.start,
            end=window.end,
        )
        ctx = DetectionContext(
            tenant_id=feature.tenant_id,
            task_id=feature.task_id,
            feature=feature,
            baseline=baseline,
            window_start=window.start,
            window_end=window.end,
            multi_scale_window=window.label,
            thresholds=thresholds,
        )
        produced: list[DriftResult] = []
        for method in self.snapshot_methods:
            if not self.detectors.has(method):
                continue
            detector = self.detectors.get(method)
            if feature.kind not in detector.supported_kinds():
                continue
            try:
                result = detector.detect_from_snapshots(ctx, snapshots)
            except Exception as exc:
                skipped.append(
                    (f"{method}:{window.label}", f"{type(exc).__name__}: {exc}"),
                )
                continue
            produced.append(result)
        return tuple(produced)


# ---------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------


def _bucket_kind_for_scale(scale: MultiScaleWindow) -> str:
    if scale == "1h":
        return "minute"
    if scale == "1d":
        return "hour"
    if scale == "7d":
        return "day"
    raise ValueError(f"unknown scale {scale!r}")


__all__ = [
    "DriftEvaluator",
    "EvaluationSweep",
    "InMemorySnapshotSource",
    "SnapshotSource",
]
