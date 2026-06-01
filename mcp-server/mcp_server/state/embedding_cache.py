"""LRU+TTL cache for text embeddings (M17).

Per-tenant scoping is enforced by including `tenant_id` in the key.
Capacity 1024 + TTL 300 s by default ; both overridable via env.
"""
from __future__ import annotations

import collections
import hashlib
import os
import time
from threading import Lock

import numpy as np


class EmbeddingCache:
    def __init__(
        self,
        *,
        capacity: int | None = None,
        ttl_seconds: float | None = None,
    ) -> None:
        self.capacity = (
            capacity if capacity is not None
            else int(os.environ.get("EFE_MCP_EMBEDDING_CACHE_CAP", "1024"))
        )
        self.ttl_seconds = (
            ttl_seconds if ttl_seconds is not None
            else float(os.environ.get("EFE_MCP_EMBEDDING_CACHE_TTL", "300"))
        )
        self._store: collections.OrderedDict[str, tuple[np.ndarray, float]] = (
            collections.OrderedDict()
        )
        self._lock = Lock()
        self._hits = 0
        self._misses = 0

    @staticmethod
    def _make_key(tenant_id: str, catalog: str, text: str) -> str:
        return hashlib.sha256(
            f"{tenant_id}|{catalog}|{text}".encode("utf-8")
        ).hexdigest()[:32]

    def get(self, *, tenant_id: str, catalog: str, text: str) -> np.ndarray | None:
        key = self._make_key(tenant_id, catalog, text)
        with self._lock:
            item = self._store.get(key)
            if item is None:
                self._misses += 1
                return None
            embedding, ts = item
            if time.time() - ts > self.ttl_seconds:
                del self._store[key]
                self._misses += 1
                return None
            self._store.move_to_end(key)
            self._hits += 1
            return embedding

    def put(
        self,
        *,
        tenant_id: str,
        catalog: str,
        text: str,
        embedding: np.ndarray,
    ) -> None:
        key = self._make_key(tenant_id, catalog, text)
        with self._lock:
            self._store[key] = (embedding, time.time())
            self._store.move_to_end(key)
            while len(self._store) > self.capacity:
                self._store.popitem(last=False)

    def clear(self) -> None:
        with self._lock:
            self._store.clear()
            self._hits = 0
            self._misses = 0

    def stats(self) -> dict:
        with self._lock:
            return {
                "entries": len(self._store),
                "capacity": self.capacity,
                "ttl_seconds": self.ttl_seconds,
                "hits": self._hits,
                "misses": self._misses,
            }
