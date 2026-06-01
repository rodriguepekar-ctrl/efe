"""Tests for class-balance invariant."""

from __future__ import annotations

import pytest
from sqlalchemy.ext.asyncio import AsyncSession

from efe_ft.db.models import TrainingSample
from efe_ft.hashing import sha256_text
from efe_ft.ingest.balance import balance_skew_check


async def _seed(session: AsyncSession, counts: dict[str, int]) -> None:
    for label, n in counts.items():
        for i in range(n):
            session.add(TrainingSample(
                id=f"id-{label}-{i}",
                tenant_id="acme", task_id="routing",
                text=f"{label} text {i}",
                text_sha256=sha256_text(f"{label}-text-{i}"),
                language="fr",
                label=label,
                source="batch",
                status="labelled",
                pii_flags=[],
            ))
    await session.commit()


async def test_empty_is_healthy(session: AsyncSession) -> None:
    report = await balance_skew_check(session, "acme", "routing")
    assert report.n_samples == 0
    assert report.is_healthy(threshold=50.0)


async def test_balanced_is_healthy(session: AsyncSession) -> None:
    await _seed(session, {"billing": 30, "refund": 40, "auth": 35})
    report = await balance_skew_check(session, "acme", "routing")
    assert report.n_samples == 105
    assert report.skew < 2.0
    assert report.is_healthy(threshold=50.0)


async def test_skewed_flagged(session: AsyncSession) -> None:
    await _seed(session, {"billing": 1, "refund": 200})
    report = await balance_skew_check(session, "acme", "routing")
    assert report.skew == 200.0
    assert not report.is_healthy(threshold=50.0)


async def test_multiple_tenants_isolated(session: AsyncSession) -> None:
    # seed one tenant
    await _seed(session, {"billing": 5, "refund": 5})
    report = await balance_skew_check(session, "other", "routing")
    assert report.n_samples == 0
