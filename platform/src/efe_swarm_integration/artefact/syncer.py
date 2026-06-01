"""Artefact syncer — copy from M03 object store to the Rust bucket."""

from __future__ import annotations

import uuid
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any, Protocol

from efe_swarm_integration.artefact.digest import compute_sha256_stream
from efe_swarm_integration.artefact.manifest import ArtefactManifest, seal_manifest
from efe_swarm_integration.errors import (
    ArtefactDigestMismatch,
    ArtefactSyncFailed,
)

__all__ = [
    "ArtefactSyncOutcome",
    "ArtefactSyncer",
    "BlobSource",
    "BucketPort",
]


class BlobSource(Protocol):
    async def iter_chunks(self, *, bucket: str, key: str) -> Any: ...  # pragma: no cover


class BucketPort(Protocol):
    async def put(
        self, *, bucket: str, key: str, chunks: Any
    ) -> None: ...  # pragma: no cover


@dataclass(frozen=True, slots=True)
class ArtefactSyncOutcome:
    ok: bool
    manifest: ArtefactManifest | None = None
    error: str | None = None


@dataclass(slots=True)
class ArtefactSyncer:
    source: BlobSource
    target: BucketPort
    target_bucket: str

    async def sync(
        self,
        *,
        tenant_id: str,
        artefact_id: str,
        source_bucket: str,
        source_key: str,
        expected_digest: bytes,
        dim: int,
        tokenizer_family: str | None = None,
        labels: tuple[tuple[str, str], ...] = (),
    ) -> ArtefactSyncOutcome:
        """Copy the blob, verify the digest, persist a sealed manifest."""
        try:
            # ``iter_chunks`` may be an async function returning an async
            # iterator, or a method that directly IS an async iterator.
            # Normalise both cases.
            maybe_iter = self.source.iter_chunks(bucket=source_bucket, key=source_key)
            if hasattr(maybe_iter, "__aiter__"):
                chunks_iter = maybe_iter
            else:
                chunks_iter = await maybe_iter  # type: ignore[assignment]
        except Exception as exc:
            raise ArtefactSyncFailed(
                f"source.iter_chunks failed: {exc}"
            ) from exc

        # Collect chunks for digest verification and put to target.
        collected: list[bytes] = []
        async for chunk in chunks_iter:  # type: ignore[attr-defined]
            collected.append(bytes(chunk))
        actual_digest = compute_sha256_stream(iter(collected))
        if actual_digest != expected_digest:
            raise ArtefactDigestMismatch(
                "source digest mismatch",
                detail={
                    "expected": expected_digest.hex(),
                    "actual": actual_digest.hex(),
                },
            )

        target_key = f"{tenant_id}/{artefact_id}"
        try:
            await self.target.put(
                bucket=self.target_bucket,
                key=target_key,
                chunks=iter(collected),
            )
        except Exception as exc:
            raise ArtefactSyncFailed(f"target.put failed: {exc}") from exc

        manifest = ArtefactManifest(
            manifest_id=uuid.uuid4().hex,
            tenant_id=tenant_id,
            artefact_id=artefact_id,
            digest=actual_digest,
            dim=int(dim),
            tokenizer_family=tokenizer_family,
            source_bucket=source_bucket,
            target_bucket=self.target_bucket,
            synced_at=datetime.now(tz=timezone.utc),
            labels=labels,
        )
        sealed = seal_manifest(manifest)
        return ArtefactSyncOutcome(ok=True, manifest=sealed)
