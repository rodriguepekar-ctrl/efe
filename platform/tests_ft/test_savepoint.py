"""Critical regression: savepoint isolation per sample in the pipeline.

Before the fix, an ``IntegrityError`` on one row would trigger
``session.rollback()`` and wipe out every prior accepted row in the
same batch. The fix wraps each row insertion in
``begin_nested()`` (a SQL SAVEPOINT) so only the offending row is
aborted.

This test seeds a sample, then submits a batch where the third sample
re-enters the same text (integrity violation). We assert that the
prior two rows are preserved AND the fourth+ rows continue to be
processed — the batch is NOT poisoned.
"""

from __future__ import annotations

from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from efe_ft.db.models import TrainingSample
from efe_ft.hashing import sha256_text
from efe_ft.ingest import (
    IngestInput,
    IngestPipeline,
    InMemoryDeduper,
    SampleIngestPayload,
)


def _payload(text: str) -> IngestInput:
    return IngestInput(payload=SampleIngestPayload(
        tenant_id="acme", task_id="routing", text=text,
        source="batch", actor_id="alice@acme",
    ))


async def test_integrity_error_is_isolated_per_sample(
    session: AsyncSession,
) -> None:
    """Planter une violation UNIQUE au milieu d'un batch ne doit pas
    pourrir les samples qui précèdent ni ceux qui suivent."""

    # 1) Pré-seed un sample avec un sha256 connu — la 3e entrée du batch
    #    collisera.
    colliding_text = "deja_present_en_base_row_preexistante"
    pre = TrainingSample(
        id="pre-existing",
        tenant_id="acme", task_id="routing",
        text=colliding_text,
        text_sha256=sha256_text(colliding_text),
        status="pending", source="batch",
    )
    session.add(pre)
    await session.commit()

    # 2) Le dedup InMemory ne connaît PAS encore `pre` (LSH vide).
    #    Le sample 3 va donc passer le dedup, puis fail sur UNIQUE.
    deduper = InMemoryDeduper()

    texts = [
        "sample un distinct complet",
        "sample deux avec un autre texte long",
        colliding_text,              # ← collision UNIQUE à l'INSERT
        "sample quatre different aussi",
        "sample cinq encore autre payload",
    ]
    pipeline = IngestPipeline(session, deduper=deduper, check_balance=False)
    outcome = await pipeline.run(
        [_payload(t) for t in texts],
        tenant_id="acme", task_id="routing", actor_id="alice@acme",
    )
    await session.commit()

    # 3) 2 avant + 2 après = 4 acceptés. La 3e est rejetée comme duplicate.
    assert outcome.summary.n_accepted == 4
    assert outcome.summary.n_rejected == 1
    assert outcome.summary.rejections == {"duplicate": 1}

    # 4) Vérif essentielle : les rows 1 et 2 sont bien persistées
    #    (le savepoint n'a pas rollback le batch entier).
    stmt = select(TrainingSample).where(
        TrainingSample.tenant_id == "acme",
        TrainingSample.task_id == "routing",
    )
    rows = list((await session.execute(stmt)).scalars())
    # pre-existing + 4 nouveaux acceptés = 5 au total
    assert len(rows) == 5
    texts_in_db = {r.text for r in rows}
    assert "sample un distinct complet" in texts_in_db
    assert "sample deux avec un autre texte long" in texts_in_db
    assert "sample quatre different aussi" in texts_in_db
    assert "sample cinq encore autre payload" in texts_in_db
