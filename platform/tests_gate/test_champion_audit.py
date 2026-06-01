"""Tests for the champion registry + audit chain."""

from __future__ import annotations

import pytest
from sqlalchemy.ext.asyncio import AsyncSession

from efe_gate.champion.audit import AuditChain, AuditKind, verify_chain
from efe_gate.champion.registry import ChampionRegistry
from efe_gate.clock import FakeClock as _FakeClock

# Re-export so type hints match the fixture name used below.
FakeClock = _FakeClock
from efe_gate.config import SignatureSettings
from efe_gate.db.models import EvalRun, GoldenSet
from efe_gate.errors import (
    ChainIntegrityBroken,
    NoChampion,
    NoPreviousChampion,
    ValidationError,
)


async def _create_eval(
    session: AsyncSession, golden_set: GoldenSet,
    run_id: str = "22222222-2222-2222-2222-222222222222",
) -> EvalRun:
    run = EvalRun(
        id="11111111-1111-1111-1111-111111111111",
        tenant_id="acme", task_id="routing",
        candidate_run_id=run_id,
        candidate_uri="file:///a",
        golden_set_id=golden_set.id,
        state="decided", state_version=1,
        submitted_by="alice",
    )
    session.add(run)
    await session.commit()
    return run


# ------------------ audit chain ------------------


async def test_audit_chain_grows(
    session: AsyncSession, audit_chain: AuditChain,
    signing_settings: SignatureSettings,
    fake_clock: FakeClock,
) -> None:
    e1 = await audit_chain.record(
        kind=AuditKind.DECISION_PROMOTE,
        actor="a", subject_id="r1", payload={"x": 1},
    )
    # Advance the clock so verify_chain iterates records in insertion
    # order even when UUIDs would otherwise sort differently.
    fake_clock.advance(1)
    e2 = await audit_chain.record(
        kind=AuditKind.DECISION_REJECT,
        actor="b", subject_id="r2", payload={"x": 2},
    )
    await session.commit()
    assert e2.prev_hash == e1.this_hash
    ok, bad = await verify_chain(
        session, "acme", signing=signing_settings,
    )
    assert ok is True
    assert bad is None


async def test_audit_unknown_kind(audit_chain: AuditChain) -> None:
    with pytest.raises(ValidationError):
        await audit_chain.record(
            kind="galaxy_brain",
            actor="x", subject_id=None, payload={},
        )


async def test_audit_empty_actor(audit_chain: AuditChain) -> None:
    with pytest.raises(ValidationError):
        await audit_chain.record(
            kind=AuditKind.DECISION_PROMOTE,
            actor="", subject_id="r", payload={},
        )


async def test_audit_chain_verifies_after_tamper(
    session: AsyncSession, audit_chain: AuditChain,
    signing_settings: SignatureSettings,
) -> None:
    e1 = await audit_chain.record(
        kind=AuditKind.DECISION_PROMOTE,
        actor="a", subject_id="r1", payload={"x": 1},
    )
    await session.commit()
    # Tamper with the signature.
    from efe_gate.db.models import AuditEvent
    row = await session.get(AuditEvent, e1.id)
    assert row is not None
    row.signature = bytes(32)
    await session.commit()
    with pytest.raises(ChainIntegrityBroken):
        await verify_chain(session, "acme", signing=signing_settings)


# ------------------ champion registry ------------------


async def test_promote_creates_champion(
    session: AsyncSession, registry: ChampionRegistry,
    golden_set: GoldenSet,
) -> None:
    eval_run = await _create_eval(session, golden_set)
    champ = await registry.promote(
        tenant_id="acme", task_id="routing",
        run_id=eval_run.candidate_run_id,
        artifact_uri="s3://art/v1",
        eval_run_id=eval_run.id,
        metrics_snapshot={"accuracy": {"role": "primary", "value": 0.9}},
        promoted_by="engine",
    )
    await session.commit()
    assert champ.version == 1
    info = await registry.get("acme", "routing")
    assert info.artifact_uri == "s3://art/v1"


