"""Artefact manifest + HMAC seal."""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any

from efe_swarm_integration.hashing import (
    DOMAIN_TAG_ARTEFACT_SYNC,
    hmac_seal,
    verify_seal,
)

__all__ = [
    "ArtefactManifest",
    "seal_manifest",
    "verify_manifest_seal",
]


@dataclass(frozen=True, slots=True)
class ArtefactManifest:
    manifest_id: str
    tenant_id: str
    artefact_id: str
    digest: bytes
    dim: int
    tokenizer_family: str | None
    source_bucket: str
    target_bucket: str
    synced_at: datetime
    labels: tuple[tuple[str, str], ...] = field(default_factory=tuple)
    seal: bytes = b""

    def __post_init__(self) -> None:
        if self.synced_at.tzinfo is None:
            raise ValueError("synced_at must be tz-aware")
        if len(self.digest) != 32:
            raise ValueError("digest must be 32 bytes (SHA-256)")

    def canonical(self) -> dict[str, Any]:
        return {
            "manifest_id": self.manifest_id,
            "tenant_id": self.tenant_id,
            "artefact_id": self.artefact_id,
            "digest": self.digest.hex(),
            "dim": int(self.dim),
            "tokenizer_family": self.tokenizer_family,
            "source_bucket": self.source_bucket,
            "target_bucket": self.target_bucket,
            "synced_at": self.synced_at.astimezone(timezone.utc).isoformat(),
            "labels": sorted(self.labels),
        }

    def sealed(self) -> "ArtefactManifest":
        mac = hmac_seal(DOMAIN_TAG_ARTEFACT_SYNC, self.canonical())
        return ArtefactManifest(
            manifest_id=self.manifest_id,
            tenant_id=self.tenant_id,
            artefact_id=self.artefact_id,
            digest=self.digest,
            dim=self.dim,
            tokenizer_family=self.tokenizer_family,
            source_bucket=self.source_bucket,
            target_bucket=self.target_bucket,
            synced_at=self.synced_at,
            labels=self.labels,
            seal=mac,
        )


def seal_manifest(manifest: ArtefactManifest) -> ArtefactManifest:
    return manifest.sealed()


def verify_manifest_seal(manifest: ArtefactManifest) -> bool:
    if not manifest.seal:
        return False
    return verify_seal(
        DOMAIN_TAG_ARTEFACT_SYNC, manifest.canonical(), manifest.seal
    )
