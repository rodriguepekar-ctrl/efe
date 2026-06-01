"""Circuit breaker — 3 states, per-tenant, rolling window."""

from __future__ import annotations

import asyncio
import enum
import time
from dataclasses import dataclass, field

from efe_swarm_integration.errors import CircuitOpen

__all__ = ["CbState", "CircuitBreaker", "CircuitBreakerRegistry"]


class CbState(str, enum.Enum):
    closed = "closed"
    open = "open"
    half_open = "half_open"


@dataclass(slots=True)
class _Window:
    samples: list[tuple[float, bool]] = field(default_factory=list)  # (mono_ts, ok)

    def prune(self, now: float, window_s: float) -> None:
        threshold = now - window_s
        self.samples = [(t, ok) for (t, ok) in self.samples if t >= threshold]

    def total(self) -> int:
        return len(self.samples)

    def errors(self) -> int:
        return sum(1 for (_, ok) in self.samples if not ok)


@dataclass(slots=True)
class CircuitBreaker:
    tenant_id: str
    error_rate_threshold: float
    window_s: float
    min_samples: int
    open_cooldown_s: float
    _state: CbState = field(default=CbState.closed)
    _opened_at: float = field(default=0.0)
    _window: _Window = field(default_factory=_Window)
    _lock: asyncio.Lock = field(default_factory=asyncio.Lock, repr=False)

    async def pre_call(self) -> None:
        """Raise :class:`CircuitOpen` if the breaker blocks the call."""
        async with self._lock:
            now = time.monotonic()
            if self._state == CbState.open:
                if now - self._opened_at >= self.open_cooldown_s:
                    self._state = CbState.half_open
                else:
                    raise CircuitOpen(
                        f"circuit open for tenant {self.tenant_id}",
                        detail={"cooldown_left_s": self.open_cooldown_s - (now - self._opened_at)},
                    )

    async def record(self, *, success: bool) -> None:
        async with self._lock:
            now = time.monotonic()
            self._window.samples.append((now, success))
            self._window.prune(now, self.window_s)

            if self._state == CbState.half_open:
                if success:
                    self._state = CbState.closed
                    self._window.samples.clear()
                else:
                    self._state = CbState.open
                    self._opened_at = now
                return

            if self._state == CbState.closed:
                total = self._window.total()
                if total >= self.min_samples:
                    err_rate = self._window.errors() / total
                    if err_rate > self.error_rate_threshold:
                        self._state = CbState.open
                        self._opened_at = now

    @property
    def state(self) -> CbState:
        return self._state


class CircuitBreakerRegistry:
    """One :class:`CircuitBreaker` per tenant."""

    __slots__ = ("_breakers", "_cfg", "_lock")

    def __init__(
        self,
        *,
        error_rate_threshold: float,
        window_s: float,
        min_samples: int,
        open_cooldown_s: float,
    ) -> None:
        self._cfg = {
            "error_rate_threshold": error_rate_threshold,
            "window_s": window_s,
            "min_samples": min_samples,
            "open_cooldown_s": open_cooldown_s,
        }
        self._breakers: dict[str, CircuitBreaker] = {}
        self._lock = asyncio.Lock()

    async def for_tenant(self, tenant_id: str) -> CircuitBreaker:
        async with self._lock:
            cb = self._breakers.get(tenant_id)
            if cb is None:
                cb = CircuitBreaker(tenant_id=tenant_id, **self._cfg)  # type: ignore[arg-type]
                self._breakers[tenant_id] = cb
            return cb

    def state_snapshot(self) -> dict[str, CbState]:
        return {t: cb.state for t, cb in self._breakers.items()}
