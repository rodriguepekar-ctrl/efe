"""Pydantic v2 schemas for the REST API.

All models set ``extra="forbid"`` so clients can't slip unexpected keys.
"""

from __future__ import annotations

from datetime import datetime
from typing import Annotated, Any, Literal

try:  # pragma: no cover - optional dep
    from pydantic import BaseModel, ConfigDict, Field
except ImportError:  # pragma: no cover
    # Very small fallback so imports succeed during bootstrap. Real API
    # requires pydantic; tests for schemas will skip if missing.
    BaseModel = object  # type: ignore[assignment,misc]

    def ConfigDict(**kwargs: Any) -> dict[str, Any]:  # type: ignore[no-redef]
        return kwargs

    def Field(*_args: Any, **_kwargs: Any) -> Any:  # type: ignore[no-redef]
        return None


__all__ = [
    "CompositeScheduleSchema",
    "CronScheduleSchema",
    "DriftTriggerCreateSchema",
    "IntervalScheduleSchema",
    "JobSummarySchema",
    "ManualFireRequest",
    "MetricTriggerCreateSchema",
    "OneShotScheduleSchema",
    "ProblemDetails",
    "RRuleScheduleSchema",
    "ScheduleCreateRequest",
    "ScheduleSchema",
    "TriggerCreateRequest",
    "TriggerSchema",
]


class _ForbidExtra(BaseModel):  # type: ignore[misc]
    model_config = ConfigDict(extra="forbid")


# ---------------------------------------------------------------------------
# Schedule payloads (sum type)
# ---------------------------------------------------------------------------


class CronScheduleSchema(_ForbidExtra):
    kind: Literal["cron"]
    expression: str
    timezone_name: str = "UTC"
    jitter_s: int = Field(default=0, ge=0, le=300)


class IntervalScheduleSchema(_ForbidExtra):
    kind: Literal["interval"]
    period_s: int = Field(ge=60, le=86_400 * 30)
    phase_s: int = Field(default=0, ge=0)
    jitter_s: int = Field(default=0, ge=0, le=300)


class RRuleScheduleSchema(_ForbidExtra):
    kind: Literal["rrule"]
    rrule_text: str
    timezone_name: str = "UTC"
    dtstart: datetime | None = None


class OneShotScheduleSchema(_ForbidExtra):
    kind: Literal["one_shot"]
    at: datetime


class CompositeScheduleSchema(_ForbidExtra):
    kind: Literal["composite"]
    op: Literal["union", "exclude"]
    left: "ScheduleSpecSchema"
    right: "ScheduleSpecSchema"


ScheduleSpecSchema = Annotated[
    CronScheduleSchema
    | IntervalScheduleSchema
    | RRuleScheduleSchema
    | OneShotScheduleSchema
    | CompositeScheduleSchema,
    Field(discriminator="kind"),
]


class ScheduleCreateRequest(_ForbidExtra):
    tenant_id: str
    task_id: str | None = None
    job_kind: str
    spec: ScheduleSpecSchema
    payload: dict[str, Any] = Field(default_factory=dict)
    enabled: bool = True


class ScheduleSchema(_ForbidExtra):
    schedule_id: str
    tenant_id: str
    task_id: str | None
    job_kind: str
    spec: ScheduleSpecSchema
    payload: dict[str, Any]
    enabled: bool
    state: str
    state_version: int
    last_evaluated_at: datetime | None
    created_at: datetime
    updated_at: datetime


# ---------------------------------------------------------------------------
# Triggers
# ---------------------------------------------------------------------------


class DriftTriggerCreateSchema(_ForbidExtra):
    kind: Literal["drift"]
    feature_match: str = "*"
    method_match: str = "*"
    threshold: float = Field(ge=0.0, le=1_000.0)
    min_severity: Literal["none", "warning", "major", "critical"] = "major"
    consecutive_breaches: int = Field(default=3, ge=1, le=20)
    suppression_s: int = Field(default=1800, ge=0, le=86_400)


class MetricTriggerCreateSchema(_ForbidExtra):
    kind: Literal["metric"]
    metric: str
    threshold: float
    op: Literal["<", "<=", ">", ">="] = ">="
    consecutive_breaches: int = Field(default=3, ge=1, le=20)
    suppression_s: int = Field(default=1800, ge=0, le=86_400)


class TriggerCreateRequest(_ForbidExtra):
    tenant_id: str
    task_id: str | None = None
    job_kind: str
    spec: DriftTriggerCreateSchema | MetricTriggerCreateSchema = Field(
        discriminator="kind"
    )
    enabled: bool = True


class TriggerSchema(_ForbidExtra):
    trigger_id: str
    tenant_id: str
    task_id: str | None
    job_kind: str
    enabled: bool
    state_version: int
    last_fired_at: datetime | None


# ---------------------------------------------------------------------------
# Manual fire
# ---------------------------------------------------------------------------


class ManualFireRequest(_ForbidExtra):
    actor: str
    idempotency_key: str = Field(min_length=8, max_length=128)
    payload: dict[str, Any] = Field(default_factory=dict)
    reason: str = "manual"


# ---------------------------------------------------------------------------
# Jobs
# ---------------------------------------------------------------------------


class JobSummarySchema(_ForbidExtra):
    job_id: str
    tenant_id: str
    task_id: str | None
    kind: str
    priority: int
    run_at: datetime
    state: str
    attempts_left: int
    created_at: datetime


# ---------------------------------------------------------------------------
# Errors
# ---------------------------------------------------------------------------


class ProblemDetails(_ForbidExtra):
    type: str
    title: str
    status: int
    detail: str | None = None
    retryable: bool = False
    request_id: str | None = None
