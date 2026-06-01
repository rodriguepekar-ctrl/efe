"""Key registry — stores metadata about cryptographic keys.

The actual material lives in the KMS ; the registry only tracks :
``key_id``, ``alg_id``, ``purpose``, ``state``, ``key_version``, dates.

CAS is applied on every transition via ``expected_version``.
"""

from __future__ import annotations

import asyncio
import enum
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Protocol

from efe_security.errors import KeyNotFound, SecurityError
from efe_security.fsm import KEY_ALLOWED, guard

__all__ = [
    "KeyAlreadyRegistered",
    "KeyEntry",
    "KeyPurpose",
    "KeyRegistry",
    "KeyState",
]


class KeyAlreadyRegistered(SecurityError):
    """Raised when :meth:`KeyRegistry.register` receives a known ``key_id``.

    Dedicated class so callers can handle re-registration specifically
    (idempotent onboard, for instance) without swallowing unrelated
    :class:`SecurityError` instances.
    """

    code = "crypto.key_already_registered"
    http_status = 409


class KeyPurpose(str, enum.Enum):
    tenant_dek_kek = "tenant_dek_kek"
    audit_chain = "audit_chain"
    jwt_signing_service = "jwt_signing_service"
    consent_signing = "consent_signing"
    dsar_receipt = "dsar_receipt"
    incident_seal = "incident_seal"
    generic_hmac = "generic_hmac"


class KeyState(str, enum.Enum):
    active = "active"
    retiring = "retiring"
    retired = "retired"


@dataclass(frozen=True, slots=True)
class KeyEntry:
    key_id: str
    alg_id: str
    purpose: KeyPurpose
    state: KeyState
    created_at: datetime
    rotated_from: str | None = None
    retired_at: datetime | None = None
    state_version: int = 1
    metadata: tuple[tuple[str, str], ...] = ()

    def __post_init__(self) -> None:
        if self.created_at.tzinfo is None:
            raise ValueError("created_at must be tz-aware")
        if self.retired_at is not None and self.retired_at.tzinfo is None:
            raise ValueError("retired_at must be tz-aware")


class _Registry(Protocol):
    async def register(self, entry: KeyEntry) -> KeyEntry: ...  # pragma: no cover
    async def get(self, key_id: str) -> KeyEntry: ...  # pragma: no cover
    async def transition(
        self, key_id: str, to_state: KeyState, *, expected_version: int, now: datetime
    ) -> int: ...  # pragma: no cover
    async def list_active(self, *, purpose: KeyPurpose | None = None) -> list[KeyEntry]: ...  # pragma: no cover


class KeyRegistry:
    """In-memory key registry. Production impl persists to Postgres."""

    __slots__ = ("_lock", "_rows")

    def __init__(self) -> None:
        self._rows: dict[str, KeyEntry] = {}
        self._lock = asyncio.Lock()

    async def register(self, entry: KeyEntry) -> KeyEntry:
        async with self._lock:
            if entry.key_id in self._rows:
                raise KeyAlreadyRegistered(
                    f"key_id already registered: {entry.key_id}"
                )
            self._rows[entry.key_id] = entry
            return entry

    async def get(self, key_id: str) -> KeyEntry:
        async with self._lock:
            row = self._rows.get(key_id)
            if row is None:
                raise KeyNotFound(key_id)
            return row

    async def transition(
        self,
        key_id: str,
        to_state: KeyState,
        *,
        expected_version: int,
        now: datetime,
    ) -> int:
        async with self._lock:
            row = self._rows.get(key_id)
            if row is None:
                raise KeyNotFound(key_id)
            if row.state_version != expected_version:
                from efe_security.errors import SecurityError as _SE

                raise _SE(
                    f"key {key_id}: version mismatch "
                    f"expected={expected_version} actual={row.state_version}"
                )
            guard(row.state.value, to_state.value, KEY_ALLOWED, aggregate=f"key {key_id}")
            updated = KeyEntry(
                key_id=row.key_id,
                alg_id=row.alg_id,
                purpose=row.purpose,
                state=to_state,
                created_at=row.created_at,
                rotated_from=row.rotated_from,
                retired_at=now if to_state == KeyState.retired else row.retired_at,
                state_version=row.state_version + 1,
                metadata=row.metadata,
            )
            self._rows[key_id] = updated
            return updated.state_version

    async def list_active(self, *, purpose: KeyPurpose | None = None) -> list[KeyEntry]:
        async with self._lock:
            return [
                r
                for r in self._rows.values()
                if r.state == KeyState.active
                and (purpose is None or r.purpose == purpose)
            ]

    async def list_all(self) -> list[KeyEntry]:
        async with self._lock:
            return list(self._rows.values())
