"""LRU+TTL cache for OCI bundle artefacts (M18 D47).

Bundles are immutable in OCI — once published with a given digest,
their content never changes. So we can cache aggressively. Default
capacity 32 bundles + TTL 600 s ; both overridable via env.

Per-tenant scoping enforced by including `tenant_id` in the key
(D11 carry-over).
"""
from __future__ import annotations

import collections
import hashlib
import os
import time
from threading import Lock
from typing import Any


class BundleCache:
    def __init__(
        self,
        *,
        capacity: int | None = None,
        ttl_seconds: float | None = None,
    ) -> None:
        self.capacity = (
            capacity if capacity is not None
            else int(os.environ.get("EFE_MCP_BUNDLE_CACHE_CAP", "32"))
        )
        self.ttl_seconds = (
            ttl_seconds if ttl_seconds is not None
            else float(os.environ.get("EFE_MCP_BUNDLE_CACHE_TTL", "600"))
        )
        self._store: collections.OrderedDict[str, tuple[Any, float]] = (
            collections.OrderedDict()
        )
        self._lock = Lock()
        self._hits = 0
        self._misses = 0

    @staticmethod
    def _make_key(
        tenant_id: str, slug: str, version: str, namespace: str = "default",
    ) -> str:
        """Audit fix M18-D49-A2 (HIGH) : `namespace` argument added so
        different consumers (inspect_bundle tool vs efe://bundles
        resource handler) can store DIFFERENT result shapes under the
        same (tenant, slug, version) tuple without colliding."""
        return hashlib.sha256(
            f"{tenant_id}|{slug}|{version}|{namespace}".encode("utf-8")
        ).hexdigest()[:32]

    def get(
        self, *, tenant_id: str, slug: str, version: str,
        namespace: str = "default",
    ) -> Any | None:
        key = self._make_key(tenant_id, slug, version, namespace)
        with self._lock:
            item = self._store.get(key)
            if item is None:
                self._misses += 1
                return None
            value, ts = item
            if time.time() - ts > self.ttl_seconds:
                del self._store[key]
                self._misses += 1
                return None
            self._store.move_to_end(key)
            self._hits += 1
            return value

    def put(
        self,
        *,
        tenant_id: str,
        slug: str,
        version: str,
        value: Any,
        namespace: str = "default",
    ) -> None:
        key = self._make_key(tenant_id, slug, version, namespace)
        with self._lock:
            self._store[key] = (value, time.time())
            self._store.move_to_end(key)
            while len(self._store) > self.capacity:
                self._store.popitem(last=False)

    def invalidate(
        self, *, tenant_id: str, slug: str, version: str,
        namespace: str = "default",
    ) -> None:
        key = self._make_key(tenant_id, slug, version, namespace)
        with self._lock:
            self._store.pop(key, None)

    def clear(self) -> None:
        with self._lock:
            self._store.clear()
            self._hits = 0
            self._misses = 0

    def stats(self) -> dict:
        with self._lock:
            total = self._hits + self._misses
            hit_rate = (self._hits / total) if total else 0.0
            return {
                "entries": len(self._store),
                "capacity": self.capacity,
                "ttl_seconds": self.ttl_seconds,
                "hits": self._hits,
                "misses": self._misses,
                "hit_rate": round(hit_rate, 4),
            }
