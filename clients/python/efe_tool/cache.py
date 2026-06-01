"""Tiny thread-safe LRU cache with TTL for classification verdicts.

Keys: ``(catalog_id, catalog_version, text_hash)``.
Values: :class:`efe_tool.types.ClassifyResult`.

The TTL is a soft expiry: entries are checked lazily on read, never
with a background sweeper (we want the client to stay effects-free).
"""

from __future__ import annotations

import threading
import time
from collections import OrderedDict
from dataclasses import dataclass
from typing import Hashable

from .types import ClassifyResult


@dataclass(frozen=True)
class CacheKey:
    catalog_id: str
    catalog_version: str
    text_hash: str

    def as_tuple(self) -> tuple[str, str, str]:
        return (self.catalog_id, self.catalog_version, self.text_hash)


class LRUCache:
    """Bounded LRU + TTL cache, thread-safe.

    Not optimised for contention (single lock) but perfectly adequate
    for typical agent traffic (~10-50 rps per process).
    """

    def __init__(self, max_entries: int = 10_000, ttl_s: float = 3600.0) -> None:
        if max_entries <= 0:
            raise ValueError("max_entries must be > 0")
        if ttl_s <= 0:
            raise ValueError("ttl_s must be > 0")
        self._max = max_entries
        self._ttl = ttl_s
        self._lock = threading.Lock()
        self._data: OrderedDict[tuple[Hashable, ...], tuple[float, ClassifyResult]] = OrderedDict()
        self._hits = 0
        self._misses = 0

    def get(self, key: CacheKey) -> ClassifyResult | None:
        k = key.as_tuple()
        with self._lock:
            entry = self._data.get(k)
            if entry is None:
                self._misses += 1
                return None
            ts, val = entry
            if time.monotonic() - ts > self._ttl:
                # Expired — drop it.
                self._data.pop(k, None)
                self._misses += 1
                return None
            # Bump to MRU position.
            self._data.move_to_end(k)
            self._hits += 1
            return val

    def put(self, key: CacheKey, value: ClassifyResult) -> None:
        k = key.as_tuple()
        with self._lock:
            self._data[k] = (time.monotonic(), value)
            self._data.move_to_end(k)
            while len(self._data) > self._max:
                self._data.popitem(last=False)

    def invalidate_prefix(self, catalog_id: str) -> int:
        """Drop every entry whose catalog_id matches. Returns drop count."""
        with self._lock:
            victims = [k for k in self._data if k[0] == catalog_id]
            for k in victims:
                self._data.pop(k, None)
            return len(victims)

    def clear(self) -> None:
        with self._lock:
            self._data.clear()

    def stats(self) -> dict[str, int]:
        with self._lock:
            return {
                "hits": self._hits,
                "misses": self._misses,
                "size": len(self._data),
                "capacity": self._max,
            }
