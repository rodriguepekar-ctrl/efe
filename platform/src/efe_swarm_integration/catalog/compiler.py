"""Compile a :class:`CatalogYaml` into a :class:`ConeSpec` ready for
registration with :class:`admin.AdminClient`.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Protocol, Sequence

from efe_swarm_integration.catalog.validator import validate_catalog_yaml
from efe_swarm_integration.catalog.yaml_loader import CatalogYaml
from efe_swarm_integration.errors import CatalogCompilationFailed
from efe_swarm_integration.types import (
    AlignmentCone,
    Anchor,
    ConeSpec,
)

__all__ = ["CatalogCompiler", "EmbedderService"]


class EmbedderService(Protocol):
    """Convert text → embedding vector."""

    async def embed(self, *texts: str) -> list[tuple[float, ...]]: ...  # pragma: no cover


@dataclass(slots=True)
class CatalogCompiler:
    embedder: EmbedderService

    async def compile(
        self,
        catalog: CatalogYaml,
        *,
        created_by: str,
        pii_linter_fn=None,
    ) -> tuple[ConeSpec, list[str]]:
        """Return ``(ConeSpec, ordered_labels)``.

        The Rust server stores anchors as a flat vector ; the compiler
        keeps the ordered ``[(label, anchor_idx)…]`` mapping in
        ``ordered_labels`` so the reconciler can map back.
        """
        validate_catalog_yaml(catalog, pii_linter=pii_linter_fn)
        ordered_labels: list[str] = []
        texts: list[str] = []
        for lb in catalog.labels:
            for anchor in lb.anchors:
                ordered_labels.append(lb.label)
                texts.append(anchor)
        vectors = await self.embedder.embed(*texts)
        if len(vectors) != len(texts):
            raise CatalogCompilationFailed(
                f"embedder returned {len(vectors)} vectors for {len(texts)} texts"
            )
        anchors = tuple(Anchor(values=tuple(float(x) for x in v)) for v in vectors)
        if any(a.dim() != catalog.dim for a in anchors):
            raise CatalogCompilationFailed(
                f"embedder dim mismatch (expected {catalog.dim})"
            )
        cone = AlignmentCone(
            anchors=anchors,
            max_projection_distance=catalog.max_projection_distance,
        )
        spec = ConeSpec(
            tenant=catalog.tenant,
            task=catalog.task,
            version=catalog.version,
            cone=cone,
            created_by=created_by,
            description=f"compiled from {catalog.catalog_id}",
            labels=(
                ("catalog_id", catalog.catalog_id),
                ("locale", catalog.locale),
                ("embedder_family", catalog.embedder.family),
                ("embedder_model", catalog.embedder.model),
                ("embedder_version", catalog.embedder.version),
            ),
        )
        return spec, ordered_labels


# Keep Sequence as a module-level alias for typing downstream.
_Labels = Sequence[str]
_ = datetime, timezone  # reserved for future created_at stamping
