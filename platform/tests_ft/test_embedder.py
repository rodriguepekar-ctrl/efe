"""Tests for embedder worker + mock backend."""

from __future__ import annotations

from pathlib import Path

import numpy as np
import pytest
from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from efe_ft.db.models import EmbeddingCache, TrainingSample
from efe_ft.embedder import EmbedderWorker, MockEmbedder
from efe_ft.hashing import sha256_text
from efe_ft.storage import LocalBlobStore


def test_mock_embedder_deterministic() -> None:
    e = MockEmbedder(dim=64)
    a = e.embed("hello world")
    b = e.embed("hello world")
    assert np.array_equal(a, b)
    assert abs(np.linalg.norm(a) - 1.0) < 1e-6


def test_mock_embedder_different_texts_differ() -> None:
    e = MockEmbedder(dim=64)
    a = e.embed("first text")
    b = e.embed("second text")
    # Cosine < 0.5 for distinct inputs
    assert float(a @ b) < 0.5


async def test_worker_embeds_pending_samples(
    session: AsyncSession, blob_store: LocalBlobStore,
) -> None:
    for i in range(3):
        session.add(TrainingSample(
            id=f"s{i}", tenant_id="acme", task_id="routing",
            text=f"hello {i}", text_sha256=sha256_text(f"hello {i}"),
            status="pending", source="batch",
        ))
    await session.commit()

    worker = EmbedderWorker(
        session, backend=MockEmbedder(dim=32), blob_store=blob_store,
    )
    done = await worker.run_once(tenant_id="acme", task_id="routing")
    await session.commit()
    assert done == 3

    rows = list((await session.execute(select(TrainingSample))).scalars())
    for r in rows:
        assert r.embedding_sha256 is not None

    cache = list((await session.execute(select(EmbeddingCache))).scalars())
    assert len(cache) == 3
    assert all(c.dim == 32 for c in cache)
