"""ReviewService — list + inspect review requests."""

from __future__ import annotations

from collections.abc import Sequence
from dataclasses import dataclass
from datetime import datetime
from typing import Any

from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from efe_gate.db.models import ReviewRequest


@dataclass(frozen=True, slots=True)
class ReviewSignature:
    reviewer_id: str
    approved: bool
    reason: str
    at: datetime


@dataclass(frozen=True, slots=True)
class ReviewView:
    id: str
    eval_run_id: str
    tenant_id: str
    task_id: str
    reason: str
    requires_dual_signoff: bool
    signatures: tuple[dict[str, Any], ...]
    status: str
    deadline: datetime
    created_at: datetime
    resolved_at: datetime | None

    @classmethod
    def from_row(cls, row: ReviewRequest) -> ReviewView:
        return cls(
            id=row.id, eval_run_id=row.eval_run_id,
            tenant_id=row.tenant_id, task_id=row.task_id,
            reason=row.reason,
            requires_dual_signoff=row.requires_dual_signoff,
            signatures=tuple(row.signatures or []),
            status=row.status,
            deadline=row.deadline,
            created_at=row.created_at,
            resolved_at=row.resolved_at,
        )

    def as_dict(self) -> dict[str, Any]:
        return {
            "id": self.id,
            "eval_run_id": self.eval_run_id,
            "tenant_id": self.tenant_id,
            "task_id": self.task_id,
            "reason": self.reason,
            "requires_dual_signoff": self.requires_dual_signoff,
            "signatures": [dict(s) for s in self.signatures],
            "status": self.status,
            "deadline": self.deadline.isoformat(),
            "created_at": self.created_at.isoformat(),
            "resolved_at": (
                self.resolved_at.isoformat()
                if self.resolved_at else None
            ),
        }


class ReviewService:
    def __init__(self, session: AsyncSession) -> None:
        self._session = session

    async def get_by_eval(self, eval_run_id: str) -> ReviewView | None:
        stmt = (
            select(ReviewRequest)
            .where(ReviewRequest.eval_run_id == eval_run_id)
            .limit(1)
        )
        row = (await self._session.execute(stmt)).scalar_one_or_none()
        return None if row is None else ReviewView.from_row(row)

    async def list_pending(
        self, tenant_id: str,
    ) -> Sequence[ReviewView]:
        stmt = (
            select(ReviewRequest)
            .where(ReviewRequest.tenant_id == tenant_id)
            .where(ReviewRequest.status == "pending")
            .order_by(ReviewRequest.created_at.asc())
        )
        rows = list((await self._session.execute(stmt)).scalars())
        return tuple(ReviewView.from_row(r) for r in rows)


__all__ = ["ReviewService", "ReviewSignature", "ReviewView"]
