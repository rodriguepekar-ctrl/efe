"""Retry executor with exponential backoff + jitter."""

from __future__ import annotations

import asyncio
import random
from collections.abc import Awaitable, Callable
from dataclasses import dataclass, field
from typing import TypeVar

from efe_inference.clock import Clock, SystemClock
from efe_inference.config import RetrySettings
from efe_inference.errors import (
    BackendAuthError,
    BackendBadRequest,
    CircuitOpen,
    InferenceError,
    SafetyBlocked,
    ValidationError,
)

_T = TypeVar("_T")


@dataclass
class RetryExecutor:
    settings: RetrySettings
    clock: Clock = field(default_factory=SystemClock)
    rng: random.Random = field(default_factory=lambda: random.Random(1337))

    async def run(
        self,
        fn: Callable[[], Awaitable[_T]],
        *,
        non_retryable: tuple[type[BaseException], ...] = (
            BackendAuthError, BackendBadRequest, ValidationError,
            SafetyBlocked, CircuitOpen,
        ),
    ) -> _T:
        last_exc: BaseException | None = None
        for attempt in range(1, self.settings.max_attempts + 1):
            try:
                return await fn()
            except non_retryable:
                raise
            except InferenceError as exc:
                if not exc.retryable_hint:
                    raise
                last_exc = exc
            except Exception as exc:
                last_exc = exc
            if attempt >= self.settings.max_attempts:
                break
            await asyncio.sleep(self._sleep_s(attempt))
        assert last_exc is not None
        raise last_exc

    def _sleep_s(self, attempt: int) -> float:
        base = self.settings.backoff_base_s * (2 ** (attempt - 1))
        base = min(self.settings.backoff_max_s, base)
        jitter = 1.0 + (float(self.rng.random()) * 2 - 1) * self.settings.jitter_factor
        return float(max(0.0, base * jitter))


__all__ = ["RetryExecutor"]
