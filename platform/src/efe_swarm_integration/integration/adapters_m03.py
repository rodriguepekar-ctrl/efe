"""M03 Registry → M12 blob source adapter.

``efe_registry`` stores versioned artefacts ; M12's
:class:`ArtefactSyncer` needs a :class:`BlobSource`. This adapter
absorbs the M03 ``Artifact`` type and exposes a thin ``BlobSource``
over the artefact's bytes.

Because M03's in-memory ``Registry`` stores blobs as local files or
object-store references, the adapter accepts a callable that opens a
chunk stream for a given ``Artifact``.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import AsyncIterator, Awaitable, Callable

from efe_registry import Artifact  # absorbed import

from efe_swarm_integration.artefact.syncer import BlobSource

__all__ = ["M03ArtifactBlobSource", "chunks_from_bytes"]


async def chunks_from_bytes(
    data: bytes, *, chunk_size: int = 64 * 1024
) -> AsyncIterator[bytes]:
    """Helper : turn a ``bytes`` blob into an async chunk iterator."""
    for i in range(0, len(data), chunk_size):
        yield data[i : i + chunk_size]


@dataclass(slots=True)
class M03ArtifactBlobSource:
    """Concrete :class:`BlobSource` that reads an M03 artefact by id.

    Callers inject ``fetch_bytes`` — the specific way to materialise a
    blob varies per deployment (local file, S3, gRPC). This keeps the
    adapter independent of the concrete backend.
    """

    fetch_bytes: Callable[[str, str], Awaitable[bytes]]
    chunk_size: int = 64 * 1024

    async def iter_chunks(
        self, *, bucket: str, key: str
    ) -> AsyncIterator[bytes]:
        blob = await self.fetch_bytes(bucket, key)
        async for chunk in chunks_from_bytes(blob, chunk_size=self.chunk_size):
            yield chunk


# Keep the absorbed symbol discoverable.
_ = Artifact, BlobSource
