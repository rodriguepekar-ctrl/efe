"""End-to-end tests for the ingestion pipeline."""

from __future__ import annotations

import pytest
from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from efe_ft.audit import verify_chain
from efe_ft.db.models import AuditEvent, TrainingSample
from efe_ft.ingest import IngestInput, IngestPipeline, SampleIngestPayload
from efe_ft.ingest.dedup import InMemoryDeduper


def _payload(text: str, label: str | None = None,
             tenant: str = "acme", task: str = "routing") -> IngestInput:
    return IngestInput(payload=SampleIngestPayload(
        tenant_id=tenant, task_id=task, text=text,
        source="batch", actor_id="alice@acme", label=label,
    ))


async def test_single_sample_accepted(
    session: AsyncSession, deduper: InMemoryDeduper,
) -> None:
    pipeline = IngestPipeline(session, deduper=deduper, check_balance=False)
    outcome = await pipeline.run(
        [_payload("bonjour le monde")],
        tenant_id="acme", task_id="routing", actor_id="alice@acme",
    )
    await session.commit()
    assert outcome.summary.n_accepted == 1
    assert outcome.summary.n_rejected == 0
    rows = list((await session.execute(select(TrainingSample))).scalars())
    assert len(rows) == 1
    assert rows[0].text == "bonjour le monde"


async def test_duplicate_rejected(
    session: AsyncSession, deduper: InMemoryDeduper,
) -> None:
    pipeline = IngestPipeline(session, deduper=deduper, check_balance=False)
    outcome = await pipeline.run(
        [_payload("identique"), _payload("identique")],
        tenant_id="acme", task_id="routing", actor_id="alice@acme",
    )
    await session.commit()
    assert outcome.summary.n_accepted == 1
    assert outcome.summary.rejections == {"duplicate": 1}


async def test_pii_masked(
    session: AsyncSession, deduper: InMemoryDeduper,
) -> None:
    pipeline = IngestPipeline(
        session, deduper=deduper, pii_policy="mask", check_balance=False,
    )
    outcome = await pipeline.run(
        [_payload("Contact alice@acme.fr pour l'incident")],
        tenant_id="acme", task_id="routing", actor_id="alice@acme",
    )
    await session.commit()
    assert outcome.summary.n_accepted == 1
    row = list((await session.execute(select(TrainingSample))).scalars())[0]
    assert "<EMAIL>" in row.text
    assert "email" in row.pii_flags


async def test_pii_reject_policy(
    session: AsyncSession, deduper: InMemoryDeduper,
) -> None:
    pipeline = IngestPipeline(
        session, deduper=deduper, pii_policy="reject", check_balance=False,
    )
    outcome = await pipeline.run(
        [_payload("Mon IBAN est FR76 3000 4000 0300 1234 5678 901")],
        tenant_id="acme", task_id="routing", actor_id="alice@acme",
    )
    await session.commit()
    assert outcome.summary.n_accepted == 0
    assert outcome.summary.rejections == {"pii_blocked": 1}


async def test_audit_chain_intact_after_batch(
    session: AsyncSession, deduper: InMemoryDeduper,
) -> None:
    pipeline = IngestPipeline(session, deduper=deduper, check_balance=False)
    await pipeline.run(
        [_payload("first"), _payload("second"), _payload("third")],
        tenant_id="acme", task_id="routing", actor_id="alice@acme",
    )
    await session.commit()
    ok, broken = await verify_chain(session, "acme")
    assert ok
    assert broken is None
    audit = list((await session.execute(select(AuditEvent))).scalars())
    assert len(audit) >= 3
    assert all(e.kind == "ingest_accepted" for e in audit)


async def test_balance_rejects_skewed_batch(
    session: AsyncSession, deduper: InMemoryDeduper,
) -> None:
    # Seed: 50 billing + 1 refund. Distinct sha256 per row.
    from efe_ft.hashing import sha256_text
    for i in range(50):
        session.add(TrainingSample(
            id=f"b{i}", tenant_id="acme", task_id="routing",
            text=f"billing seed {i}",
            text_sha256=sha256_text(f"billing seed {i}"),
            label="billing", source="batch", status="labelled",
        ))
    session.add(TrainingSample(
        id="r1", tenant_id="acme", task_id="routing",
        text="refund seed 1", text_sha256=sha256_text("refund seed 1"),
        label="refund", source="batch", status="labelled",
    ))
    await session.commit()

    pipeline = IngestPipeline(
        session, deduper=deduper, balance_skew_max=10.0,
    )
    # Attempt to add 60 billing — would push skew to 110 vs 1.
    inputs = [_payload(f"billing new {i}", label="billing")
              for i in range(60)]
    outcome = await pipeline.run(
        inputs, tenant_id="acme", task_id="routing", actor_id="alice@acme",
    )
    await session.commit()
    assert outcome.summary.n_accepted == 0
    assert outcome.summary.rejections == {"balance_skew": 60}


async def test_mixed_batch_per_item_isolation(
    session: AsyncSession, deduper: InMemoryDeduper,
) -> None:
    pipeline = IngestPipeline(session, deduper=deduper, check_balance=False)
    outcome = await pipeline.run(
        [
            _payload("first valid"),
            _payload("first valid"),            # dup
            _payload("second alice@acme.fr"),   # PII mask, accepted
            _payload("third and unique"),
        ],
        tenant_id="acme", task_id="routing", actor_id="alice@acme",
    )
    await session.commit()
    assert outcome.summary.n_accepted == 3
    assert outcome.summary.n_rejected == 1
    assert outcome.summary.rejections == {"duplicate": 1}
