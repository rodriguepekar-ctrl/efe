"""JWKS cache.

A JSON Web Key Set is refreshed periodically from an IdP. The cache
keeps the previous-version keys during a grace period so tokens signed
with an older ``kid`` still verify.

This module is I/O agnostic : the actual HTTP fetch is pluggable via
:class:`JwksSource` Protocol. The :class:`StaticJwksSource` is used in
tests.
"""

from __future__ import annotations

import asyncio
import logging
from dataclasses import dataclass, field
from datetime import datetime, timedelta
from typing import Any, Protocol

from efe_security.errors import CryptoError, InvalidKid

_LOG = logging.getLogger("efe_security.authn.jwks")

__all__ = ["Jwk", "JwksCache", "JwksSource", "StaticJwksSource"]


@dataclass(frozen=True, slots=True)
class Jwk:
    """Minimal JWK structure (alg_id-agnostic)."""

    kid: str
    alg: str  # e.g. "ES256", "EdDSA"
    key_bytes: bytes  # raw public key bytes (impl-specific encoding)
    extra: tuple[tuple[str, str], ...] = ()


class JwksSource(Protocol):
    async def fetch(self) -> list[Jwk]: ...  # pragma: no cover


@dataclass(slots=True)
class StaticJwksSource:
    """Test helper : serves a fixed list."""

    keys: tuple[Jwk, ...]

    async def fetch(self) -> list[Jwk]:
        return list(self.keys)


@dataclass(slots=True)
class JwksCache:
    """Async JWKS cache with TTL + grace window."""

    source: JwksSource
    ttl_s: int = 300
    grace_window_h: int = 24
    _entries: dict[str, tuple[Jwk, datetime]] = field(default_factory=dict, init=False)
    _last_refresh: datetime | None = field(default=None, init=False)
    _lock: asyncio.Lock = field(default_factory=asyncio.Lock, init=False, repr=False)

    async def get(self, kid: str, *, now: datetime) -> Jwk:
        async with self._lock:
            entry = self._entries.get(kid)
            if entry is not None and (now - entry[1]).total_seconds() < self.ttl_s:
                return entry[0]
        await self._refresh(now=now)
        async with self._lock:
            entry = self._entries.get(kid)
            if entry is None:
                raise InvalidKid(f"kid {kid!r} not found in JWKS")
            return entry[0]

    async def _refresh(self, *, now: datetime) -> None:
        async with self._lock:
            if (
                self._last_refresh is not None
                and (now - self._last_refresh).total_seconds() < self.ttl_s
            ):
                return
            try:
                keys = await self.source.fetch()
            except Exception as exc:  # pragma: no cover - best-effort refresh
                _LOG.warning("jwks.refresh_failed err=%s", exc)
                return
            # Evict only entries beyond the grace window so in-flight
            # tokens signed by a rotating IdP still verify briefly.
            horizon = now - timedelta(hours=self.grace_window_h)
            self._entries = {
                k: v for k, v in self._entries.items() if v[1] > horizon
            }
            # Merge new keys (new keys overwrite old with same kid).
            for k in keys:
                self._entries[k.kid] = (k, now)
            self._last_refresh = now
            _LOG.info("jwks.refreshed total_keys=%d", len(self._entries))

    def snapshot(self) -> list[Jwk]:
        return [entry[0] for entry in self._entries.values()]


# Silence type-checker for the Any import path.
_ = Any
_ = CryptoError