async def test_promote_bumps_version(
    session: AsyncSession, registry: ChampionRegistry,
    golden_set: GoldenSet,
) -> None:
    e1 = await _create_eval(
        session, golden_set,
        run_id="33333333-3333-3333-3333-333333333333",
    )
    await registry.promote(
        tenant_id="acme", task_id="routing",
        run_id=e1.candidate_run_id,
        artifact_uri="s3://a1",
        eval_run_id=e1.id,
        metrics_snapshot={"accuracy": {"role": "primary", "value": 0.9}},
        promoted_by="e",
    )
    # A second eval row with a new candidate run id.
    e2 = EvalRun(
        id="44444444-4444-4444-4444-444444444444",
        tenant_id="acme", task_id="routing",
        candidate_run_id="55555555-5555-5555-5555-555555555555",
        candidate_uri="file:///b",
        golden_set_id=golden_set.id,
        state="decided", state_version=1, submitted_by="a",
    )
    session.add(e2)
    await session.commit()
    ch = await registry.promote(
        tenant_id="acme", task_id="routing",
        run_id=e2.candidate_run_id,
        artifact_uri="s3://a2",
        eval_run_id=e2.id,
        metrics_snapshot={"accuracy": {"role": "primary", "value": 0.91}},
        promoted_by="e",
    )
    assert ch.version == 2


async def test_get_raises_when_no_champion(
    registry: ChampionRegistry,
) -> None:
    with pytest.raises(NoChampion):
        await registry.get("nobody", "none")


async def test_rollback_requires_previous(
    session: AsyncSession, registry: ChampionRegistry,
    golden_set: GoldenSet,
) -> None:
    e = await _create_eval(session, golden_set)
    await registry.promote(
        tenant_id="acme", task_id="routing",
        run_id=e.candidate_run_id, artifact_uri="s3://v1",
        eval_run_id=e.id,
        metrics_snapshot={"accuracy": {"role": "primary", "value": 0.9}},
        promoted_by="e",
    )
    with pytest.raises(NoPreviousChampion):
        await registry.rollback(
            tenant_id="acme", task_id="routing", actor="daemon",
        )


async def test_rollback_restores_previous(
    session: AsyncSession, registry: ChampionRegistry,
    golden_set: GoldenSet, fake_clock: FakeClock,
) -> None:
    e1 = await _create_eval(
        session, golden_set,
        run_id="aaaaaaaa-1111-1111-1111-111111111111",
    )
    await registry.promote(
        tenant_id="acme", task_id="routing",
        run_id=e1.candidate_run_id, artifact_uri="s3://v1",
        eval_run_id=e1.id,
        metrics_snapshot={"accuracy": {"role": "primary", "value": 0.9}},
        promoted_by="e",
    )
    fake_clock.advance(60)  # strictly later timestamp
    # Second promote.
    e2 = EvalRun(
        id="bbbbbbbb-2222-2222-2222-222222222222",
        tenant_id="acme", task_id="routing",
        candidate_run_id="cccccccc-3333-3333-3333-333333333333",
        candidate_uri="file:///b", golden_set_id=golden_set.id,
        state="decided", state_version=1, submitted_by="a",
    )
    session.add(e2)
    await session.commit()
    await registry.promote(
        tenant_id="acme", task_id="routing",
        run_id=e2.candidate_run_id, artifact_uri="s3://v2",
        eval_run_id=e2.id,
        metrics_snapshot={"accuracy": {"role": "primary", "value": 0.91}},
        promoted_by="e",
    )
    # Rollback.
    restored = await registry.rollback(
        tenant_id="acme", task_id="routing", actor="daemon",
    )
    await session.commit()
    assert restored.artifact_uri == "s3://v1"
    assert restored.run_id == "aaaaaaaa-1111-1111-1111-111111111111"
