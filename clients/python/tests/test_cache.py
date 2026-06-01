"""Tests for the LRU+TTL cache."""

from __future__ import annotations

import time

import pytest

from efe_tool.cache import CacheKey, LRUCache
from efe_tool.types import ClassifyResult


def _result(label: str = "x") -> ClassifyResult:
    return ClassifyResult(
        label=label,
        confidence=0.9,
        alignment_delta=0.1,
        rounds=5,
        reject=False,
        swarm_id="sid",
        request_id="rid",
        text_hash="hash",
        catalog_version="1",
    )


def test_miss_then_hit() -> None:
    c = LRUCache(max_entries=10, ttl_s=60)
    k = CacheKey("cat", "1", "h1")
    assert c.get(k) is None
    c.put(k, _result("a"))
    assert c.get(k).label == "a"
    stats = c.stats()
    assert stats["hits"] == 1 and stats["misses"] == 1


def test_lru_eviction() -> None:
    c = LRUCache(max_entries=2, ttl_s=60)
    c.put(CacheKey("cat", "1", "a"), _result("a"))
    c.put(CacheKey("cat", "1", "b"), _result("b"))
    c.put(CacheKey("cat", "1", "c"), _result("c"))  # evicts "a"
    assert c.get(CacheKey("cat", "1", "a")) is None
    assert c.get(CacheKey("cat", "1", "b")).label == "b"
    assert c.get(CacheKey("cat", "1", "c")).label == "c"


def test_ttl_expiry() -> None:
    c = LRUCache(max_entries=10, ttl_s=0.05)
    k = CacheKey("cat", "1", "h1")
    c.put(k, _result("a"))
    assert c.get(k).label == "a"
    time.sleep(0.06)
    assert c.get(k) is None


def test_invalidate_prefix() -> None:
    c = LRUCache(max_entries=10, ttl_s=60)
    c.put(CacheKey("cat_a", "1", "h1"), _result("a"))
    c.put(CacheKey("cat_a", "1", "h2"), _result("b"))
    c.put(CacheKey("cat_b", "1", "h3"), _result("c"))
    assert c.invalidate_prefix("cat_a") == 2
    assert c.get(CacheKey("cat_a", "1", "h1")) is None
    assert c.get(CacheKey("cat_b", "1", "h3")).label == "c"


def test_rejects_invalid_construction() -> None:
    with pytest.raises(ValueError):
        LRUCache(max_entries=0)
    with pytest.raises(ValueError):
        LRUCache(max_entries=10, ttl_s=0)
