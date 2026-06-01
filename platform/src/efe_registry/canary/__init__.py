"""Canary deployment controller + schedule definitions."""

from __future__ import annotations

from efe_registry.canary.controller import CanaryController, CanaryTickResult
from efe_registry.canary.schedule import (
    SCHEDULE_GEOMETRIC,
    SCHEDULE_LINEAR,
    Schedule,
    Step,
    find_step_index,
    schedule_for,
)
from efe_registry.canary.slo_gateway import (
    InMemorySloGateway,
    SloBreach,
    SloGateway,
)

__all__ = [
    "SCHEDULE_GEOMETRIC",
    "SCHEDULE_LINEAR",
    "CanaryController",
    "CanaryTickResult",
    "InMemorySloGateway",
    "Schedule",
    "SloBreach",
    "SloGateway",
    "Step",
    "find_step_index",
    "schedule_for",
]
