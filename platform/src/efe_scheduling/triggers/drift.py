"""Drift-threshold trigger.

Fires when a :class:`DriftEvent` matches the configured feature / method
filter and the debouncer authorises it.
"""

from __future__ import annotations

import enum
import fnmatch
from dataclasses import dataclass, field
from datetime import datetime
from typing import Any

from efe_scheduling.triggers.base import Decision, Fire, Hold, Reset, TriggerContext
from efe_scheduling.triggers.debouncer import Debouncer, DebouncerConfig, DebouncerState
from efe_scheduling.types import JobKind

__all__ = [
    "DriftEvent",
    "DriftSeverity",
    "DriftThresholdTrigger",
    "DriftTriggerSpec",
]


class DriftSeverity(str, enum.Enum):
    none = "none"
    warning = "warning"
    major = "major"
    critical = "critical"


_SEVERITY_ORDER: dict[DriftSeverity, int] = {
    DriftSeverity.none: 0,
    DriftSeverity.warning: 1,
    DriftSeverity.major: 2,
    DriftSeverity.critical: 3,
}


@dataclass(frozen=True, slots=True)
class DriftEvent:
    """Lightweight mirror of ``efe_drift.records.DriftResult`` keeping
    only the fields M10 cares about."""

    result_id: str
    tenant_id: str
    task_id: str | None
    feature_name: str
    method: str
    score: float
    severity: DriftSeverity
    observed_at: datetime
    window_start: datetime
    window_end: datetime

    def __post_init__(self) -> None:
        for ts in (self.observed_at, self.window_start, self.window_end):
            if ts.tzinfo is None:
                raise ValueError("DriftEvent timestamps must be tz-aware")


@dataclass(frozen=True, slots=True)
class DriftTriggerSpec:
    feature_match: str = "*"  # glob pattern
    method_match: str = "*"
    threshold: float = 0.25
    min_severity: DriftSeverity = DriftSeverity.major
    consecutive_breaches: int = 3
    suppression_s: int = 1800
    extra_payload: dict[str, Any] = field(default_factory=dict)

    def __post_init__(self) -> None:
        if self.consecutive_breaches < 1:
            raise ValueError("consecutive_breaches must be >= 1")
        if self.suppression_s < 0:
            raise ValueError("suppression_s must be >= 0")
        if not 0.0 <= self.threshold:
            raise ValueError("threshold must be >= 0")


class DriftThresholdTrigger:
    """Trigger that fires on sustained drift above a threshold."""

    __slots__ = (
        "_debouncer",
        "_spec",
        "enabled",
        "job_kind",
        "task_id",
        "tenant_id",
        "trigger_id",
    )

    def __init__(
        self,
        *,
        trigger_id: str,
        tenant_id: str,
        task_id: str | None,
        job_kind: JobKind,
        spec: DriftTriggerSpec,
        enabled: bool = True,
    ) -> None:
        self.trigger_id = trigger_id
        self.tenant_id = tenant_id
        self.task_id = task_id
        self.job_kind = job_kind
        self._spec = spec
        self.enabled = enabled
        self._debouncer = Debouncer(
            DebouncerConfig(
                consecutive_breaches=spec.consecutive_breaches,
                suppression_s=spec.suppression_s,
            )
        )

    # ------------------------------------------------------------------
    # Matching
    # ------------------------------------------------------------------

    def _match(self, ev: DriftEvent) -> bool:
        if ev.tenant_id != self.tenant_id:
            return False
        if self.task_id is not None and ev.task_id != self.task_id:
            return False
        if not fnmatch.fnmatchcase(ev.feature_name, self._spec.feature_match):
            return False
        if not fnmatch.fnmatchcase(ev.method, self._spec.method_match):
            return False
        return True

    def _is_breach(self, ev: DriftEvent) -> bool:
        above_score = ev.score >= self._spec.threshold
        above_sev = _SEVERITY_ORDER[ev.severity] >= _SEVERITY_ORDER[self._spec.min_severity]
        return above_score and above_sev

    # ------------------------------------------------------------------
    # Should-fire entry point
    # ------------------------------------------------------------------

    def should_fire(self, ctx: TriggerContext) -> Decision:
        if not self.enabled:
            return Hold(reason="trigger disabled", debouncer_state=dict(ctx.debouncer_state))
        ev = ctx.signal
        if not isinstance(ev, DriftEvent):
            return Hold(reason="signal is not a DriftEvent", debouncer_state=dict(ctx.debouncer_state))
        if not self._match(ev):
            return Hold(reason="no match", debouncer_state=dict(ctx.debouncer_state))

        state = DebouncerState.from_dict(ctx.debouncer_state)
        is_breach = self._is_breach(ev)
        new_state, fire = self._debouncer.step(state, is_breach=is_breach, now=ctx.now)
        new_state_dict = new_state.to_dict()
        if fire:
            payload = {
                "source": "drift",
                "tenant_id": ev.tenant_id,
                "task_id": ev.task_id,
                "feature_name": ev.feature_name,
                "method": ev.method,
                "score": ev.score,
                "severity": ev.severity.value,
                "result_id": ev.result_id,
                "debouncer_state": new_state_dict,
                **dict(self._spec.extra_payload),
            }
            return Fire(
                payload=payload,
                reason=f"drift {ev.method}={ev.score:.3f}",
                debouncer_state=new_state_dict,
            )
        if is_breach:
            return Hold(
                reason=f"accumulating {new_state.count}",
                debouncer_state=new_state_dict,
            )
        if state.count > 0:
            return Reset(reason="cleared", debouncer_state=new_state_dict)
        return Hold(reason="no breach", debouncer_state=new_state_dict)
