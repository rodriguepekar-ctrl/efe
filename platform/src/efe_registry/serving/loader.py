"""Artifact download + verification pipeline.

At load time a serving pod:

1. Downloads the artifact from S3 into a local directory.
2. Verifies the ``model.safetensors`` signature (L3).
3. Hands control to a :class:`ServingAdapter` for engine creation.

This module contains the download + verify orchestration; the engine
creation belongs to :mod:`vllm_adapter`.
"""

from __future__ import annotations

import asyncio
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

from efe_registry.core.artifact import Artifact
from efe_registry.errors import (
    ArtifactVerificationFailed,
    ModelArtifactDownloadFailed,
)
from efe_registry.hashing import sha256_file
from efe_registry.signing.models import (
    VerificationInput,
    VerificationLayer,
)
from efe_registry.signing.verifier import SignatureVerifier


class S3Downloader(Protocol):
    async def download(
        self, s3_url: str, dest: Path,
    ) -> None:  # pragma: no cover
        ...


@dataclass
class InMemoryS3Downloader:
    """Test helper — copies preloaded files into dest."""

    files: dict[str, bytes]
    should_fail: bool = False

    async def download(self, s3_url: str, dest: Path) -> None:
        if self.should_fail:
            raise ModelArtifactDownloadFailed(
                f"simulated download failure for {s3_url!r}",
            )
        dest.mkdir(parents=True, exist_ok=True)
        for relpath, content in self.files.items():
            p = dest / relpath
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_bytes(content)


@dataclass(frozen=True, slots=True)
class LoadedArtifact:
    artifact_id: str
    local_root: Path
    content_hash_verified_hex: str


class ModelLoader:
    """Downloads + verifies an artifact onto local disk."""

    def __init__(
        self,
        *,
        downloader: S3Downloader,
        verifier: SignatureVerifier,
        local_root: Path,
        download_timeout_s: float = 600.0,
    ) -> None:
        self._downloader = downloader
        self._verifier = verifier
        self._root = local_root
        self._timeout_s = download_timeout_s

    async def load(self, artifact: Artifact) -> LoadedArtifact:
        dest = self._root / artifact.artifact_id
        blob = dest / "model.safetensors"

        # Skip download if already present with matching hash.
        if not blob.exists():
            try:
                await asyncio.wait_for(
                    self._downloader.download(artifact.s3_url, dest),
                    timeout=self._timeout_s,
                )
            except asyncio.TimeoutError:
                raise ModelArtifactDownloadFailed(
                    f"download timeout for {artifact.s3_url!r}",
                ) from None
            if not blob.exists():
                raise ModelArtifactDownloadFailed(
                    f"blob not present after download: {blob!s}",
                )

        # Compute local SHA-256 and compare with the registered content_hash.
        actual = sha256_file(blob)
        if actual != artifact.content_hash:
            raise ArtifactVerificationFailed(
                f"content_hash mismatch on blob {blob!s}: "
                f"expected {artifact.content_hash.hex()[:16]}…, "
                f"got {actual.hex()[:16]}…",
            )

        # Cosign L3 verify.
        sig = dest / "model.safetensors.sig"
        cert = dest / "model.safetensors.cert"
        if sig.exists():
            result = await self._verifier.verify(VerificationInput(
                artifact_id=artifact.artifact_id,
                layer=VerificationLayer.L3_PRE_LOAD,
                blob_path=blob,
                signature_path=sig,
                certificate_path=cert if cert.exists() else None,
            ))
            if not result.ok:
                raise ArtifactVerificationFailed(
                    f"cosign verify-blob failed for "
                    f"{artifact.artifact_id}: {result.details}",
                )

        return LoadedArtifact(
            artifact_id=artifact.artifact_id,
            local_root=dest,
            content_hash_verified_hex=actual.hex(),
        )


__all__ = [
    "InMemoryS3Downloader",
    "LoadedArtifact",
    "ModelLoader",
    "S3Downloader",
]
