"""Continuous Learning glue.

:class:`ContinuousLearningPipeline` is the orchestrator that converts
events (drift, gate verdict, ticks, job-succeeded) into ``JobSpec``
instances to enqueue. It delegates all decision-making to the pure
functions in the sibling modules.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from typing import Any, Mapping

from efe_scheduling.clock import Clock, SystemClock
from efe_scheduling.hashing import DOMAIN_TAG_SCHED_FIRE, hmac_seal
from efe_scheduling.learning.baseline_refresh import (
    BaselineRefreshInputs,
    BaselineRefreshOutcome,
    decide_baseline_refresh,
)
from efe_scheduling.learning.retrain import (
    RetrainInputs,
    RetrainOutcome,
    decide_retrain,
)
from efe_scheduling.triggers.drift import DriftEvent, DriftSeverity
from efe_scheduling.types import (
    JobKind,
    JobPriority,
    JobSpec,
    Origin,
    RetryPolicyKind,
    RetryPolicySpec,
    new_uuid,
)

__all__ = ["ContinuousLearningPipeline", "LearningConfig"]

_LOG = logging.getLogger("efe_scheduling.learning.pipeline")


@dataclass(frozen=True, slots=True)
class LearningConfig:
    # Baseline refresh
    baseline_min_samples: int = 1_000
    baseline_max_variance_ratio: float = 0.25
    baseline_min_interval_h: int = 12
    # Retrain
    retrain_overdue_d: int = 14
    retrain_accuracy_alarm: float = -0.02
    retrain_min_critical_features: int = 3
    # Canary / rollback cool-down
    rollback_cool_down_h: int = 24


@dataclass(slots=True)
class _State:
    """In-memory state of the pipeline (per tenant)."""

    last_retrain_at: datetime | None = None
    cool_down_until: datetime | None = None
    baseline_last_refresh_at: dict[str, datetime] = field(default_factory=dict)


class ContinuousLearningPipeline:
    """Event-driven orchestrator."""

    __slots__ = ("_cfg", "_clock", "_state")

    def __init__(self, cfg: LearningConfig | None = None, clock: Clock | None = None) -> None:
        self._cfg = cfg if cfg is not None else LearningConfig()
        self._clock = clock if clock is not None else SystemClock()
        self._state = _State()

    @property
    def state(self) -> _State:
        return self._state

    # ------------------------------------------------------------------
    # Public entry points
    # ------------------------------------------------------------------

    async def on_drift(self, event: DriftEvent) -> list[JobSpec]:
        """React to a drift event.

        Emits :

        * a ``baseline_refresh`` if the decision requests it,
        * a ``retrain_decision`` if severity is critical enough.
        """
        out: list[JobSpec] = []
        now = self._clock.now()

        # Baseline-refresh opportunity on any major+ drift.
        if event.severity in (DriftSeverity.major, DriftSeverity.critical):
            br_decision = decide_baseline_refresh(
                BaselineRefreshInputs(
                    feature_id=event.feature_name,
                    feature_active=True,
                    last_refresh_at=self._state.baseline_last_refresh_at.get(
                        event.feature_name
                    ),
                    now=now,
                    sample_count_last_window=max(
                        self._cfg.baseline_min_samples,
                        self._cfg.baseline_min_samples,
                    ),
                    min_samples=self._cfg.baseline_min_samples,
                    variance_ratio=0.0,
                    max_variance_ratio=self._cfg.baseline_max_variance_ratio,
                ),
                min_interval_h=self._cfg.baseline_min_interval_h,
            )
            if br_decision.outcome == BaselineRefreshOutcome.refresh:
                job = self._make_job(
                    kind=JobKind.baseline_refresh,
                    tenant_id=event.tenant_id,
                    task_id=event.task_id,
                    payload={
                        "feature_name": event.feature_name,
                        "window_end": event.window_end.astimezone(timezone.utc).isoformat(),
                        "reason": br_decision.reason,
                    },
                    origin_ref=event.result_id,
                    now=now,
                    priority=JobPriority.high,
                )
                self._state.baseline_last_refresh_at[event.feature_name] = now
                out.append(job)

        # Retrain decision on critical drift.
        if event.severity == DriftSeverity.critical:
            rd = decide_retrain(
                RetrainInputs(
                    now=now,
                    last_retrain_at=self._state.last_retrain_at,
                    cool_down_until=self._state.cool_down_until,
                    max_drift_severity="critical",
                    critical_feature_count=self._cfg.retrain_min_critical_features,
                    overdue_threshold_d=self._cfg.retrain_overdue_d,
                    accuracy_delta_last_7d=0.0,
                    accuracy_delta_alarm=self._cfg.retrain_accuracy_alarm,
                    dataset_coverage_ok=True,
                    min_critical_features=self._cfg.retrain_min_critical_features,
                )
            )
            if rd.outcome == RetrainOutcome.retrain:
                job = self._make_job(
                    kind=JobKind.retrain_decision,
                    tenant_id=event.tenant_id,
                    task_id=event.task_id,
                    payload={"reason": rd.reason, "source_event": event.result_id},
                    origin_ref=event.result_id,
                    now=now,
                    priority=JobPriority.high,
                )
                out.append(job)
        return out

    async def on_gate_failed(
        self,
        tenant_id: str,
        task_id: str | None,
        major_failures: int,
    ) -> list[JobSpec]:
        """On a gate failure, freeze future retrains for the cool-down period."""
        if major_failures <= 0:
            return []
        now = self._clock.now()
        self._state.cool_down_until = now + timedelta(
            hours=self._cfg.rollback_cool_down_h
        )
        job = self._make_job(
            kind=JobKind.rollback,
            tenant_id=tenant_id,
            task_id=task_id,
            payload={
                "reason": "gate_failed",
                "major_failures": major_failures,
                "cool_down_until": self._state.cool_down_until.isoformat(),
            },
            origin_ref="gate_verdict",
            now=now,
            priority=JobPriority.critical,
        )
        return [job]

    async def on_job_succeeded(
        self,
        *,
        kind: JobKind,
        tenant_id: str,
        task_id: str | None,
        result: Mapping[str, Any],
    ) -> list[JobSpec]:
        """Chain follow-up jobs on successful completion."""
        now = self._clock.now()
        chained: list[JobSpec] = []
        match kind:
            case JobKind.retrain_decision:
                chained.append(
                    self._make_job(
                        kind=JobKind.data_curation,
                        tenant_id=tenant_id,
                        task_id=task_id,
                        payload={"parent_result": dict(result)},
                        origin_ref=str(result.get("job_id", "chained")),
                        now=now,
                        priority=JobPriority.high,
                    )
                )
            case JobKind.data_curation:
                chained.append(
                    self._make_job(
                        kind=JobKind.training_job,
                        tenant_id=tenant_id,
                        task_id=task_id,
                        payload={"dataset_manifest": dict(result)},
                        origin_ref=str(result.get("job_id", "chained")),
                        now=now,
                        priority=JobPriority.normal,
                    )
                )
            case JobKind.training_job:
                self._state.last_retrain_at = now
                chained.append(
                    self._make_job(
                        kind=JobKind.validation_job,
                        tenant_id=tenant_id,
                        task_id=task_id,
                        payload={"artifact_id": result.get("artifact_id")},
                        origin_ref=str(result.get("job_id", "chained")),
                        now=now,
                        priority=JobPriority.normal,
                    )
                )
            case JobKind.validation_job:
                chained.append(
                    self._make_job(
                        kind=JobKind.canary_evaluate,
                        tenant_id=tenant_id,
                        task_id=task_id,
                        payload={"artifact_id": result.get("artifact_id")},
                        origin_ref=str(result.get("job_id", "chained")),
                        now=now,
                        priority=JobPriority.normal,
                    )
                )
            case JobKind.canary_evaluate:
                chained.append(
                    self._make_job(
                        kind=JobKind.promotion_decision,
                        tenant_id=tenant_id,
                        task_id=task_id,
                        payload={"artifact_id": result.get("artifact_id")},
                        origin_ref=str(result.get("job_id", "chained")),
                        now=now,
                        priority=JobPriority.high,
                    )
                )
            case _:
                pass
        return chained

    # ------------------------------------------------------------------
    # Helpers
    # ------------------------------------------------------------------

    def _make_job(
        self,
        *,
        kind: JobKind,
        tenant_id: str,
        task_id: str | None,
        payload: dict[str, Any],
        origin_ref: str,
        now: datetime,
        priority: JobPriority,
    ) -> JobSpec:
        job_id = new_uuid()
        # Deterministic fire_key : does NOT include job_id, so replaying the
        # same (origin_ref, kind, tenant, task) event yields the same key
        # and the JobStore ON CONFLICT DO NOTHING suppresses the duplicate.
        # This backs invariant I1 for trigger-driven firings.
        fire_key = hmac_seal(
            DOMAIN_TAG_SCHED_FIRE,
            {
                "source": "learning",
                "kind": kind.value,
                "tenant_id": tenant_id,
                "task_id": task_id,
                "origin_ref": origin_ref,
            },
        )
        spec = JobSpec(
            job_id=job_id,
            fire_key=fire_key,
            tenant_id=tenant_id,
            task_id=task_id,
            kind=kind,
            payload=dict(payload),
            priority=priority,
            priority_seq=job_id,
            run_at=now,
            deadline=None,
            max_attempts=3,
            retry_policy=RetryPolicySpec(
                kind=RetryPolicyKind.decorrelated_jitter,
                base_ms=1_000,
                max_ms=60_000,
                jitter_ratio=0.0,
            ),
            origin=Origin.chained,
            origin_ref=origin_ref,
            created_at=now,
        ).sealed()
        _LOG.info("learning.pipeline.enqueue kind=%s tenant=%s", kind, tenant_id)
        return spec
