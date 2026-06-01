"""Retry classifier + backoff helper."""

from __future__ import annotations

import random

from efe_swarm_integration.config import RetryConfig

__all__ = ["RetryClassifier", "backoff_delay_ms"]


class RetryClassifier:
    """Decide whether to retry a gRPC call based on status code."""

    __slots__ = ("_cfg",)

    def __init__(self, cfg: RetryConfig) -> None:
        self._cfg = cfg

    def should_retry(
        self, *, status: str, attempt: int, idempotent: bool
    ) -> bool:
        s = status.upper()
        if s in ("UNAVAILABLE", "SERVICE_UNAVAILABLE"):
            return attempt < self._cfg.max_attempts_unavailable
        if s in ("RESOURCE_EXHAUSTED", "QUOTA_EXCEEDED"):
            return attempt < self._cfg.max_attempts_resource_exhausted
        if s == "DEADLINE_EXCEEDED":
            return idempotent and attempt < self._cfg.max_attempts_deadline
        return False


def backoff_delay_ms(
    attempt: int,
    *,
    cfg: RetryConfig,
    rng: random.Random | None = None,
) -> int:
    """Decorrelated-jitter backoff in ms."""
    if attempt < 1:
        raise ValueError("attempt must be >= 1")
    base = cfg.base_delay_ms * (2 ** (attempt - 1))
    capped = min(cfg.max_delay_ms, base)
    rng = rng or random.Random()
    return int(capped / 2 + rng.random() * (capped / 2))
