"""Background embedder worker.

Pulls ``training_samples`` rows without ``embedding_sha256``, computes
the embedding, writes it to the blob store, and upserts
``embedding_cache``.

For simplicity the worker polls Postgres rather than relying on a
Redis queue. With a small WHERE-clause index on ``embedding_sha256
IS NULL`` that's plenty fast for MVP volumes. Phase 2 may move to a
pull-based Redis stream.
"""

from __future__ import annotations

from typing import Protocol

import numpy as np
from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from efe_ft.audit import AuditChain
from efe_ft.db.models import EmbeddingCache, TrainingSample
from efe_ft.hashing import sha256_bytes
from efe_ft.storage import BlobStore


class EmbeddingBackend(Protocol):
    @property
    def dim(self) -> int: ...
    @property
    def version(self) -> str: ...
    def embed(self, text: str) -> np.ndarray: ...
    def embed_batch(self, texts: list[str]) -> np.ndarray: ...


class EmbedderWorker:
    """One pass over pending samples for one tenant/task."""

    def __init__(
        self,
        session: AsyncSession,
        *,
        backend: EmbeddingBackend,
        blob_store: BlobStore,
        batch_size: int = 32,
    ) -> None:
        self._session = session
        self._backend = backend
        self._store = blob_store
        self._batch_size = batch_size

    async def run_once(
        self, *, tenant_id: str, task_id: str, limit: int = 1000,
    ) -> int:
        """Embed up to ``limit`` pending samples. Returns the count done."""
        processed = 0
        while processed < limit:
            stmt = (
                select(TrainingSample)
                .where(
                    TrainingSample.tenant_id == tenant_id,
                    TrainingSample.task_id == task_id,
                    TrainingSample.embedding_sha256.is_(None),
                    TrainingSample.status != "archived",
                )
                .order_by(TrainingSample.created_at.asc())
                .limit(min(self._batch_size, limit - processed))
            )
            result = await self._session.execute(stmt)
            batch = list(result.scalars())
            if not batch:
                break
            texts = [row.text for row in batch]
            vecs = self._backend.embed_batch(texts)
            await self._persist(batch, vecs, tenant_id)
            processed += len(batch)
        return processed

    async def _persist(
        self,
        batch: list[TrainingSample],
        vecs: np.ndarray,
        tenant_id: str,
    ) -> None:
        chain = AuditChain(self._session, tenant_id)
        for row, vec in zip(batch, vecs, strict=True):
            blob = vec.astype(np.float32).tobytes()
            key = self._storage_key(tenant_id, row.task_id, row.text_sha256)
            checksum = await self._store.put(key, blob)
            # Upsert cache entry (skip if already exists).
            cache_key = {
                "text_sha256": row.text_sha256,
                "embedder_version": self._backend.version,
            }
            existing = await self._session.get(
                EmbeddingCache, (row.text_sha256, self._backend.version),
            )
            if existing is None:
                self._session.add(EmbeddingCache(
                    **cache_key,
                    dim=self._backend.dim,
                    s3_uri=key,
                    bytes_size=len(blob),
                    checksum=checksum,
                ))
            row.embedding_sha256 = sha256_bytes(blob)
            await chain.record(
                kind="embedding_cached",
                actor_id="system:embedder",
                subject_id=row.id,
                payload={
                    "embedder_version": self._backend.version,
                    "dim": self._backend.dim,
                    "bytes": len(blob),
                },
            )
        await self._session.flush()

    @staticmethod
    def _storage_key(tenant_id: str, task_id: str, sha: bytes) -> str:
        hex_s = sha.hex()
        return f"{tenant_id}/{task_id}/embeddings/{hex_s[:2]}/{hex_s[2:]}.npy"


__all__ = ["EmbedderWorker", "EmbeddingBackend"]
