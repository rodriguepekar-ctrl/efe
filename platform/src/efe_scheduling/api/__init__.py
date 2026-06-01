"""REST API — Pydantic schemas + FastAPI-compatible app builder.

FastAPI is optional : the :func:`build_app` function gracefully degrades
to a stub when the package is missing, so unit tests that only import
schemas still work.
"""

from __future__ import annotations

from efe_scheduling.api.deps import Deps
from efe_scheduling.api.http_app import build_app
from efe_scheduling.api.schemas import (
    CronScheduleSchema,
    DriftTriggerCreateSchema,
    IntervalScheduleSchema,
    JobSummarySchema,
    ManualFireRequest,
    OneShotScheduleSchema,
    ProblemDetails,
    RRuleScheduleSchema,
    ScheduleCreateRequest,
    ScheduleSchema,
    TriggerCreateRequest,
    TriggerSchema,
)

__all__ = [
    "CronScheduleSchema",
    "Deps",
    "DriftTriggerCreateSchema",
    "IntervalScheduleSchema",
    "JobSummarySchema",
    "ManualFireRequest",
    "OneShotScheduleSchema",
    "ProblemDetails",
    "RRuleScheduleSchema",
    "ScheduleCreateRequest",
    "ScheduleSchema",
    "TriggerCreateRequest",
    "TriggerSchema",
    "build_app",
]
