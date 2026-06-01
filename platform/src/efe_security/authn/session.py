"""Opaque sessions — server-side identifier usable for instant revocation.

The client sends a random 256-bit token ; the server resolves it to a
:class:`Principal` via a :class:`SessionStore`. Stored tokens are hashed
via HMAC to mitigate DB exposure.
"""

from __future__ import annotations

import asyncio
from dataclasses import dataclass, field
from datetime import datetime, timedelta
from typing import Any, Protocol

from efe_security.context import Principal
from efe_security.crypto import (
    DOMAIN_TAG_SESSION_OPAQUE,
    csprng_bytes,
    hmac_seal,
)
from efe_security.errors import SessionRevoked

__all__ = [
    "OpaqueSession",
    "SessionStore",
    "new_session_token",
]


def new_session_token(nbytes: int = 32) -> str:
    return csprng_bytes(nbytes).hex()


def _hash_token(token: str) -> bytes:
    return hmac_seal(DOMAIN_TAG_SESSION_OPAQUE, {"token": token})


@dataclass(frozen=True, slots=True)
class OpaqueSession:
    session_id: str
    token_hash: bytes
    principal: Principal
    created_at: datetime
    expires_at: datetime
    revoked_at: datetime | None = None

    def is_active(self, *, now: datetime) -> bool:
        if self.revoked_at is not None:
            return False
        return now < self.expires_at


class _StoreProto(Protocol):
    async def issue(self, principal: Principal, *, ttl: timedelta, now: datetime) -> tuple[str, OpaqueSession]: ...  # pragma: no cover
    async def resolve(self, token: str, *, now: datetime) -> OpaqueSession: ...  # pragma: no cover
    async def revoke(self, session_id: str, *, now: datetime) -> None: ...  # pragma: no cover


@dataclass(slots=True)
class SessionStore:
    """In-memory implementation. Production ports to Redis."""

    _by_hash: dict[bytes, OpaqueSession] = field(default_factory=dict)
    _by_id: dict[str, bytes] = field(default_factory=dict)
    _lock: asyncio.Lock = field(default_factory=asyncio.Lock, repr=False)

    async def issue(
        self,
        principal: Principal,
        *,
        ttl: timedelta,
        now: datetime,
    ) -> tuple[str, OpaqueSession]:
        token = new_session_token()
        h = _hash_token(token)
        session = OpaqueSession(
            session_id=h[:16].hex(),
            token_hash=h,
            principal=principal,
            created_at=now,
            expires_at=now + ttl,
        )
        async with self._lock:
            self._by_hash[h] = session
            self._by_id[session.session_id] = h
        return token, session

    async def resolve(self, token: str, *, now: datetime) -> OpaqueSession:
        h = _hash_token(token)
        async with self._lock:
            session = self._by_hash.get(h)
        if session is None:
            raise SessionRevoked("unknown session")
        if not session.is_active(now=now):
            raise SessionRevoked("session expired or revoked")
        return session

    async def revoke(self, session_id: str, *, now: datetime) -> None:
        async with self._lock:
            h = self._by_id.get(session_id)
            if h is None:
                return
            session = self._by_hash.get(h)
            if session is None:
                return
            self._by_hash[h] = OpaqueSession(
                session_id=session.session_id,
                token_hash=session.token_hash,
                principal=session.principal,
                created_at=session.created_at,
                expires_at=session.expires_at,
                revoked_at=now,
            )


_ = Any
