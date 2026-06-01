"""Verbatim + semantic cache — partitioning, TTL, hit/miss."""

from __future__ import annotations

import pytest

from efe_inference.cache.embed import DeterministicEmbedder
from efe_inference.cache.key import CacheKeyBuilder
from efe_inference.cache.semantic import InMemorySemanticCache
from efe_inference.cache.verbatim import InMemoryVerbatimCache
from efe_inference.messages.request import (
    ChatMessage,
    ChatRequest,
    Role,
    TextBlock,
)
from efe_inference.messages.response import (
    ChatChoice,
    ChatResponse,
    FinishReason,
    Usage,
)


def _req(tenant: str, task: str, text: str) -> ChatRequest:
    return ChatRequest(
        tenant_id=tenant, task_id=task, request_id="r",
        messages=(
            ChatMessage(role=Role.USER, content=(TextBlock(text),)),
        ),
    )


def _resp(text: str = "ok") -> ChatResponse:
    return ChatResponse(
        request_id="r",
        model="mock",
        choices=(
            ChatChoice(
                index=0,
                message=ChatMessage(
                    role=Role.ASSISTANT,
                    content=(TextBlock(text),),
                ),
                finish_reason=FinishReason.STOP,
            ),
        ),
        usage=Usage(prompt_tokens=1, completion_tokens=1, total_tokens=2),
        backend_kind="mock",
        backend_latency_ms=1.0,
        adapter_overhead_ms=1.0,
    )


@pytest.mark.asyncio
async def test_verbatim_cache_hit():
    c = InMemoryVerbatimCache()
    kb = CacheKeyBuilder()
    r = _req("t", "q", "hi")
    k = kb.build(r)
    await c.put(k, _resp(), ttl_s=60)
    hit = await c.get(k)
    assert hit is not None


@pytest.mark.asyncio
async def test_verbatim_cache_miss():
    c = InMemoryVerbatimCache()
    kb = CacheKeyBuilder()
    hit = await c.get(kb.build(_req("t", "q", "hi")))
    assert hit is None


@pytest.mark.asyncio
async def test_verbatim_cache_tenant_partition():
    c = InMemoryVerbatimCache()
    kb = CacheKeyBuilder()
    r1 = _req("tenant-A", "q", "hi")
    r2 = _req("tenant-B", "q", "hi")
    await c.put(kb.build(r1), _resp("A"), ttl_s=60)
    assert await c.get(kb.build(r2)) is None  # different tenant


@pytest.mark.asyncio
async def test_semantic_cache_hit_close_enough():
    embedder = DeterministicEmbedder(dim=16)
    cache = InMemorySemanticCache(embedder=embedder)
    r = _req("t", "q", "What is the capital of France?")
    await cache.put(r, _resp("Paris"), ttl_s=60)
    hit = await cache.get(r, threshold=0.99)
    assert hit is not None
    assert hit.similarity >= 0.99


@pytest.mark.asyncio
async def test_semantic_cache_strict_tenant_partition():
    embedder = DeterministicEmbedder(dim=16)
    cache = InMemorySemanticCache(embedder=embedder)
    r_a = _req("tenant-A", "q", "question")
    r_b = _req("tenant-B", "q", "question")  # identical text
    await cache.put(r_a, _resp("A"), ttl_s=60)
    hit_b = await cache.get(r_b, threshold=0.0)
    assert hit_b is None  # different partition


@pytest.mark.asyncio
async def test_semantic_cache_invalidate_drops_index():
    embedder = DeterministicEmbedder(dim=16)
    cache = InMemorySemanticCache(embedder=embedder)
    r = _req("t", "q", "x")
    await cache.put(r, _resp(), ttl_s=60)
    dropped = await cache.invalidate("t", "q")
    assert dropped == 1
    assert await cache.get(r, threshold=0.0) is None


def test_cache_key_tenant_scoped():
    kb = CacheKeyBuilder()
    a = kb.build(_req("tenant-A", "q", "hi"))
    b = kb.build(_req("tenant-B", "q", "hi"))
    assert a.tenant_id != b.tenant_id
    # Same prompt → same hash, but redis_key differs.
    assert a.prompt_hash_hex == b.prompt_hash_hex
    assert a.redis_key() != b.redis_key()
