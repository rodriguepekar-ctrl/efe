"""Tests for the golden-set manager."""

from __future__ import annotations

import hashlib
from typing import Any

import pytest
from sqlalchemy.ext.asyncio import AsyncSession

from efe_gate.clock import FakeClock
from efe_gate.errors import (
    GoldenSetDeprecated,
    GoldenSetLocked,
    NotFound,
    ValidationError,
)
from efe_gate.golden import GoldenSetManager


def _hash(text: str) -> str:
    return hashlib.sha256(text.encode()).hexdigest()


async def test_create_roundtrip(
    session: AsyncSession, fake_clock: FakeClock,
) -> None:
    mgr = GoldenSetManager(session, clock=fake_clock)
    info = await mgr.create(
        tenant_id="acme", task_id="routing",
        snapshot_uri="file:///gold",
        content_hash=_hash("abc"),
        n_samples=1000,
        created_by="curator",
    )
    assert info.n_samples == 1000
    assert info.deprecated is False
    got = await mgr.get(info.id)
    assert got.content_hash == _hash("abc")


async def test_create_rejects_duplicate_content(
    session: AsyncSession, fake_clock: FakeClock,
) -> None:
    mgr = GoldenSetManager(session, clock=fake_clock)
    await mgr.create(
        tenant_id="acme", task_id="routing",
        snapshot_uri="file:///gold",
        content_hash=_hash("dup"),
        n_samples=10, created_by="curator",
    )
    await session.commit()
    from efe_gate.errors import GoldenSetError
    with pytest.raises(GoldenSetError):
        await mgr.create(
            tenant_id="acme", task_id="routing",
            snapshot_uri="file:///gold2",
            content_hash=_hash("dup"),
            n_samples=10, created_by="curator2",
        )


async def test_latest_for(
    session: AsyncSession, fake_clock: FakeClock,
) -> None:
    mgr = GoldenSetManager(session, clock=fake_clock)
    a = await mgr.create(
        tenant_id="acme", task_id="routing",
        snapshot_uri="file:///g1", content_hash=_hash("one"),
        n_samples=10, created_by="c",
    )
    fake_clock.advance(60)
    b = await mgr.create(
        tenant_id="acme", task_id="routing",
        snapshot_uri="file:///g2", content_hash=_hash("two"),
        n_samples=10, created_by="c",
    )
    await session.commit()
    latest = await mgr.latest_for("acme", "routing")
    assert latest is not None
    assert latest.id == b.id
    _ = a


async def test_deprecate(
    session: AsyncSession, fake_clock: FakeClock,
) -> None:
    mgr = GoldenSetManager(session, clock=fake_clock)
    gs = await mgr.create(
        tenant_id="acme", task_id="routing",
        snapshot_uri="file:///g", content_hash=_hash("x"),
        n_samples=10, created_by="c",
    )
    updated = await mgr.deprecate(gs.id, actor="ops")
    assert updated.deprecated is True
    # Deprecating again raises.
    with pytest.raises(GoldenSetDeprecated):
        await mgr.deprecate(gs.id, actor="ops")


async def test_mutation_blocked(
    session: AsyncSession, fake_clock: FakeClock,
) -> None:
    mgr = GoldenSetManager(session, clock=fake_clock)
    gs = await mgr.create(
        tenant_id="acme", task_id="routing",
        snapshot_uri="file:///g", content_hash=_hash("m"),
        n_samples=10, created_by="c",
    )
    with pytest.raises(GoldenSetLocked):
        await mgr.assert_mutation_blocked(gs.id, field="snapshot_uri")


async def test_get_missing(
    session: AsyncSession, fake_clock: FakeClock,
) -> None:
    mgr = GoldenSetManager(session, clock=fake_clock)
    with pytest.raises(NotFound):
        await mgr.get("00000000-0000-0000-0000-000000000000")


async def test_create_validation_errors(
    session: AsyncSession, fake_clock: FakeClock,
) -> None:
    mgr = GoldenSetManager(session, clock=fake_clock)
    with pytest.raises(ValidationError):
        await mgr.create(
            tenant_id="", task_id="routing",
            snapshot_uri="file:///g", content_hash=_hash("v"),
            n_samples=10, created_by="c",
        )
    with pytest.raises(ValidationError):
        await mgr.create(
            tenant_id="acme", task_id="routing",
            snapshot_uri="", content_hash=_hash("v2"),
            n_samples=10, created_by="c",
        )
    with pytest.raises(ValidationError):
        await mgr.create(
            tenant_id="acme", task_id="routing",
            snapshot_uri="file:///g", content_hash=_hash("v3"),
            n_samples=0, created_by="c",
        )


async def test_hash_accepts_bytes(
    session: AsyncSession, fake_clock: FakeClock,
) -> None:
    mgr = GoldenSetManager(session, clock=fake_clock)
    raw = bytes.fromhex(_hash("bytes-in"))
    info = await mgr.create(
        tenant_id="acme", task_id="routing",
        snapshot_uri="file:///bx", content_hash=raw,
        n_samples=10, created_by="c",
    )
    assert info.content_hash == raw.hex()


async def test_hash_validation_wrong_size(
    session: AsyncSession, fake_clock: FakeClock,
) -> None:
    mgr = GoldenSetManager(session, clock=fake_clock)
    with pytest.raises(ValidationError):
        await mgr.create(
            tenant_id="acme", task_id="routing",
            snapshot_uri="file:///x", content_hash="abc",
            n_samples=1, created_by="c",
        )


async def test_list_excludes_deprecated(
    session: AsyncSession, fake_clock: FakeClock,
) -> None:
    mgr = GoldenSetManager(session, clock=fake_clock)
    a = await mgr.create(
        tenant_id="acme", task_id="routing",
        snapshot_uri="file:///a",
        content_hash=_hash("a"),
        n_samples=10, created_by="c",
    )
    fake_clock.advance(1)
    b = await mgr.create(
        tenant_id="acme", task_id="routing",
        snapshot_uri="file:///b",
        content_hash=_hash("b"),
        n_samples=10, created_by="c",
    )
    await mgr.deprecate(b.id, actor="ops")
    await session.commit()
    lst = await mgr.list_for("acme", "routing")
    assert {x.id for x in lst} == {a.id}
    lst_all = await mgr.list_for(
        "acme", "routing", include_deprecated=True,
    )
    assert {x.id for x in lst_all} == {a.id, b.id}


def _silence_unused(value: Any) -> Any:
    return value


_ = _silence_unused
