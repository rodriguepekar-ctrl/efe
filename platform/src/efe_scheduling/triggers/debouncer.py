"""Debouncer state machine.

Keeps track of *consecutive breaches* and *suppression window* for a
single ``(trigger_id, key)`` pair.

States :

* ``idle`` — no active breach streak.
* ``count<N>`` — counted N consecutive breaches, under the threshold.
* ``fire`` — just fired, waiting for ``suppression_s`` to elapse before
  returning to ``idle``.

Public API is a pure function :func:`step` : it takes the current state,
the observed sample (breach / normal), the configuration and the current
``now``, and returns a new state plus a decision.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timedelta
from typing import Any

__all__ = ["Debouncer", "DebouncerState", "DebouncerConfig"]


@dataclass(frozen=True, slots=True)
class DebouncerConfig:
    consecutive_breaches: int = 3
    suppression_s: int = 1800  # 30 min

    def __post_init__(self) -> None:
        if self.consecutive_breaches < 1:
            raise ValueError("consecutive_breaches must be >= 1")
        if self.suppression_s < 0:
            raise ValueError("suppression_s must be >= 0")


@dataclass(frozen=True, slots=True)
class DebouncerState:
    count: int = 0
    last_fired_at: datetime | None = None
    phase: str = "idle"  # idle | counting | suppressed

    def to_dict(self) -> dict[str, Any]:
        return {
            "count": self.count,
            "last_fired_at": (
                self.last_fired_at.isoformat() if self.last_fired_at else None
            ),
            "phase": self.phase,
        }

    @staticmethod
    def from_dict(d: dict[str, Any]) -> DebouncerState:
        lf = d.get("last_fired_at")
        return DebouncerState(
            count=int(d.get("count", 0)),
            last_fired_at=(datetime.fromisoformat(lf) if lf else None),
            phase=str(d.get("phase", "idle")),
        )


class Debouncer:
    """Pure decision helper for a debouncer.

    The debouncer itself is stateless — the state lives on the caller
    and is passed in and out.
    """

    __slots__ = ("_cfg",)

    def __init__(self, cfg: DebouncerConfig) -> None:
        self._cfg = cfg

    @property
    def config(self) -> DebouncerConfig:
        return self._cfg

    def step(
        self,
        state: DebouncerState,
        *,
        is_breach: bool,
        now: datetime,
    ) -> tuple[DebouncerState, bool]:
        """Advance the state machine.

        Returns ``(new_state, should_fire)``.
        """
        # Suppression window first
        if (
            state.phase == "suppressed"
            and state.last_fired_at is not None
            and (now - state.last_fired_at) < timedelta(seconds=self._cfg.suppression_s)
        ):
            return state, False

        # Suppression window elapsed ; fall through to idle
        if state.phase == "suppressed":
            state = DebouncerState(count=0, last_fired_at=state.last_fired_at, phase="idle")

        if not is_breach:
            # Any normal sample resets the count, keeps last_fired_at.
            if state.count == 0 and state.phase == "idle":
                return state, False
            return (
                DebouncerState(count=0, last_fired_at=state.last_fired_at, phase="idle"),
                False,
            )

        # A breach sample
        new_count = state.count + 1
        if new_count >= self._cfg.consecutive_breaches:
            fired_state = DebouncerState(
                count=0, last_fired_at=now, phase="suppressed"
            )
            return fired_state, True
        return DebouncerState(count=new_count, last_fired_at=state.last_fired_at, phase="counting"), False
