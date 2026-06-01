"""M03 (registry) + M05 (fine-tuning) → M12 artefact sync adapters.

M03 stores artefact blobs ; M05 emits events announcing that a newly
fine-tuned artefact is ready. M12's :class:`ArtefactSyncer` needs a
:class:`BlobSource` pointing to M03 and a trigger for M05 events.

Since M03 / M05 runtime APIs differ between deployments, this module
exposes *Protocol-based* adapters that take the minimum surface
required by M12 and expose it in the M12 shape.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, AsyncIterator, Protocol

from efe_swarm_integration.artefact.digest import compute_sha256
from efe_swarm_integration.artefact.syncer import (
    ArtefactSyncer,
    BlobSource,
    BucketPort,
)
from efe_swarm_integration.errors import ArtefactSyncFailed

__all__ = [
    "M03BlobReader",
    "M03RegistryBlobSource",
    "M05ArtefactEventHandler",
    "M05FtArtefactEvent",
]


# ---------------------------------------------------------------------------
# M03 — registry blob source
# ---------------------------------------------------------------------------


class M03BlobReader(Protocol):
    """Minimal contract :class:`efe_registry` must fulfil for M12."""

    async def read_blob(
        self, *, bucket: str, key: str
    ) -> AsyncIterator[bytes]: ...  # pragma: no cover


@dataclass(slots=True)
class M03RegistryBlobSource:
    """Concrete :class:`BlobSource` backed by M03.

    The inner reader is abstracted so M12 does not hard-depend on a
    specific M03 class name ; the deployment code wires a particular
    reader (``efe_registry.store.BlobStore`` or similar).
    """

    reader: M03BlobReader

    async def iter_chunks(
        self, *, bucket: str, key: str
    ) -> AsyncIterator[bytes]:
        # Pass-through — match the ``BlobSource`` shape.
        return await self.reader.read_blob(bucket=bucket, key=key)  # type: ignore[return-value]


# ---------------------------------------------------------------------------
# M05 — FT artefact event handler
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class M05FtArtefactEvent:
    """Normalised view of M05's ``artefact.published`` event."""

    tenant_id: str
    artefact_id: str
    source_bucket: str
    source_key: str
    digest: bytes
    dim: int
    tokenizer_family: str | None = None
    labels: tuple[tuple[str, str], ...] = field(default_factory=tuple)


@dataclass(slots=True)
class M05ArtefactEventHandler:
    """Bridge an M05 artefact event into :class:`ArtefactSyncer`.

    Typical wiring in production : a NATS subscriber on
    ``efe.ft.artefact.published`` calls
    :meth:`M05ArtefactEventHandler.handle` with the decoded payload.
    The handler calls the syncer and returns the resulting signed
    :class:`ArtefactManifest`.
    """

    syncer: ArtefactSyncer

    async def handle(
        self, event: M05FtArtefactEvent
    ) -> "ArtefactSyncOutcome":  # quote the forward ref
        if len(event.digest) != 32:
            raise ArtefactSyncFailed(
                f"bad digest length {len(event.digest)} (expected 32)",
                detail={"artefact_id": event.artefact_id},
            )
        outcome = await self.syncer.sync(
            tenant_id=event.tenant_id,
            artefact_id=event.artefact_id,
            source_bucket=event.source_bucket,
            source_key=event.source_key,
            expected_digest=event.digest,
            dim=event.dim,
            tokenizer_family=event.tokenizer_family,
            labels=event.labels,
        )
        return outcome


# Re-export for Sphinx + IDE navigation.
from efe_swarm_integration.artefact.syncer import ArtefactSyncOutcome  # noqa: E402


_ = BlobSource, BucketPort, Any, compute_sha256  # keep imports live
