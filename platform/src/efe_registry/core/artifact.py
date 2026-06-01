"""Artifact dataclass + role/state enums.

``Artifact`` is an immutable view the rest of the package consumes;
it is hydrated from :class:`efe_registry.db.models.Artifact` ORM rows
via :meth:`Artifact.from_row`.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime
from enum import Enum
from typing import Final

from efe_registry.db.models import Artifact as ArtifactRow


class ArtifactState(str, Enum):
    """9-state FSM per architecture doc §5.1."""

    DRAFT = "draft"
    UPLOADED = "uploaded"
    SIGNED = "signed"
    ADMITTED = "admitted"
    SERVING = "serving"
    CHAMPION = "champion"
    DEPRECATED = "deprecated"
    PURGED = "purged"
    QUARANTINED = "quarantined"
    REJECTED = "rejected"


class ArtifactRole(str, Enum):
    """Endpoint / routing role of an artifact instance."""

    CHAMPION = "champion"
    CANDIDATE = "candidate"
    SHADOW = "shadow"
    STANDBY_WARM = "standby_warm"


TERMINAL_STATES: Final[frozenset[ArtifactState]] = frozenset({
    ArtifactState.PURGED,
})


def artifact_state_values() -> frozenset[str]:
    return frozenset(s.value for s in ArtifactState)


def artifact_role_values() -> frozenset[str]:
    return frozenset(r.value for r in ArtifactRole)


@dataclass(frozen=True, slots=True)
class Artifact:
    """Immutable snapshot of an artifact at read time."""

    artifact_id: str
    tenant_id: str
    task_id: str
    semver: str
    content_hash: bytes
    oci_ref: str
    s3_url: str
    parent_run_id: str
    parent_decision_id: str | None
    size_bytes: int
    framework: str
    state: ArtifactState
    state_version: int
    created_at: datetime
    uploaded_at: datetime | None
    signed_at: datetime | None
    admitted_at: datetime | None
    served_at: datetime | None
    promoted_at: datetime | None
    deprecated_at: datetime | None
    purged_at: datetime | None

    @classmethod
    def from_row(cls, row: ArtifactRow) -> Artifact:
        return cls(
            artifact_id=row.artifact_id,
            tenant_id=row.tenant_id,
            task_id=row.task_id,
            semver=row.semver,
            content_hash=bytes(row.content_hash),
            oci_ref=row.oci_ref,
            s3_url=row.s3_url,
            parent_run_id=row.parent_run_id,
            parent_decision_id=row.parent_decision_id,
            size_bytes=row.size_bytes,
            framework=row.framework,
            state=ArtifactState(row.state),
            state_version=row.state_version,
            created_at=row.created_at,
            uploaded_at=row.uploaded_at,
            signed_at=row.signed_at,
            admitted_at=row.admitted_at,
            served_at=row.served_at,
            promoted_at=row.promoted_at,
            deprecated_at=row.deprecated_at,
            purged_at=row.purged_at,
        )

    @property
    def is_terminal(self) -> bool:
        return self.state in TERMINAL_STATES

    @property
    def content_hash_hex(self) -> str:
        return self.content_hash.hex()


__all__ = [
    "TERMINAL_STATES",
    "Artifact",
    "ArtifactRole",
    "ArtifactState",
    "artifact_role_values",
    "artifact_state_values",
]
