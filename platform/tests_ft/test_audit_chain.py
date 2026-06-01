"""Tests for the audit chain — tamper detection."""

from __future__ import annotations

import pytest
from sqlalchemy import update
from sqlalchemy.ext.asyncio import AsyncSession

from efe_ft.audit import AuditChain, verify_chain
from efe_ft.db.models import AuditEvent


async def test_empty_chain_verifies(session: AsyncSession) -> None:
    ok, first = await verify_chain(session, "acme")
    assert ok
    assert first is None


async def test_single_entry_verifies(session: AsyncSession) -> None:
    chain = AuditChain(session, "acme")
    await chain.record(
        kind="ingest_accepted", actor_id="alice",
        payload={"n": 1},
    )
    await session.commit()
    ok, first = await verify_chain(session, "acme")
    assert ok and first is None


async def test_multi_entry_chain_links(session: AsyncSession) -> None:
    chain = AuditChain(session, "acme")
    e1 = await chain.record(kind="ingest_accepted", actor_id="alice")
    e2 = await chain.record(kind="label_written", actor_id="alice",
                            subject_id=e1.id)
    e3 = await chain.record(kind="snapshot_materialised", actor_id="alice")
    await session.commit()
    assert e2.prev_hash == e1.this_hash
    assert e3.prev_hash == e2.this_hash
    ok, first = await verify_chain(session, "acme")
    assert ok and first is None


async def test_tamper_detected(session: AsyncSession) -> None:
    chain = AuditChain(session, "acme")
    e1 = await chain.record(kind="ingest_accepted", actor_id="alice",
                            payload={"x": 1})
    e2 = await chain.record(kind="label_written", actor_id="alice",
                            subject_id=e1.id)
    await session.commit()
    # Tamper with e1's payload directly.
    await session.execute(
        update(AuditEvent)
        .where(AuditEvent.id == e1.id)
        .values(payload={"x": 999})
    )
    await session.commit()
    ok, first_broken = await verify_chain(session, "acme")
    assert not ok
    assert first_broken == e1.id


async def test_unknown_kind_rejected(session: AsyncSession) -> None:
    chain = AuditChain(session, "acme")
    with pytest.raises(ValueError, match="unknown audit kind"):
        await chain.record(kind="bogus_kind", actor_id="alice")


async def test_cross_tenant_chains_independent(session: AsyncSession) -> None:
    a = AuditChain(session, "acme")
    b = AuditChain(session, "other")
    await a.record(kind="ingest_accepted", actor_id="alice")
    await b.record(kind="ingest_accepted", actor_id="bob")
    await session.commit()
    ok_a, _ = await verify_chain(session, "acme")
    ok_b, _ = await verify_chain(session, "other")
    assert ok_a and ok_b
