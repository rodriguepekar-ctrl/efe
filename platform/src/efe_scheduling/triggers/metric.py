"""Metric threshold trigger.

Driven by a :class:`MetricSample` that the engine or a sampler has
obtained (e.g. by querying Prometheus). The trigger compares the value
against a threshold with the configured comparator, then runs the
breach/normal decision through the debouncer.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime
from typing import Any, Literal

from efe_scheduling.triggers.base import Decision, Fire, Hold, Reset, TriggerContext
from efe_scheduling.triggers.debouncer import Debouncer, DebouncerConfig, DebouncerState
from efe_scheduling.types import JobKind

__all__ = ["MetricSample", "MetricTrigger", "MetricTriggerSpec"]


Operator = Literal["<", "<=", ">", ">="]


@dataclass(frozen=True, slots=True)
class MetricSample:
    tenant_id: str
    task_id: str | None
    metric: str  # e.g. "latency_p99_ms"
    value: float
    observed_at: datetime

    def __post_init__(self) -> None:
        if self.observed_at.tzinfo is None:
            raise ValueError("observed_at must be tz-aware")


@dataclass(frozen=True, slots=True)
class MetricTriggerSpec:
    metric: str
    threshold: float
    op: Operator = ">="
    consecutive_breaches: int = 3
    suppression_s: int = 1800
    extra_payload: dict[str, Any] = field(default_factory=dict)


def _compare(op: Operator, v: float, t: float) -> bool:
    match op:
        case "<":
            return v < t
        case "<=":
            return v <= t
        case ">":
            return v > t
        case ">=":
            return v >= t


class MetricTrigger:
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
        spec: MetricTriggerSpec,
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

    def should_fire(self, ctx: TriggerContext) -> Decision:
        ds_in = dict(ctx.debouncer_state)
        if not self.enabled:
            return Hold(reason="trigger disabled", debouncer_state=ds_in)
        sample = ctx.signal
        if not isinstance(sample, MetricSample):
            return Hold(reason="not a MetricSample", debouncer_state=ds_in)
        if sample.tenant_id != self.tenant_id:
            return Hold(reason="tenant mismatch", debouncer_state=ds_in)
        if self.task_id is not None and sample.task_id != self.task_id:
            return Hold(reason="task mismatch", debouncer_state=ds_in)
        if sample.metric != self._spec.metric:
            return Hold(reason="metric name mismatch", debouncer_state=ds_in)

        state = DebouncerState.from_dict(ctx.debouncer_state)
        is_breach = _compare(self._spec.op, sample.value, self._spec.threshold)
        new_state, fire = self._debouncer.step(state, is_breach=is_breach, now=ctx.now)
        new_state_dict = new_state.to_dict()
        if fire:
            payload = {
                "source": "metric",
                "tenant_id": sample.tenant_id,
                "task_id": sample.task_id,
                "metric": sample.metric,
                "value": sample.value,
                "op": self._spec.op,
                "threshold": self._spec.threshold,
                "debouncer_state": new_state_dict,
                **dict(self._spec.extra_payload),
            }
            return Fire(
                payload=payload,
                reason=f"{sample.metric} {self._spec.op} {self._spec.threshold}",
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
