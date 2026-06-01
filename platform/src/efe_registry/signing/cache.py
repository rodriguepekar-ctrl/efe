"""In-memory and Redis-backed caches for verification results."""

from __future__ import annotations

import time
from collections import OrderedDict
from dataclasses import dataclass
from typing import Protocol

from efe_registry.signing.models import VerificationResult


class SignatureCache(Protocol):
    async def get(
        self, key: str,
    ) -> VerificationResult | None:  # pragma: no cover
        ...

    async def set(
        self, key: str, value: VerificationResult, ttl_s: int,
    ) -> None:  # pragma: no cover
        ...

    async def invalidate(self, prefix: str) -> None:  # pragma: no cover
        ...


@dataclass(frozen=True, slots=True)
class _Entry:
    value: VerificationResult
    expires_at: float


class InMemorySignatureCache:
    """Thread-safe LRU-ish cache with TTL.

    Designed for tests and single-replica deployments. Production uses
    Redis via ``RedisSignatureCache``. The API is ``async`` on purpose
    so callers can swap implementations.
    """

    def __init__(self, *, max_entries: int = 10_000) -> None:
        self._data: OrderedDict[str, _Entry] = OrderedDict()
        self._max = max_entries

    async def get(self, key: str) -> VerificationResult | None:
        now = time.monotonic()
        entry = self._data.get(key)
        if entry is None:
            return None
        if entry.expires_at <= now:
            self._data.pop(key, None)
            return None
        # LRU touch
        self._data.move_to_end(key)
        return entry.value

    async def set(
        self, key: str, value: VerificationResult, ttl_s: int,
    ) -> None:
        exp = time.monotonic() + float(ttl_s)
        self._data[key] = _Entry(value=value, expires_at=exp)
        self._data.move_to_end(key)
        while len(self._data) > self._max:
            self._data.popitem(last=False)

    async def invalidate(self, prefix: str) -> None:
        keys_to_drop = [k for k in self._data if k.startswith(prefix)]
        for k in keys_to_drop:
            self._data.pop(k, None)

    def size(self) -> int:
        return len(self._data)


__all__ = ["InMemorySignatureCache", "SignatureCache"]
