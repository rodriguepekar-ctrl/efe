"""Tests for :class:`EscalationService`."""

from __future__ import annotations

import pytest
from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from efe_ft.audit import verify_chain
from efe_ft.db.models import LabelHistory, TrainingSample
from efe_ft.errors import LabelingError
from efe_ft.hashing import sha256_text
from efe_ft.labeling.escalation import EscalationService


async def _seed_sample(
    session: AsyncSession, sid: str, tenant: str = "acme", task: str = "routing",
) -> None:
    session.add(TrainingSample(
        id=sid, tenant_id=tenant, task_id=task,
        text=f"payload {sid}", text_sha256=sha256_text(f"payload {sid}"),
        status="labelled", source="batch",
    ))


async def _add_label(
    session: AsyncSession, sid: str, annotator: str, value: str,
    *, is_expert: bool = False, round_num: int = 1,
) -> None:
    session.add(LabelHistory(
        sample_id=sid, annotator_id=annotator,
        label_value=value, round=round_num, is_expert=is_expert,
    ))


async def test_empty_queue_when_no_disagreements(
    session: AsyncSession,
) -> None:
    await _seed_sample(session, "s1")
    await _add_label(session, "s1", "alice", "billing")
    await session.commit()
    esc = EscalationService(session)
    assert await esc.queue(tenant_id="acme", task_id="routing") == []


async def test_single_disagreement_surfaces(session: AsyncSession) -> None:
    await _seed_sample(session, "s1")
    await _add_label(session, "s1", "alice", "billing")
    await _add_label(session, "s1", "bob", "refund")
    await session.commit()
    esc = EscalationService(session)
    queue = await esc.queue(tenant_id="acme", task_id="routing")
    assert len(queue) == 1
    item = queue[0]
    assert item.sample_id == "s1"
    proposals = dict(item.labels_proposed)
    assert proposals == {"alice": "billing", "bob": "refund"}


async def test_agreement_not_in_queue(session: AsyncSession) -> None:
    await _seed_sample(session, "s1")
    await _add_label(session, "s1", "alice", "billing")
    await _add_label(session, "s1", "bob", "billing")
    await session.commit()
    esc = EscalationService(session)
    assert await esc.queue(tenant_id="acme", task_id="routing") == []


async def test_already_resolved_not_in_queue(session: AsyncSession) -> None:
    await _seed_sample(session, "s1")
    await _add_label(session, "s1", "alice", "billing")
    await _add_label(session, "s1", "bob", "refund")
    await _add_label(session, "s1", "carol", "refund",
                     is_expert=True, round_num=3)
    await session.commit()
    esc = EscalationService(session)
    assert await esc.queue(tenant_id="acme", task_id="routing") == []


async def test_cross_tenant_isolated(session: AsyncSession) -> None:
    await _seed_sample(session, "acme1", tenant="acme")
    await _add_label(session, "acme1", "alice", "billing")
    await _add_label(session, "acme1", "bob", "refund")
    await _seed_sample(session, "other1", tenant="other")
    await _add_label(session, "other1", "dave", "a")
    await _add_label(session, "other1", "eve", "b")
    await session.commit()
    esc = EscalationService(session)
    acme_queue = await esc.queue(tenant_id="acme", task_id="routing")
    other_queue = await esc.queue(tenant_id="other", task_id="routing")
    assert len(acme_queue) == 1
    assert acme_queue[0].sample_id == "acme1"
    assert len(other_queue) == 1
    assert other_queue[0].sample_id == "other1"


async def test_resolve_writes_expert_label(session: AsyncSession) -> None:
    await _seed_sample(session, "s1")
    await _add_label(session, "s1", "alice", "billing")
    await _add_label(session, "s1", "bob", "refund")
    await session.commit()

    esc = EscalationService(session)
    history_id = await esc.resolve(
        tenant_id="acme", sample_id="s1",
        expert_id="carol", label="refund",
    )
    await session.commit()
    assert history_id

    rows = list((await session.execute(select(LabelHistory)
                                       .where(LabelHistory.is_expert))).scalars())
    assert len(rows) == 1
    assert rows[0].label_value == "refund"
    assert rows[0].round == 3

    sample = await session.get(TrainingSample, "s1")
    assert sample is not None
    assert sample.label == "refund"

    # Audit chain remains intact.
    ok, _ = await verify_chain(session, "acme")
    assert ok


async def test_resolve_cross_tenant_blocked(session: AsyncSession) -> None:
    await _seed_sample(session, "s1", tenant="acme")
    await session.commit()
    esc = EscalationService(session)
    with pytest.raises(LabelingError, match="cross-tenant"):
        await esc.resolve(
            tenant_id="other", sample_id="s1",
            expert_id="carol", label="billing",
        )
