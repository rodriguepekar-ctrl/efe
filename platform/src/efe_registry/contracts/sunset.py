"""DeprecationSweeper — raises alerts before sunset, purges after."""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Protocol

from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker

from efe_registry.clock import Clock, SystemClock
from efe_registry.config import ContractSettings
from efe_registry.db.models import Contract


def _as_utc(dt: datetime) -> datetime:
    """SQLite strips tzinfo on JSON deserialisation — re-attach UTC."""
    if dt.tzinfo is None:
        return dt.replace(tzinfo=timezone.utc)
    return dt.astimezone(timezone.utc)


@dataclass(frozen=True, slots=True)
class SweeperAlert:
    contract_hash: bytes
    sunset_at: datetime
    days_remaining: int


class AlertSink(Protocol):
    async def notify(
        self, alerts: tuple[SweeperAlert, ...],
    ) -> None:  # pragma: no cover
        ...


@dataclass
class InMemoryAlertSink:
    received: list[SweeperAlert] = None  # type: ignore[assignment]

    def __post_init__(self) -> None:
        if self.received is None:
            self.received = []

    async def notify(self, alerts: tuple[SweeperAlert, ...]) -> None:
        self.received.extend(alerts)


class DeprecationSweeper:
    def __init__(
        self,
        *,
        session_factory: async_sessionmaker[AsyncSession],
        settings: ContractSettings,
        sink: AlertSink,
        clock: Clock | None = None,
    ) -> None:
        self._sf = session_factory
        self._cfg = settings
        self._sink = sink
        self._clock: Clock = clock or SystemClock()

    async def tick(self) -> tuple[SweeperAlert, ...]:
        now = self._clock.now()
        alerts: list[SweeperAlert] = []
        async with self._sf() as session:
            stmt = (
                select(Contract)
                .where(
                    Contract.deprecated_at.is_not(None),
                    Contract.sunset_at.is_not(None),
                )
            )
            rows = list((await session.execute(stmt)).scalars())
            for row in rows:
                if row.sunset_at is None:
                    continue
                remaining = (_as_utc(row.sunset_at) - now).total_seconds()
                days_remaining = int(remaining // 86_400)
                if days_remaining in self._cfg.alert_days_before_sunset:
                    alerts.append(SweeperAlert(
                        contract_hash=bytes(row.contract_hash),
                        sunset_at=row.sunset_at,
                        days_remaining=days_remaining,
                    ))
        if alerts:
            await self._sink.notify(tuple(alerts))
        return tuple(alerts)


__all__ = [
    "AlertSink",
    "DeprecationSweeper",
    "InMemoryAlertSink",
    "SweeperAlert",
]
