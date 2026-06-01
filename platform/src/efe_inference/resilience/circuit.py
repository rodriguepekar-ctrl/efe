"""CircuitBreaker — 3-state FSM, one per (tenant, backend).

States: ``closed`` (normal), ``open`` (refuse calls), ``half_open``
(probe). Transitions exactly follow M07_ARCHITECTURE.md §13.2. The
implementation is fully in-memory here; a production deployment keeps
a copy in Redis + Postgres for rehydration across replicas.
"""

from __future__ import annotations

import asyncio
from dataclasses import dataclass, field
from enum import Enum

from efe_inference.clock import Clock, SystemClock
from efe_inference.config import CircuitSettings
from efe_inference.errors import CircuitOpen


class CircuitStateValue(str, Enum):
    CLOSED = "closed"
    OPEN = "open"
    HALF_OPEN = "half_open"


@dataclass(slots=True)
class _State:
    value: CircuitStateValue = CircuitStateValue.CLOSED
    failures: int = 0
    successes: int = 0
    opened_at_mono: float = 0.0
    last_transition_mono: float = 0.0
    # M1 — ``first_failure_mono`` tracks the start of the current rolling
    # window. When ``now - first_failure_mono > window_s`` we reset
    # ``failures`` so a backend that fails once per day doesn't
    # eventually trip the breaker from stale noise.
    first_failure_mono: float = 0.0


@dataclass
class CircuitBreaker:
    """Per-(tenant, backend) circuit state."""

    settings: CircuitSettings
    clock: Clock = field(default_factory=SystemClock)
    _state: _State = field(default_factory=_State)
    _lock: asyncio.Lock = field(default_factory=asyncio.Lock)

    @property
    def state(self) -> CircuitStateValue:
        return self._state.value

    @property
    def failures(self) -> int:
        return self._state.failures

    async def allow(self) -> None:
        """Raise :class:`CircuitOpen` if the call must be refused."""
        async with self._lock:
            now = self.clock.now_monotonic_s()
            self._maybe_transition_open_to_half(now)
            if self._state.value == CircuitStateValue.OPEN:
                raise CircuitOpen("circuit is open; refused before dispatch")

    async def on_success(self) -> None:
        async with self._lock:
            now = self.clock.now_monotonic_s()
            if self._state.value == CircuitStateValue.HALF_OPEN:
                self._state.successes += 1
                if self._state.successes >= self.settings.success_threshold:
                    self._set(CircuitStateValue.CLOSED, now)
                    self._state.failures = 0
                    self._state.successes = 0
            elif self._state.value == CircuitStateValue.CLOSED:
                self._state.failures = 0

    async def on_failure(self) -> None:
        async with self._lock:
            now = self.clock.now_monotonic_s()
            if self._state.value == CircuitStateValue.CLOSED:
                # M1 — rolling failure window. Reset the counter if the
                # previous failure is older than ``window_s``.
                if (
                    self._state.failures == 0
                    or now - self._state.first_failure_mono
                        > self.settings.window_s
                ):
                    self._state.first_failure_mono = now
                    self._state.failures = 0
                self._state.failures += 1
                if self._state.failures >= self.settings.failure_threshold:
                    self._set(CircuitStateValue.OPEN, now)
                    self._state.opened_at_mono = now
            elif self._state.value == CircuitStateValue.HALF_OPEN:
                # Any failure during probe → immediately re-open.
                self._set(CircuitStateValue.OPEN, now)
                self._state.opened_at_mono = now
                self._state.successes = 0

    def _maybe_transition_open_to_half(self, now: float) -> None:
        if self._state.value != CircuitStateValue.OPEN:
            return
        elapsed = now - self._state.opened_at_mono
        if elapsed >= self.settings.cool_down_s:
            self._set(CircuitStateValue.HALF_OPEN, now)
            self._state.successes = 0

    def _set(self, value: CircuitStateValue, now: float) -> None:
        self._state.value = value
        self._state.last_transition_mono = now


_KEY = tuple[str, str]    # (tenant, backend)


@dataclass
class CircuitBreakerRegistry:
    """One breaker per ``(tenant, backend_id)``."""

    settings: CircuitSettings
    clock: Clock = field(default_factory=SystemClock)
    _breakers: dict[_KEY, CircuitBreaker] = field(default_factory=dict)

    def get(self, tenant_id: str, backend_id: str) -> CircuitBreaker:
        k = (tenant_id, backend_id)
        br = self._breakers.get(k)
        if br is None:
            br = CircuitBreaker(settings=self.settings, clock=self.clock)
            self._breakers[k] = br
        return br

    def snapshot(self) -> dict[_KEY, CircuitStateValue]:
        return {k: v.state for k, v in self._breakers.items()}


__all__ = [
    "CircuitBreaker",
    "CircuitBreakerRegistry",
    "CircuitStateValue",
]
