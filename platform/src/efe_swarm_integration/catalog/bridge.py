"""CatalogBridge — public API that M10 / M06 call.

Flows :

* ``resolve(tenant, feature, task?) → ConeRef``.
* ``register_or_evolve(catalog_yaml, ordered_labels) → ConeRef``.
* ``evolve(feature, op) → ConeRef``.
* ``labels_for(binding) → Sequence[str]`` (stored in labels metadata).
"""

from __future__ import annotations

import uuid
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Sequence

from efe_swarm_integration.catalog.compiler import CatalogCompiler
from efe_swarm_integration.catalog.store import (
    CatalogBinding,
    ConeBindingStore,
)
from efe_swarm_integration.catalog.yaml_loader import CatalogYaml
from efe_swarm_integration.errors import ConeRefNotBound
from efe_swarm_integration.types import ConeRef, ConeSpec

__all__ = ["CatalogBridge", "CatalogBinding"]


@dataclass(slots=True)
class CatalogBridge:
    store: ConeBindingStore
    compiler: CatalogCompiler | None = None

    async def resolve(
        self, *, tenant_id: str, feature_id: str, task_id: str | None = None
    ) -> ConeRef:
        binding = await self.store.get(
            tenant_id=tenant_id, feature_id=feature_id, task_id=task_id
        )
        return binding.cone_ref

    async def register_binding(
        self,
        *,
        tenant_id: str,
        feature_id: str,
        task_id: str | None,
        cone_ref: ConeRef,
        embedder_family: str,
        embedder_version: str,
        anchor_count: int,
        labels: Sequence[str],
        now: datetime,
    ) -> CatalogBinding:
        binding = CatalogBinding(
            binding_id=uuid.uuid4().hex,
            tenant_id=tenant_id,
            feature_id=feature_id,
            task_id=task_id,
            cone_ref=cone_ref,
            embedder_family=embedder_family,
            embedder_version=embedder_version,
            anchor_count=anchor_count,
            labels=tuple(labels),
            created_at=now,
        )
        return await self.store.upsert(binding)

    async def compile_and_prepare(
        self,
        catalog: CatalogYaml,
        *,
        created_by: str,
        pii_linter=None,
    ) -> tuple[ConeSpec, list[str]]:
        if self.compiler is None:
            raise RuntimeError("CatalogBridge.compiler is not configured")
        return await self.compiler.compile(
            catalog, created_by=created_by, pii_linter_fn=pii_linter
        )

    async def labels_for(
        self,
        *,
        tenant_id: str,
        feature_id: str,
        task_id: str | None = None,
    ) -> tuple[str, ...]:
        try:
            binding = await self.store.get(
                tenant_id=tenant_id, feature_id=feature_id, task_id=task_id
            )
        except ConeRefNotBound:
            return ()
        return binding.labels


_ = datetime, timezone, field
