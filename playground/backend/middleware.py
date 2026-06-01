"""
EFE Playground — middleware utilities.

Audit fix M15-A3 — token-bucket rate limit per tenant (M11).
Audit fix M15-A4 — PII hash mode (D13).
Audit fix M15-A6 — idempotency cache (M11).

Production EFE uses Redis-backed implementations ; playground uses
in-memory + thread-local for offline operation.
"""
from __future__ import annotations

import collections
import hashlib
import re
import time
from dataclasses import dataclass
from typing import Optional


# ─────────────────────────────────────────────────────────────────────
# A3 — Token-bucket rate limit per tenant (M11)
# ─────────────────────────────────────────────────────────────────────

# Conservative defaults for the playground (production is per-tenant tunable).
DEFAULT_CAPACITY = 50           # max burst per tenant
DEFAULT_REFILL_PER_SEC = 10.0   # 10 req/sec sustained


@dataclass
class TokenBucket:
    capacity: float
    refill_per_sec: float
    current: float
    last_refill: float

    def allow(self, cost: float = 1.0) -> bool:
        now = time.time()
        elapsed = now - self.last_refill
        self.current = min(self.capacity, self.current + elapsed * self.refill_per_sec)
        self.last_refill = now
        if self.current >= cost:
            self.current -= cost
            return True
        return False


class RateLimiter:
    """Per-tenant token-bucket rate limiter.

    Mirror of M11 rate limit pattern. Single-process, no Redis.
    """

    def __init__(
        self,
        *,
        default_capacity: float = DEFAULT_CAPACITY,
        default_refill_per_sec: float = DEFAULT_REFILL_PER_SEC,
    ) -> None:
        self.default_capacity = default_capacity
        self.default_refill_per_sec = default_refill_per_sec
        self._buckets: dict[str, TokenBucket] = {}

    def check(self, tenant_id: str, cost: float = 1.0) -> tuple[bool, dict]:
        """Returns (allowed, info_dict).

        `info_dict` contains tenant_id, current_tokens, refill_per_sec.
        """
        if tenant_id not in self._buckets:
            self._buckets[tenant_id] = TokenBucket(
                capacity=self.default_capacity,
                refill_per_sec=self.default_refill_per_sec,
                current=self.default_capacity,
                last_refill=time.time(),
            )
        bucket = self._buckets[tenant_id]
        allowed = bucket.allow(cost)
        return allowed, {
            "tenant_id": tenant_id,
            "current_tokens": round(bucket.current, 2),
            "capacity": bucket.capacity,
            "refill_per_sec": bucket.refill_per_sec,
        }


# ─────────────────────────────────────────────────────────────────────
# A4 — PII hash mode (D13)
# ─────────────────────────────────────────────────────────────────────

# Regexes mirror `crates/efe-server/src/observability/pii_hash.rs`.
# These are heuristics — not exhaustive ; production uses a richer set.

_EMAIL_RE = re.compile(r'\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Z|a-z]{2,}\b')
_PHONE_RE = re.compile(r'\b(?:\+?\d{1,3}[\s.-]?)?\(?\d{3}\)?[\s.-]?\d{3}[\s.-]?\d{4}\b')
_SSN_RE = re.compile(r'\b\d{3}-\d{2}-\d{4}\b')
_CC_RE = re.compile(r'\b(?:\d[ -]*?){13,19}\b')


def hash_pii_token(value: str) -> str:
    """Replace value with `<PII_HASH:abc12345>` (8 hex chars from SHA-256)."""
    h = hashlib.sha256(value.encode("utf-8")).hexdigest()[:8]
    return f"<PII_HASH:{h}>"


def scrub_pii(text: str) -> tuple[str, dict]:
    """Returns (scrubbed_text, scrub_info).

    scrub_info has counts per category : email, phone, ssn, credit_card.
    """
    counts = {"email": 0, "phone": 0, "ssn": 0, "credit_card": 0}
    out = text
    out, n = _EMAIL_RE.subn(lambda m: hash_pii_token(m.group(0)), out)
    counts["email"] = n
    out, n = _SSN_RE.subn(lambda m: hash_pii_token(m.group(0)), out)
    counts["ssn"] = n
    out, n = _CC_RE.subn(lambda m: hash_pii_token(m.group(0)), out)
    counts["credit_card"] = n
    out, n = _PHONE_RE.subn(lambda m: hash_pii_token(m.group(0)), out)
    counts["phone"] = n
    return out, counts


# ─────────────────────────────────────────────────────────────────────
# A6 — Idempotency cache (M11)
# ─────────────────────────────────────────────────────────────────────


@dataclass
class IdempotencyEntry:
    response: dict
    status_code: int
    timestamp: float


class IdempotencyCache:
    """LRU + TTL idempotency cache.

    Mirror of M11 idempotency pattern. Client supplies `Idempotency-Key`
    header ; identical key + identical body within TTL returns cached
    response with `X-EFE-Idempotency-Replay: true`.
    """

    def __init__(
        self,
        *,
        ttl_seconds: float = 300.0,   # 5 min default per M11
        capacity: int = 5000,
    ) -> None:
        self.ttl_seconds = ttl_seconds
        self.capacity = capacity
        self._store: collections.OrderedDict[str, IdempotencyEntry] = (
            collections.OrderedDict()
        )

    def _make_key(
        self, tenant_id: str, idempotency_key: str, body_hash: str
    ) -> str:
        return f"{tenant_id}|{idempotency_key}|{body_hash}"

    def lookup(
        self, *, tenant_id: str, idempotency_key: str, body: bytes
    ) -> Optional[IdempotencyEntry]:
        if not idempotency_key:
            return None
        body_hash = hashlib.sha256(body).hexdigest()[:16]
        key = self._make_key(tenant_id, idempotency_key, body_hash)
        entry = self._store.get(key)
        if entry is None:
            return None
        # Check TTL
        if time.time() - entry.timestamp > self.ttl_seconds:
            del self._store[key]
            return None
        # Move to end (LRU)
        self._store.move_to_end(key)
        return entry

    def store(
        self,
        *,
        tenant_id: str,
        idempotency_key: str,
        body: bytes,
        response: dict,
        status_code: int,
    ) -> None:
        if not idempotency_key:
            return
        body_hash = hashlib.sha256(body).hexdigest()[:16]
        key = self._make_key(tenant_id, idempotency_key, body_hash)
        self._store[key] = IdempotencyEntry(
            response=response,
            status_code=status_code,
            timestamp=time.time(),
        )
        # Evict oldest if over capacity
        while len(self._store) > self.capacity:
            self._store.popitem(last=False)

    def stats(self) -> dict:
        return {
            "entries": len(self._store),
            "capacity": self.capacity,
            "ttl_seconds": self.ttl_seconds,
        }
