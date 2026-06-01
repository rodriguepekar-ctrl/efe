"""Per-(token, tool) rate limiter (M16).

The MCP server is the closest enforcement point to the LLM client. A
runaway client (e.g. a model that loops on `efe_classify`) shouldn't be
able to flood the EFE backend ; the backend's own M11 rate limit will
catch it eventually but only after the LLM has burned tokens for nothing.

Algorithm : token bucket per (token_id, tool_name) tuple. `token_id` is
the SHA-256 of the raw bearer token (so we never store the token plain
text in memory). Defaults are conservative for a per-user MCP session :
100 req/min, capacity 100. Override via `EFE_MCP_RATE_LIMIT_PER_MIN`.

Single-process. Acceptable because each MCP server instance is a per-user
subprocess (stdio) or a per-tenant pod (HTTP+SSE). For multi-replica
deployments a Redis backend is provided in M19.
"""
from __future__ import annotations

import hashlib
import time
from dataclasses import dataclass
from threading import Lock
from typing import Optional


@dataclass
class _Bucket:
    capacity: float
    refill_per_sec: float
    current: float
    last_refill: float

    def try_consume(self, cost: float = 1.0, *, now: Optional[float] = None) -> bool:
        now = now if now is not None else time.time()
        elapsed = now - self.last_refill
        self.current = min(self.capacity, self.current + elapsed * self.refill_per_sec)
        self.last_refill = now
        if self.current >= cost:
            self.current -= cost
            return True
        return False

    def remaining(self) -> float:
        return round(max(0.0, self.current), 2)


class RateLimitExceeded(Exception):
    """Raised when the per-(token, tool) bucket is empty."""

    def __init__(
        self,
        message: str,
        *,
        retry_after_seconds: float,
        current_tokens: float,
        capacity: float,
    ) -> None:
        super().__init__(message)
        self.retry_after_seconds = retry_after_seconds
        self.current_tokens = current_tokens
        self.capacity = capacity

    def to_dict(self) -> dict[str, object]:
        return {
            "message": str(self),
            "retry_after_seconds": round(self.retry_after_seconds, 2),
            "current_tokens": round(self.current_tokens, 2),
            "capacity": self.capacity,
        }


class RateLimiter:
    """Per-(token, tool) token-bucket limiter.

    `per_min` is converted to (refill_per_sec = per_min / 60) and the
    capacity is set to `per_min` (i.e. one minute's worth of burst).
    """

    def __init__(self, *, per_min: int = 100) -> None:
        self.per_min = max(1, int(per_min))
        self._buckets: dict[str, _Bucket] = {}
        self._lock = Lock()

    @staticmethod
    def _hash_token(raw_token: str) -> str:
        return hashlib.sha256(raw_token.encode("utf-8")).hexdigest()[:16]

    def _key(self, raw_token: str, tool: str) -> str:
        return f"{self._hash_token(raw_token)}|{tool}"

    def check(
        self, *, raw_token: str, tool: str, cost: float = 1.0
    ) -> dict[str, object]:
        """Consume one token. Raises RateLimitExceeded if bucket is empty.

        Returns a small info dict on success (current_tokens, capacity)
        suitable for OTel attributes."""
        key = self._key(raw_token, tool)
        capacity = float(self.per_min)
        refill = capacity / 60.0
        with self._lock:
            bucket = self._buckets.get(key)
            if bucket is None:
                bucket = _Bucket(
                    capacity=capacity,
                    refill_per_sec=refill,
                    current=capacity,
                    last_refill=time.time(),
                )
                self._buckets[key] = bucket
            allowed = bucket.try_consume(cost)
            current = bucket.remaining()
            if not allowed:
                # How long until 1 token is available again ?
                deficit = max(0.0, cost - bucket.current)
                retry_after = deficit / bucket.refill_per_sec if bucket.refill_per_sec else 60.0
                raise RateLimitExceeded(
                    f"rate_limit_exceeded for tool {tool!r} : "
                    f"{self.per_min} requests/minute",
                    retry_after_seconds=retry_after,
                    current_tokens=current,
                    capacity=capacity,
                )
        return {
            "current_tokens": current,
            "capacity": capacity,
            "refill_per_sec": round(refill, 4),
        }

    def reset(self) -> None:
        """Clear all buckets — for tests."""
        with self._lock:
            self._buckets.clear()

    def stats(self) -> dict[str, object]:
        with self._lock:
            return {
                "buckets": len(self._buckets),
                "per_min": self.per_min,
            }
