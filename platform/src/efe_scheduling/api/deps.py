"""Dependency container injected into the FastAPI app."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import TYPE_CHECKING

from efe_scheduling.clock import Clock, SystemClock
from efe_scheduling.randomness import Randomness, SystemRandomness
from efe_scheduling.telemetry.events import EventBus, InMemoryEventBus

if TYPE_CHECKING:
    from efe_scheduling.storage.protocols import (
        IdempotencyStore,
        JobStore,
        RunStore,
        ScheduleStore,
        TriggerStore,
    )

__all__ = ["Deps"]


@dataclass(slots=True)
class Deps:
    """Bag of dependencies resolved at app construction time."""

    schedule_store: "ScheduleStore"
    job_store: "JobStore"
    run_store: "RunStore"
    trigger_store: "TriggerStore"
    idempotency_store: "IdempotencyStore"
    clock: Clock = field(default_factory=SystemClock)
    random_source: Randomness = field(default_factory=SystemRandomness)
    event_bus: EventBus = field(default_factory=InMemoryEventBus)
