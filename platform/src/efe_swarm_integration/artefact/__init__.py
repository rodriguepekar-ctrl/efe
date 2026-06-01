"""Artefact sync — M05 → swarm-facing bucket."""

from __future__ import annotations

from efe_swarm_integration.artefact.digest import compute_sha256
from efe_swarm_integration.artefact.manifest import (
    ArtefactManifest,
    seal_manifest,
    verify_manifest_seal,
)
from efe_swarm_integration.artefact.syncer import (
    ArtefactSyncOutcome,
    ArtefactSyncer,
    BucketPort,
    BlobSource,
)

__all__ = [
    "ArtefactManifest",
    "ArtefactSyncOutcome",
    "ArtefactSyncer",
    "BlobSource",
    "BucketPort",
    "compute_sha256",
    "seal_manifest",
    "verify_manifest_seal",
]
