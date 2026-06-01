"""ContractRegistry — register, lookup, deprecate (input, output) contracts."""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timedelta
from typing import Any

from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker

from efe_registry.clock import Clock, SystemClock
from efe_registry.config import ContractSettings
from efe_registry.contracts.canonicalizer import canonicalize_schema
from efe_registry.contracts.hasher import contract_hash_from_schemas
from efe_registry.db.models import ArtifactContract, Contract
from efe_registry.errors import (
    ContractNotFound,
    ContractSunset,
)


@dataclass(frozen=True, slots=True)
class ContractRecord:
    contract_hash: bytes
    registered_at: datetime
    deprecated_at: datetime | None
    sunset_at: datetime | None
    description: str | None


class ContractRegistry:
    def __init__(
        self,
        *,
        session_factory: async_sessionmaker[AsyncSession],
        settings: ContractSettings,
        clock: Clock | None = None,
    ) -> None:
        self._sf = session_factory
        self._cfg = settings
        self._clock: Clock = clock or SystemClock()

    # ------------------------------------------------------------ register

    async def register(
        self,
        *,
        input_schema: Any,
        output_schema: Any,
        description: str | None = None,
    ) -> bytes:
        h = contract_hash_from_schemas(input_schema, output_schema)
        async with self._sf() as session:
            existing = await session.get(Contract, h)
            if existing is not None:
                return h
            session.add(Contract(
                contract_hash=h,
                input_schema_canonical=canonicalize_schema(input_schema),
                output_schema_canonical=canonicalize_schema(output_schema),
                registered_at=self._clock.now(),
                description=description,
            ))
            await session.commit()
        return h

    # ------------------------------------------------------------ lookup

    async def resolve(self, contract_hash: bytes) -> ContractRecord:
        async with self._sf() as session:
            row = await session.get(Contract, contract_hash)
            if row is None:
                raise ContractNotFound(
                    f"contract {contract_hash.hex()[:16]}… not registered",
                )
            if row.sunset_at is not None and row.sunset_at <= self._clock.now():
                raise ContractSunset(
                    f"contract {contract_hash.hex()[:16]}… has sunset on "
                    f"{row.sunset_at.isoformat()}",
                )
            return ContractRecord(
                contract_hash=bytes(row.contract_hash),
                registered_at=row.registered_at,
                deprecated_at=row.deprecated_at,
                sunset_at=row.sunset_at,
                description=row.description,
            )

    # ------------------------------------------------------------ deprecate

    async def deprecate(self, contract_hash: bytes) -> ContractRecord:
        async with self._sf() as session:
            row = await session.get(Contract, contract_hash)
            if row is None:
                raise ContractNotFound(
                    f"contract {contract_hash.hex()[:16]}… not registered",
                )
            now = self._clock.now()
            if row.deprecated_at is None:
                row.deprecated_at = now
                row.sunset_at = now + timedelta(
                    days=self._cfg.deprecation_grace_days,
                )
            await session.commit()
            return ContractRecord(
                contract_hash=bytes(row.contract_hash),
                registered_at=row.registered_at,
                deprecated_at=row.deprecated_at,
                sunset_at=row.sunset_at,
                description=row.description,
            )

    # ------------------------------------------------------------ bind

    async def bind_artifact(
        self, *, artifact_id: str, contract_hash: bytes,
    ) -> None:
        async with self._sf() as session:
            # Idempotent — primary key enforces uniqueness.
            existing = await session.get(
                ArtifactContract, (artifact_id, contract_hash),
            )
            if existing is not None:
                return
            session.add(ArtifactContract(
                artifact_id=artifact_id,
                contract_hash=contract_hash,
            ))
            await session.commit()

    async def artifacts_for_contract(
        self, contract_hash: bytes,
    ) -> tuple[str, ...]:
        async with self._sf() as session:
            stmt = (
                select(ArtifactContract.artifact_id)
                .where(ArtifactContract.contract_hash == contract_hash)
            )
            rows = list((await session.execute(stmt)).all())
            return tuple(str(r[0]) for r in rows)


__all__ = ["ContractRecord", "ContractRegistry"]
