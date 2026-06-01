"""Semantic cache — embedding + cosine threshold + strict tenant partition.

Production implementation would use RediSearch (vector index). For
MVP + tests we ship :class:`InMemorySemanticCache` that performs a
linear scan — O(N) but OK for N < 10k. Both implementations satisfy
the same :class:`SemanticCache` protocol.
"""

from __future__ import annotations

import math
import time
from collections.abc import Sequence
from dataclasses import dataclass
from typing import Protocol

from efe_inference.cache.embed import Embedder
from efe_inference.cache.key import CacheKeyBuilder
from efe_inference.messages.request import ChatRequest
from efe_inference.messages.response import ChatResponse


@dataclass(frozen=True, slots=True)
class SemanticHit:
    response: ChatResponse
    similarity: float


class SemanticCache(Protocol):
    async def get(
        self, req: ChatRequest, *, threshold: float,
    ) -> SemanticHit | None:  # pragma: no cover
        ...

    async def put(
        self, req: ChatRequest, resp: ChatResponse, *, ttl_s: int,
    ) -> None:  # pragma: no cover
        ...

    async def invalidate(
        self, tenant_id: str, task_id: str,
    ) -> int:  # pragma: no cover
        ...


@dataclass(slots=True)
class _Entry:
    embedding: list[float]
    response: ChatResponse
    expires_at_mono: float


class InMemorySemanticCache:
    """Partitioned by ``(tenant, task)`` — never crosses boundaries."""

    def __init__(
        self,
        embedder: Embedder,
        *,
        key_builder: CacheKeyBuilder | None = None,
        max_entries_per_task: int = 10_000,
    ) -> None:
        self._embedder = embedder
        self._kb = key_builder or CacheKeyBuilder()
        self._max = max_entries_per_task
        self._store: dict[tuple[str, str], list[_Entry]] = {}

    async def get(
        self, req: ChatRequest, *, threshold: float,
    ) -> SemanticHit | None:
        tkey = (req.tenant_id, req.task_id)
        bucket = self._store.get(tkey)
        if not bucket:
            return None
        now = time.monotonic()
        # purge expired in place
        bucket[:] = [e for e in bucket if e.expires_at_mono > now]
        if not bucket:
            return None
        embedding = (await self._embedder.embed([req.total_text()]))[0]
        best: _Entry | None = None
        best_sim = -1.0
        for entry in bucket:
            sim = _cosine(embedding, entry.embedding)
            if sim > best_sim:
                best_sim = sim
                best = entry
        if best is None or best_sim < threshold:
            return None
        return SemanticHit(response=best.response, similarity=best_sim)

    async def put(
        self, req: ChatRequest, resp: ChatResponse, *, ttl_s: int,
    ) -> None:
        if ttl_s <= 0:
            return
        tkey = (req.tenant_id, req.task_id)
        bucket = self._store.setdefault(tkey, [])
        embedding = (await self._embedder.embed([req.total_text()]))[0]
        bucket.append(_Entry(
            embedding=embedding,
            response=resp,
            expires_at_mono=time.monotonic() + float(ttl_s),
        ))
        # Evict oldest once over capacity.
        while len(bucket) > self._max:
            bucket.pop(0)

    async def invalidate(self, tenant_id: str, task_id: str) -> int:
        tkey = (tenant_id, task_id)
        bucket = self._store.pop(tkey, None)
        return len(bucket) if bucket else 0

    def size(self, tenant_id: str, task_id: str) -> int:
        return len(self._store.get((tenant_id, task_id), []))


def _cosine(a: Sequence[float], b: Sequence[float]) -> float:
    if len(a) != len(b) or not a:
        return 0.0
    dot = sum(x * y for x, y in zip(a, b, strict=True))
    na = math.sqrt(sum(x * x for x in a)) or 1.0
    nb = math.sqrt(sum(y * y for y in b)) or 1.0
    return dot / (na * nb)


__all__ = ["InMemorySemanticCache", "SemanticCache", "SemanticHit"]
