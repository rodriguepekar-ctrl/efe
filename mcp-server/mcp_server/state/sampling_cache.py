"""LRU+TTL cache for sampling responses (M17).

Capacity 64 + TTL 60 s by default. Smaller than embedding cache because
sampling responses are larger and less frequently re-used in a single
session. Per-tenant scoping enforced via tenant_id in key.
"""
from __future__ import annotations

import collections
import hashlib
import os
import time
from threading import Lock


class SamplingCache:
    def __init__(
        self,
        *,
        capacity: int | None = None,
        ttl_seconds: float | None = None,
    ) -> None:
        self.capacity = (
            capacity if capacity is not None
            else int(os.environ.get("EFE_MCP_SAMPLING_CACHE_CAP", "64"))
        )
        self.ttl_seconds = (
            ttl_seconds if ttl_seconds is not None
            else float(os.environ.get("EFE_MCP_SAMPLING_CACHE_TTL", "60"))
        )
        self._store: collections.OrderedDict[str, tuple[str, float]] = (
            collections.OrderedDict()
        )
        self._lock = Lock()

    @staticmethod
    def _make_key(tenant_id: str, prompt: str, model_hint: str | None) -> str:
        return hashlib.sha256(
            f"{tenant_id}|{model_hint or ''}|{prompt}".encode("utf-8")
        ).hexdigest()[:32]

    def get(self, *, tenant_id: str, prompt: str, model_hint: str | None) -> str | None:
        key = self._make_key(tenant_id, prompt, model_hint)
        with self._lock:
            item = self._store.get(key)
            if item is None:
                return None
            value, ts = item
            if time.time() - ts > self.ttl_seconds:
                del self._store[key]
                return None
            self._store.move_to_end(key)
            return value

    def put(
        self, *, tenant_id: str, prompt: str, model_hint: str | None, response: str
    ) -> None:
        key = self._make_key(tenant_id, prompt, model_hint)
        with self._lock:
            self._store[key] = (response, time.time())
            self._store.move_to_end(key)
            while len(self._store) > self.capacity:
                self._store.popitem(last=False)

    def clear(self) -> None:
        with self._lock:
            self._store.clear()

    def stats(self) -> dict:
        with self._lock:
            return {
                "entries": len(self._store),
                "capacity": self.capacity,
                "ttl_seconds": self.ttl_seconds,
            }
