"""Verbatim cache — exact hash match, tenant-partitioned."""

from __future__ import annotations

import time
from collections import OrderedDict
from dataclasses import dataclass
from typing import Protocol

from efe_inference.cache.key import CacheKey
from efe_inference.messages.response import ChatResponse


class VerbatimCache(Protocol):
    async def get(
        self, key: CacheKey,
    ) -> ChatResponse | None:  # pragma: no cover
        ...

    async def put(
        self, key: CacheKey, value: ChatResponse, ttl_s: int,
    ) -> None:  # pragma: no cover
        ...

    async def invalidate(
        self, tenant_id: str, task_id: str,
    ) -> int:  # pragma: no cover
        ...


@dataclass(frozen=True, slots=True)
class _Entry:
    value: ChatResponse
    expires_at_mono: float


class InMemoryVerbatimCache:
    """Thread-safe(ish) TTL LRU. Good for dev + tests; prod uses Redis."""

    def __init__(self, *, max_entries: int = 100_000) -> None:
        self._data: OrderedDict[tuple[str, str, str], _Entry] = OrderedDict()
        self._max = max_entries

    async def get(self, key: CacheKey) -> ChatResponse | None:
        tkey = (key.tenant_id, key.task_id, key.prompt_hash_hex)
        entry = self._data.get(tkey)
        if entry is None:
            return None
        now = time.monotonic()
        if entry.expires_at_mono <= now:
            self._data.pop(tkey, None)
            return None
        self._data.move_to_end(tkey)
        return entry.value

    async def put(
        self, key: CacheKey, value: ChatResponse, ttl_s: int,
    ) -> None:
        if ttl_s <= 0:
            return
        tkey = (key.tenant_id, key.task_id, key.prompt_hash_hex)
        self._data[tkey] = _Entry(
            value=value, expires_at_mono=time.monotonic() + float(ttl_s),
        )
        self._data.move_to_end(tkey)
        while len(self._data) > self._max:
            self._data.popitem(last=False)

    async def invalidate(self, tenant_id: str, task_id: str) -> int:
        dropped = 0
        for tkey in list(self._data.keys()):
            if tkey[0] == tenant_id and tkey[1] == task_id:
                self._data.pop(tkey, None)
                dropped += 1
        return dropped

    def size(self) -> int:
        return len(self._data)


__all__ = ["InMemoryVerbatimCache", "VerbatimCache"]
