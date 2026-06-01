"""Service account registry — internal identities for pods / daemons.

A service account carries a fixed SPIFFE ID and a set of default scopes.
In production, rotation is driven by cert-manager (short-lived certs) ;
the registry only tracks metadata.
"""

from __future__ import annotations

import asyncio
from dataclasses import dataclass, field
from datetime import datetime

__all__ = ["ServiceAccount", "ServiceAccountRegistry"]


@dataclass(frozen=True, slots=True)
class ServiceAccount:
    name: str
    spiffe_id: str
    scopes: frozenset[str]
    created_at: datetime
    disabled_at: datetime | None = None
    labels: tuple[tuple[str, str], ...] = ()


@dataclass(slots=True)
class ServiceAccountRegistry:
    _rows: dict[str, ServiceAccount] = field(default_factory=dict)
    _lock: asyncio.Lock = field(default_factory=asyncio.Lock, repr=False)

    async def register(self, sa: ServiceAccount) -> ServiceAccount:
        async with self._lock:
            if sa.name in self._rows:
                raise ValueError(f"service_account already exists: {sa.name}")
            self._rows[sa.name] = sa
            return sa

    async def get(self, name: str) -> ServiceAccount | None:
        async with self._lock:
            return self._rows.get(name)

    async def list(self) -> list[ServiceAccount]:
        async with self._lock:
            return list(self._rows.values())

    async def disable(self, name: str, *, now: datetime) -> bool:
        async with self._lock:
            row = self._rows.get(name)
            if row is None:
                return False
            self._rows[name] = ServiceAccount(
                name=row.name,
                spiffe_id=row.spiffe_id,
                scopes=row.scopes,
                created_at=row.created_at,
                disabled_at=now,
                labels=row.labels,
            )
            return True
