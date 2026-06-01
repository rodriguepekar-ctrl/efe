"""Tests for the LabelerService end-to-end flow."""

from __future__ import annotations

import pytest
from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from efe_ft.db.models import LabelHistory, TrainingSample
from efe_ft.errors import SampleNotAssigned, UnknownLabel
from efe_ft.hashing import sha256_text
from efe_ft.labeling.labeler_service import LabelerService
from efe_ft.labeling.session import InMemorySessionStore

CLASS_SET = frozenset(["billing", "refund", "auth"])


async def _seed(session: AsyncSession, ids: list[str]) -> None:
    for sid in ids:
        session.add(TrainingSample(
            id=sid, tenant_id="acme", task_id="routing",
            text=f"text for {sid}",
            text_sha256=sha256_text(sid),
            status="pending", source="batch",
        ))
    await session.commit()


async def test_session_lifecycle(session: AsyncSession) -> None:
    store = InMemorySessionStore()
    svc = LabelerService(session, store=store, class_set=CLASS_SET)
    s = await svc.start_session(
        annotator_id="alice", tenant_id="acme", task_id="routing",
    )
    assert s.session_id.startswith("sess_")
    await svc.end_session(s.session_id)


async def test_submit_label_writes_history(session: AsyncSession) -> None:
    await _seed(session, ["s1", "s2", "s3"])
    store = InMemorySessionStore()
    svc = LabelerService(session, store=store, class_set=CLASS_SET)
    sess = await svc.start_session(
        annotator_id="alice", tenant_id="acme", task_id="routing",
    )
    # Simulate AL batch
    state = await store.get(sess.session_id)
    await store.set(state.with_batch(["s1", "s2", "s3"]), ttl_s=300)

    result = await svc.submit_label(
        session_id=sess.session_id, sample_id="s1",
        label="billing", time_ms=1500,
    )
    await session.commit()
    assert result.accepted
    assert result.next_sample_id == "s2"
    assert result.labels_submitted == 1

    rows = list((await session.execute(select(LabelHistory))).scalars())
    assert len(rows) == 1
    assert rows[0].label_value == "billing"
    assert rows[0].annotator_id == "alice"

    sample = await session.get(TrainingSample, "s1")
    assert sample is not None
    assert sample.label == "billing"
    assert sample.status == "labelled"


async def test_unknown_label_raises(session: AsyncSession) -> None:
    await _seed(session, ["s1"])
    store = InMemorySessionStore()
    svc = LabelerService(session, store=store, class_set=CLASS_SET)
    sess = await svc.start_session(
        annotator_id="alice", tenant_id="acme", task_id="routing",
    )
    state = await store.get(sess.session_id)
    await store.set(state.with_batch(["s1"]), ttl_s=300)
    with pytest.raises(UnknownLabel):
        await svc.submit_label(
            session_id=sess.session_id, sample_id="s1", label="mystery",
        )


async def test_sample_not_in_batch_rejected(session: AsyncSession) -> None:
    await _seed(session, ["s1", "s2"])
    store = InMemorySessionStore()
    svc = LabelerService(session, store=store, class_set=CLASS_SET)
    sess = await svc.start_session(
        annotator_id="alice", tenant_id="acme", task_id="routing",
    )
    state = await store.get(sess.session_id)
    await store.set(state.with_batch(["s1"]), ttl_s=300)
    with pytest.raises(SampleNotAssigned):
        await svc.submit_label(
            session_id=sess.session_id, sample_id="s2", label="billing",
        )


async def test_flag_skip_advances_cursor(session: AsyncSession) -> None:
    await _seed(session, ["s1", "s2"])
    store = InMemorySessionStore()
    svc = LabelerService(session, store=store, class_set=CLASS_SET)
    sess = await svc.start_session(
        annotator_id="alice", tenant_id="acme", task_id="routing",
    )
    state = await store.get(sess.session_id)
    await store.set(state.with_batch(["s1", "s2"]), ttl_s=300)
    result = await svc.flag(
        session_id=sess.session_id, sample_id="s1", reason="skip",
    )
    await session.commit()
    assert result.next_sample_id == "s2"


async def test_flag_harmful_archives_sample(session: AsyncSession) -> None:
    await _seed(session, ["s1"])
    store = InMemorySessionStore()
    svc = LabelerService(session, store=store, class_set=CLASS_SET)
    sess = await svc.start_session(
        annotator_id="alice", tenant_id="acme", task_id="routing",
    )
    state = await store.get(sess.session_id)
    await store.set(state.with_batch(["s1"]), ttl_s=300)
    await svc.flag(
        session_id=sess.session_id, sample_id="s1", reason="harmful",
    )
    await session.commit()
    row = await session.get(TrainingSample, "s1")
    assert row is not None and row.status == "archived"


async def test_dual_label_disagreement_triggers_escalation(
    session: AsyncSession,
) -> None:
    await _seed(session, ["s1"])
    store = InMemorySessionStore()
    svc = LabelerService(session, store=store, class_set=CLASS_SET)

    # Alice labels s1 as billing
    a = await svc.start_session(
        annotator_id="alice", tenant_id="acme", task_id="routing",
    )
    await store.set((await store.get(a.session_id)).with_batch(["s1"]), 300)
    await svc.submit_label(
        session_id=a.session_id, sample_id="s1", label="billing",
    )
    await session.commit()

    # Bob labels s1 as refund — disagreement
    b = await svc.start_session(
        annotator_id="bob", tenant_id="acme", task_id="routing",
    )
    await store.set((await store.get(b.session_id)).with_batch(["s1"]), 300)
    result = await svc.submit_label(
        session_id=b.session_id, sample_id="s1", label="refund",
    )
    await session.commit()

    assert result.triggered_escalation
    assert result.escalation_ref is not None
