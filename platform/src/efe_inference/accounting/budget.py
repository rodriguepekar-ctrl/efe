"""BudgetEnforcer — tenant-level $/day cap with Redis-like store."""

from __future__ import annotations

import asyncio
from dataclasses import dataclass, field
from datetime import date, timezone
from decimal import Decimal
from typing import Protocol

from efe_inference.clock import Clock, SystemClock
from efe_inference.errors import BudgetExceeded


class BudgetStore(Protocol):
    async def get_spend(
        self, tenant_id: str, day: date,
    ) -> Decimal:  # pragma: no cover
        ...

    async def add_spend(
        self, tenant_id: str, day: date, amount: Decimal,
    ) -> Decimal:  # pragma: no cover
        ...

    async def reserve_if_under(
        self,
        tenant_id: str,
        day: date,
        *,
        max_allowed: Decimal,
    ) -> bool:  # pragma: no cover
        """Atomic: refuse if current spend ``>= max_allowed``, else no-op."""


@dataclass
class InMemoryBudgetStore:
    """Atomic in-memory store.

    Fix C2/M3 — every public method is serialised through an
    ``asyncio.Lock`` so two concurrent ``preflight → record`` loops
    cannot both pass the guard before the second one is charged.
    """

    _state: dict[tuple[str, date], Decimal] = field(default_factory=dict)
    _lock: asyncio.Lock = field(default_factory=asyncio.Lock)

    async def get_spend(self, tenant_id: str, day: date) -> Decimal:
        async with self._lock:
            return self._state.get((tenant_id, day), Decimal(0))

    async def add_spend(
        self, tenant_id: str, day: date, amount: Decimal,
    ) -> Decimal:
        async with self._lock:
            key = (tenant_id, day)
            new_val = self._state.get(key, Decimal(0)) + amount
            self._state[key] = new_val
            return new_val

    async def reserve_if_under(
        self,
        tenant_id: str,
        day: date,
        *,
        max_allowed: Decimal,
    ) -> bool:
        async with self._lock:
            return self._state.get((tenant_id, day), Decimal(0)) < max_allowed


@dataclass
class BudgetEnforcer:
    store: BudgetStore
    default_daily_budget_usd: Decimal = Decimal("100")
    tenant_budgets: dict[str, Decimal] = field(default_factory=dict)
    clock: Clock = field(default_factory=SystemClock)
    enforced: bool = True

    async def preflight(self, tenant_id: str) -> None:
        if not self.enforced:
            return
        day = self._today()
        budget = self.tenant_budgets.get(
            tenant_id, self.default_daily_budget_usd,
        )
        # C2 — ``reserve_if_under`` is atomic on the store and collapses
        # the get + guard into a single critical section. If the store
        # refuses, we raise immediately.
        under = await self.store.reserve_if_under(
            tenant_id, day, max_allowed=budget,
        )
        if not under:
            spend = await self.store.get_spend(tenant_id, day)
            raise BudgetExceeded(
                f"tenant {tenant_id!r} has exceeded daily budget: "
                f"{spend} >= {budget}",
                detail={"tenant_id": tenant_id,
                         "spend_usd": str(spend),
                         "budget_usd": str(budget)},
            )

    async def record(
        self, tenant_id: str, amount_usd: Decimal,
    ) -> Decimal:
        day = self._today()
        return await self.store.add_spend(tenant_id, day, amount_usd)

    def _today(self) -> date:
        now = self.clock.now()
        if now.tzinfo is None:
            now = now.replace(tzinfo=timezone.utc)
        return now.astimezone(timezone.utc).date()


__all__ = [
    "BudgetEnforcer",
    "BudgetStore",
    "InMemoryBudgetStore",
]
