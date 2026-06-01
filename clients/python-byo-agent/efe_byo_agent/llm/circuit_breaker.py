"""Per-provider circuit breaker (M20).

State machine :
  CLOSED        : healthy ; requests go through.
  OPEN          : N consecutive failures observed ; requests skipped
                  for `open_duration_s` ; after that, transition to
                  HALF_OPEN.
  HALF_OPEN     : one probe request allowed ; success → CLOSED, failure
                  → OPEN with reset timer.

Thread-safety : the breaker is intended to be used per-Agent ; if you
share an Agent across asyncio tasks, the breaker is naturally
single-threaded since asyncio is cooperative. In a true multi-threaded
context, wrap calls with `asyncio.Lock`.
"""
from __future__ import annotations

import time
from dataclasses import dataclass, field
from enum import Enum


class CircuitState(str, Enum):
    CLOSED = "closed"
    OPEN = "open"
    HALF_OPEN = "half_open"


@dataclass
class CircuitBreaker:
    """Failure-driven circuit breaker.

    Args :
      threshold       : consecutive failures required to OPEN. Default 5.
      open_duration_s : seconds to wait in OPEN before HALF_OPEN. Default 60.
    """

    threshold: int = 5
    open_duration_s: float = 60.0
    state: CircuitState = field(default=CircuitState.CLOSED)
    consecutive_failures: int = 0
    opened_at: float | None = None

    def is_closed(self) -> bool:
        """Return True if a request should be attempted."""
        if self.state is CircuitState.CLOSED:
            return True
        if self.state is CircuitState.OPEN:
            if (
                self.opened_at is not None
                and (time.monotonic() - self.opened_at) >= self.open_duration_s
            ):
                self.state = CircuitState.HALF_OPEN
                return True   # allow one probe
            return False
        # HALF_OPEN : let one through ; record_* will transition.
        return True

    def record_success(self) -> None:
        self.consecutive_failures = 0
        self.state = CircuitState.CLOSED
        self.opened_at = None

    def record_failure(self) -> None:
        self.consecutive_failures += 1
        if self.consecutive_failures >= self.threshold:
            self.state = CircuitState.OPEN
            self.opened_at = time.monotonic()

    def reset(self) -> None:
        """Force the breaker back to CLOSED. Used for tests."""
        self.consecutive_failures = 0
        self.state = CircuitState.CLOSED
        self.opened_at = None


__all__ = ["CircuitBreaker", "CircuitState"]
