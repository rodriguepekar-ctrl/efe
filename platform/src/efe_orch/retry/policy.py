"""Retry policy: should-retry decision + exponential backoff.

The policy is intentionally *pure* and deterministic given a seed.
Callers feed it the current ``attempt`` count, the error class, and
the configuration; it returns:

* ``RetryDecision.should_retry`` — whether to schedule a retry,
* ``RetryDecision.delay_s`` — how long to wait before re-queueing,
* ``RetryDecision.next_pool_hint`` — optional pool upgrade hint
  (non-None only for :attr:`ErrorClass.OOM`).

Backoff formula (RFC-style decorrelated exponential):

    base = base_backoff_s * 2 ** (attempt - 1)
    jitter = U(-jitter_ratio, +jitter_ratio) * base
    delay = min(max_backoff_s, base + jitter)

Where ``U(a, b)`` is a deterministic pseudo-random float drawn from
the caller-supplied :class:`random.Random` (or a fresh one, seeded
by run_id, if none is provided).
"""

from __future__ import annotations

import random
from dataclasses import dataclass
from typing import Final

from efe_orch.config import RetrySettings
from efe_orch.retry.classifier import ErrorClass

#: Error classes that the policy *never* retries.
NON_RETRYABLE: Final[frozenset[ErrorClass]] = frozenset({
    ErrorClass.DATA,
    ErrorClass.CONFIG,
    ErrorClass.FATAL,
})


@dataclass(frozen=True, slots=True)
class RetryDecision:
    """The answer to ``should we retry this run?``"""

    should_retry: bool
    delay_s: float
    reason: str
    next_pool_hint: str | None = None
    attempt_next: int = 0


class BackoffPolicy:
    """Exponential + jitter backoff."""

    def __init__(self, settings: RetrySettings) -> None:
        self._s = settings

    def delay(
        self, *, attempt: int, rng: random.Random | None = None,
    ) -> float:
        if attempt <= 0:
            return 0.0
        rnd = rng or random.Random()
        cap: float = float(self._s.max_backoff_s)
        # Exponential component
        raw: float = float(self._s.base_backoff_s) * float(2 ** (attempt - 1))
        # Jitter in [-ratio, +ratio]
        jitter: float = (rnd.random() * 2 - 1) * float(self._s.jitter_ratio) * raw
        out: float = max(0.0, min(cap, raw + jitter))
        return out


class RetryPolicy:
    """Decide whether — and when — to retry a failed run."""

    #: Ordered list of pools to bump to when we hit an OOM-class error.
    #: The list is consulted left-to-right; we pick the first entry
    #: that is *strictly bigger* than the current pool.
    _POOL_UPGRADE_LADDER: Final[tuple[str, ...]] = (
        "cpu-small",
        "cpu-large",
        "gpu-a10",
        "gpu-a100",
    )

    def __init__(self, settings: RetrySettings) -> None:
        self._settings = settings
        self._backoff = BackoffPolicy(settings)

    # ------------------------------------------------------------------ API

    def should_retry(
        self,
        *,
        error_class: ErrorClass,
        attempt: int,
        current_pool: str | None,
        rng: random.Random | None = None,
    ) -> RetryDecision:
        """Main decision function. ``attempt`` is the 1-based count of
        the attempt that just failed (so ``1`` after the first run,
        ``2`` after the first retry, etc.)."""
        if error_class in NON_RETRYABLE:
            return RetryDecision(
                should_retry=False,
                delay_s=0.0,
                reason=f"error class {error_class.value} is non-retryable",
                attempt_next=attempt,
            )
        if attempt >= self._settings.max_attempts:
            return RetryDecision(
                should_retry=False,
                delay_s=0.0,
                reason=(
                    f"attempt budget exhausted "
                    f"({attempt}/{self._settings.max_attempts})"
                ),
                attempt_next=attempt,
            )
        delay = self._backoff.delay(attempt=attempt, rng=rng)
        upgrade = self._upgrade_hint(error_class, current_pool)
        return RetryDecision(
            should_retry=True,
            delay_s=delay,
            reason=(
                f"{error_class.value} error, retrying attempt "
                f"{attempt + 1}/{self._settings.max_attempts} "
                f"in {delay:.1f}s"
                + (f" (upgrade to {upgrade})" if upgrade else "")
            ),
            next_pool_hint=upgrade,
            attempt_next=attempt + 1,
        )

    # ---------------------------------------------------------------- helpers

    def _upgrade_hint(
        self, error_class: ErrorClass, current_pool: str | None,
    ) -> str | None:
        if error_class is not ErrorClass.OOM:
            return None
        if current_pool is None:
            # Guess: OOM without a known pool = start at cpu-large.
            return "cpu-large"
        try:
            idx = self._POOL_UPGRADE_LADDER.index(current_pool)
        except ValueError:
            return None
        if idx + 1 >= len(self._POOL_UPGRADE_LADDER):
            return None
        return self._POOL_UPGRADE_LADDER[idx + 1]


__all__ = [
    "NON_RETRYABLE",
    "BackoffPolicy",
    "RetryDecision",
    "RetryPolicy",
]
