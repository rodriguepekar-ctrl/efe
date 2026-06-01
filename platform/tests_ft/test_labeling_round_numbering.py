"""Regression: round_num counts DISTINCT prior non-expert annotators.

Before the fix, ``round = 1 + len(prior_rows)`` over-counted when the
same annotator re-labelled a sample. This test pins the new behaviour:

* round = 1 for the first annotator
* round = 2 for the second distinct annotator (dual-label pass)
* re-labelling by an annotator who already touched the sample keeps
  their row at round = 1 (they are not a "second" annotator).
"""

from __future__ import annotations

from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from efe_ft.db.models import LabelHistory, TrainingSample
from efe_ft.hashing import sha256_text
from efe_ft.labeling.labeler_service import LabelerService
from efe_ft.labeling.session import InMemorySessionStore

CLASS_SET = frozenset(["billing", "refund", "auth"])


async def _seed(session: AsyncSession, ids: list[str]) -> None:
    for sid in ids:
        session.add(TrainingSample(
            id=sid, tenant_id="acme", task_id="routing",
            text=f"t {sid}", text_sha256=sha256_text(sid),
            status="pending", source="batch",
        ))
    await session.commit()


async def test_round_counts_distinct_annotators(session: AsyncSession) -> None:
    await _seed(session, ["s1"])
    store = InMemorySessionStore()
    svc = LabelerService(session, store=store, class_set=CLASS_SET)

    # Alice — round should be 1
    a = await svc.start_session(
        annotator_id="alice", tenant_id="acme", task_id="routing",
    )
    await store.set((await store.get(a.session_id)).with_batch(["s1"]), 300)
    await svc.submit_label(
        session_id=a.session_id, sample_id="s1", label="billing",
    )
    await session.commit()

    rows = list((await session.execute(select(LabelHistory))).scalars())
    assert len(rows) == 1
    assert rows[0].round == 1

    # Alice re-labels the same sample — should STILL be round=1
    await store.set(
        (await store.get(a.session_id)).with_batch(["s1"]), 300,
    )
    await svc.submit_label(
        session_id=a.session_id, sample_id="s1", label="refund",
    )
    await session.commit()
    rows = list((await session.execute(
        select(LabelHistory).order_by(LabelHistory.created_at.asc())
    )).scalars())
    assert len(rows) == 2
    assert all(r.round == 1 for r in rows), f"rounds: {[r.round for r in rows]}"

    # Bob labels s1 — round should jump to 2 (dual-label)
    b = await svc.start_session(
        annotator_id="bob", tenant_id="acme", task_id="routing",
    )
    await store.set((await store.get(b.session_id)).with_batch(["s1"]), 300)
    await svc.submit_label(
        session_id=b.session_id, sample_id="s1", label="auth",
    )
    await session.commit()
    rows = list((await session.execute(
        select(LabelHistory).order_by(LabelHistory.created_at.asc())
    )).scalars())
    assert len(rows) == 3
    assert rows[-1].round == 2
    assert rows[-1].annotator_id == "bob"
