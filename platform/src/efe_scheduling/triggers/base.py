"""Trigger Protocol and decision types."""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime
from typing import Any, Protocol, Union

from efe_scheduling.clock import Clock
from efe_scheduling.randomness import Randomness
from efe_scheduling.types import JobKind

__all__ = [
    "Decision",
    "Fire",
    "Hold",
    "Reset",
    "Trigger",
    "TriggerContext",
]


# ---------------------------------------------------------------------------
# Decision algebraic type
# ---------------------------------------------------------------------------
#
# Every decision variant carries the *new* ``debouncer_state`` so the caller
# can persist it atomically. Without this, a Hold or Reset would silently
# lose the accumulated counter and the trigger would never fire.


@dataclass(frozen=True, slots=True)
class Fire:
    """Fire with an accompanying payload."""

    payload: dict[str, Any] = field(default_factory=dict)
    reason: str = ""
    debouncer_state: dict[str, Any] = field(default_factory=dict)


@dataclass(frozen=True, slots=True)
class Hold:
    """Don't fire ; the caller must still persist ``debouncer_state``."""

    reason: str = ""
    debouncer_state: dict[str, Any] = field(default_factory=dict)


@dataclass(frozen=True, slots=True)
class Reset:
    """Don't fire and reset the debouncer (observed a clean state)."""

    reason: str = ""
    debouncer_state: dict[str, Any] = field(default_factory=dict)


Decision = Union[Fire, Hold, Reset]


# ---------------------------------------------------------------------------
# Context
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class TriggerContext:
    """Runtime context passed to :meth:`Trigger.should_fire`."""

    tenant_id: str
    now: datetime
    clock: Clock
    random_source: Randomness
    signal: Any
    debouncer_state: dict[str, Any] = field(default_factory=dict)


# ---------------------------------------------------------------------------
# Protocol
# ---------------------------------------------------------------------------


class Trigger(Protocol):
    trigger_id: str
    tenant_id: str
    task_id: str | None
    job_kind: JobKind
    enabled: bool

    def should_fire(self, ctx: TriggerContext) -> Decision: ...  # pragma: no cover
