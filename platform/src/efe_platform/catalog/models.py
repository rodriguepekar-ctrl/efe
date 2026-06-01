"""Typed data model for a platform-level catalog."""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import date
from typing import Any


@dataclass(frozen=True, slots=True)
class InlineAnchor:
    """A user-authored anchor vector embedded directly in the catalog."""

    id: str
    values: tuple[float, ...]

    def __post_init__(self) -> None:
        if not self.id:
            raise ValueError("anchor.id must be non-empty")
        if not self.values:
            raise ValueError("anchor.values must be non-empty")


@dataclass(frozen=True, slots=True)
class CatalogTask:
    """A single named task exposed by the catalog."""

    id: str
    description: str
    tenant: str
    cone_key: str
    dim_latent: int
    anchors: tuple[str, ...] = ()
    tags: tuple[str, ...] = ()

    def __post_init__(self) -> None:
        if not self.id:
            raise ValueError("task.id must be non-empty")
        if not self.tenant:
            raise ValueError("task.tenant must be non-empty")
        if not self.cone_key.startswith(f"{self.tenant}/"):
            raise ValueError(
                f"cone_key {self.cone_key!r} must start with tenant/{self.tenant}/"
            )
        if self.dim_latent <= 0:
            raise ValueError("task.dim_latent must be > 0")


@dataclass(frozen=True, slots=True)
class CatalogMetadata:
    """Free-form metadata block."""

    owner: str = ""
    created_at: date | None = None
    tags: tuple[str, ...] = ()


@dataclass(frozen=True, slots=True)
class Catalog:
    """A platform catalog as a whole."""

    version: int
    tasks: tuple[CatalogTask, ...]
    anchors: tuple[InlineAnchor, ...] = ()
    metadata: CatalogMetadata = field(default_factory=CatalogMetadata)

    def __post_init__(self) -> None:
        if self.version < 1:
            raise ValueError("catalog.version must be >= 1")
        ids = [t.id for t in self.tasks]
        if len(ids) != len(set(ids)):
            raise ValueError("duplicate task id in catalog")
        anchor_ids = [a.id for a in self.anchors]
        if len(anchor_ids) != len(set(anchor_ids)):
            raise ValueError("duplicate anchor id in catalog")

    def get_task(self, task_id: str) -> CatalogTask:
        for t in self.tasks:
            if t.id == task_id:
                return t
        raise KeyError(f"task {task_id!r} not found in catalog")

    def by_tenant(self, tenant: str) -> tuple[CatalogTask, ...]:
        return tuple(t for t in self.tasks if t.tenant == tenant)

    def to_dict(self) -> dict[str, Any]:
        return {
            "version": self.version,
            "tasks": [
                {
                    "id": t.id,
                    "description": t.description,
                    "tenant": t.tenant,
                    "cone_key": t.cone_key,
                    "dim_latent": t.dim_latent,
                    "anchors": list(t.anchors),
                    "tags": list(t.tags),
                }
                for t in self.tasks
            ],
            "anchors": [
                {"id": a.id, "values": list(a.values)} for a in self.anchors
            ],
            "metadata": {
                "owner": self.metadata.owner,
                "created_at": (
                    self.metadata.created_at.isoformat()
                    if self.metadata.created_at
                    else None
                ),
                "tags": list(self.metadata.tags),
            },
        }


__all__ = ["Catalog", "CatalogMetadata", "CatalogTask", "InlineAnchor"]
