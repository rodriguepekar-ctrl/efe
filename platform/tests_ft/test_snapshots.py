"""Tests for snapshot materialisation."""

from __future__ import annotations

import pytest
from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from efe_ft.audit import verify_chain
from efe_ft.db.models import SnapshotMember, TrainingSample
from efe_ft.errors import SnapshotError
from efe_ft.hashing import sha256_text
from efe_ft.snapshots import MaterialisationSpec, Partitioning, materialise


async def _seed(session: AsyncSession, n: int, tenant: str = "acme") -> None:
    for i in range(n):
        session.add(TrainingSample(
            id=f"s{i:04d}",
            tenant_id=tenant, task_id="routing",
            text=f"text {i}",
            text_sha256=sha256_text(f"text {i}"),
            label="billing" if i % 2 == 0 else "refund",
            status="labelled", source="batch",
        ))
    await session.commit()


async def test_materialise_happy_path(session: AsyncSession) -> None:
    await _seed(session, 12)
    snap = await materialise(session, MaterialisationSpec(
        tenant_id="acme", task_id="routing",
        partitioning=Partitioning(train=8, val=2, holdout=2, seed=42),
    ))
    await session.commit()
    assert snap.n_samples == 12
    assert dict(snap.label_balance) == {"billing": 6, "refund": 6}
    members = list((await session.execute(select(SnapshotMember))).scalars())
    assert len(members) == 12
    parts = {m.partition for m in members}
    assert parts == {"train", "val", "holdout"}


async def test_materialise_is_idempotent(session: AsyncSession) -> None:
    await _seed(session, 15)
    spec = MaterialisationSpec(tenant_id="acme", task_id="routing")
    snap1 = await materialise(session, spec)
    await session.commit()
    snap2 = await materialise(session, spec)
    await session.commit()
    assert snap1.id == snap2.id
    assert snap1.content_hash == snap2.content_hash


async def test_materialise_too_few_samples(session: AsyncSession) -> None:
    await _seed(session, 3)
    with pytest.raises(SnapshotError, match="not enough samples"):
        await materialise(session, MaterialisationSpec(
            tenant_id="acme", task_id="routing", min_samples=10,
        ))


async def test_partitioning_total_exceeds_samples(session: AsyncSession) -> None:
    await _seed(session, 5)
    with pytest.raises(SnapshotError, match="partitioning"):
        await materialise(session, MaterialisationSpec(
            tenant_id="acme", task_id="routing",
            partitioning=Partitioning(train=10, val=5, holdout=5),
            min_samples=1,
        ))


async def test_audit_records_materialisation(session: AsyncSession) -> None:
    await _seed(session, 10)
    await materialise(session, MaterialisationSpec(
        tenant_id="acme", task_id="routing", min_samples=5,
    ))
    await session.commit()
    ok, _ = await verify_chain(session, "acme")
    assert ok
