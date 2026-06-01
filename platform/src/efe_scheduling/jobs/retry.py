"""Retry & backoff.

Four policies are supported :

* ``fixed``              — constant ``base_ms`` delay.
* ``linear``             — ``base_ms * attempt`` (capped).
* ``exponential``        — ``base_ms * 2^(attempt-1)`` (capped).
* ``decorrelated_jitter``— ``rand(base_ms, min(max_ms, prev*3))`` (AWS).

Jitter (when not baked into the policy) is a multiplicative ± factor
``jitter_ratio``. The :class:`RetryEngine` is stateful per job (it
remembers the previous delay) but lightweight (a few floats).
"""

from __future__ import annotations

import logging
from dataclasses import dataclass
from typing import TYPE_CHECKING

from efe_scheduling.randomness import Randomness, SystemRandomness
from efe_scheduling.types import RetryPolicyKind, RetryPolicySpec

if TYPE_CHECKING:  # pragma: no cover
    pass

__all__ = [
    "RetryDecision",
    "RetryEngine",
    "backoff_delay_ms",
    "is_retryable",
]


_LOG = logging.getLogger("efe_scheduling.retry")


@dataclass(frozen=True, slots=True)
class RetryDecision:
    """Outcome of the retry engine for a single failure."""

    should_retry: bool
    delay_ms: int
    reason: str


def is_retryable(error: BaseException) -> bool:
    """Default policy : any exception carrying a truthy ``retryable_hint``
    attribute is retryable ; everything else is not.

    ``asyncio.TimeoutError`` is retryable (transient).
    """
    import asyncio as _asyncio

    if isinstance(error, _asyncio.TimeoutError):
        return True
    hint = getattr(error, "retryable_hint", None)
    if isinstance(hint, bool):
        return hint
    return False


def backoff_delay_ms(
    policy: RetryPolicySpec,
    attempt_idx: int,
    *,
    prev_delay_ms: int | None = None,
    random_source: Randomness | None = None,
) -> int:
    """Return the delay (milliseconds) to wait before the ``attempt_idx``
    retry.

    ``attempt_idx`` is 1-based — the first retry uses ``attempt_idx=1``.
    """
    if attempt_idx < 1:
        raise ValueError("attempt_idx must be >= 1")
    rng = random_source if random_source is not None else SystemRandomness()
    base = policy.base_ms
    cap = policy.max_ms

    match policy.kind:
        case RetryPolicyKind.fixed:
            d = base
        case RetryPolicyKind.linear:
            d = base * attempt_idx
        case RetryPolicyKind.exponential:
            d = base * (2 ** (attempt_idx - 1))
        case RetryPolicyKind.decorrelated_jitter:
            prev = prev_delay_ms if prev_delay_ms is not None else base
            upper = min(cap, prev * 3)
            if upper < base:
                upper = base
            # uniform integer in [base, upper]
            d = int(rng.uniform(float(base), float(upper)))

    d = max(base, min(cap, d))

    # Optional multiplicative jitter for non-decorrelated policies.
    if policy.kind != RetryPolicyKind.decorrelated_jitter and policy.jitter_ratio > 0:
        jitter_span = int(d * policy.jitter_ratio)
        if jitter_span > 0:
            offset = rng.randint(-jitter_span, jitter_span)
            d = max(0, d + offset)

    return int(d)


class RetryEngine:
    """Stateful wrapper around :func:`backoff_delay_ms`.

    One instance per job-in-flight. Records the previous delay so the
    decorrelated-jitter policy can self-reference.
    """

    __slots__ = ("_policy", "_prev", "_random")

    def __init__(
        self,
        policy: RetryPolicySpec,
        *,
        random_source: Randomness | None = None,
    ) -> None:
        self._policy = policy
        self._prev: int | None = None
        self._random = random_source if random_source is not None else SystemRandomness()

    def decide(
        self,
        *,
        error: BaseException | None,
        attempt_idx: int,
        attempts_left: int,
        wall_clock_budget_exhausted: bool,
    ) -> RetryDecision:
        if error is None:
            return RetryDecision(False, 0, "no error")
        if wall_clock_budget_exhausted:
            return RetryDecision(False, 0, "wall-clock budget exhausted")
        if attempts_left <= 0:
            return RetryDecision(False, 0, "attempts exhausted")
        if not is_retryable(error):
            return RetryDecision(False, 0, f"non-retryable: {type(error).__name__}")
        delay = backoff_delay_ms(
            self._policy,
            attempt_idx,
            prev_delay_ms=self._prev,
            random_source=self._random,
        )
        self._prev = delay
        return RetryDecision(True, delay, "backing off")
