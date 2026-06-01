"""GateService — CRUD on ``promotion_gates`` rules."""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime
from typing import Any

from sqlalchemy.ext.asyncio import AsyncSession

from efe_gate.clock import Clock, SystemClock
from efe_gate.db.models import PromotionGate
from efe_gate.decision.rule import PromotionRule
from efe_gate.errors import NotFound, ValidationError


@dataclass(frozen=True, slots=True)
class GateInfo:
    tenant_id: str
    task_id: str
    rule: PromotionRule
    updated_by: str
    updated_at: datetime

    def as_dict(self) -> dict[str, Any]:
        return {
            "tenant_id": self.tenant_id,
            "task_id": self.task_id,
            "rule": self.rule.as_dict(),
            "updated_by": self.updated_by,
            "updated_at": self.updated_at.isoformat(),
        }


class GateService:
    """Read / upsert rule configuration."""

    def __init__(
        self,
        session: AsyncSession,
        *,
        clock: Clock | None = None,
    ) -> None:
        self._session = session
        self._clock: Clock = clock or SystemClock()

    async def get(self, tenant_id: str, task_id: str) -> GateInfo:
        row = await self._session.get(PromotionGate, (tenant_id, task_id))
        if row is None:
            raise NotFound(
                f"no promotion gate for {tenant_id}/{task_id}",
            )
        return GateInfo(
            tenant_id=row.tenant_id, task_id=row.task_id,
            rule=PromotionRule.from_dict(dict(row.rule or {})),
            updated_by=row.updated_by, updated_at=row.updated_at,
        )

    async def upsert(
        self,
        *,
        tenant_id: str, task_id: str,
        rule: PromotionRule,
        updated_by: str,
    ) -> GateInfo:
        if not updated_by:
            raise ValidationError("updated_by required")
        row = await self._session.get(PromotionGate, (tenant_id, task_id))
        now = self._clock.now()
        if row is None:
            row = PromotionGate(
                tenant_id=tenant_id, task_id=task_id,
                rule=rule.as_dict(),
                updated_by=updated_by, updated_at=now,
            )
            self._session.add(row)
        else:
            row.rule = rule.as_dict()
            row.updated_by = updated_by
            row.updated_at = now
        await self._session.flush()
        return GateInfo(
            tenant_id=row.tenant_id, task_id=row.task_id,
            rule=rule,
            updated_by=row.updated_by, updated_at=row.updated_at,
        )


__all__ = ["GateInfo", "GateService"]
