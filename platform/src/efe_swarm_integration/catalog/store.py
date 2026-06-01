"""Feature_id / task_id ↔ ConeRef binding store."""

from __future__ import annotations

import asyncio
import uuid
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Protocol

from efe_swarm_integration.errors import ConeAlreadyExists, ConeRefNotBound
from efe_swarm_integration.types import ConeRef

__all__ = ["CatalogBinding", "ConeBindingStore", "InMemoryConeBindingStore"]


@dataclass(frozen=True, slots=True)
class CatalogBinding:
    binding_id: str
    tenant_id: str
    feature_id: str
    task_id: str | None
    cone_ref: ConeRef
    embedder_family: str
    embedder_version: str
    anchor_count: int
    labels: tuple[str, ...]
    created_at: datetime
    disabled_at: datetime | None = None
    state_version: int = 1


class ConeBindingStore(Protocol):
    async def upsert(self, binding: CatalogBinding) -> CatalogBinding: ...  # pragma: no cover
    async def get(self, *, tenant_id: str, feature_id: str, task_id: str | None = None) -> CatalogBinding: ...  # pragma: no cover
    async def list(self, *, tenant_id: str) -> list[CatalogBinding]: ...  # pragma: no cover
    async def disable(self, binding_id: str, *, now: datetime) -> bool: ...  # pragma: no cover


@dataclass(slots=True)
class InMemoryConeBindingStore:
    _by_key: dict[tuple[str, str, str | None], CatalogBinding] = field(default_factory=dict)
    _by_id: dict[str, CatalogBinding] = field(default_factory=dict)
    _lock: asyncio.Lock = field(default_factory=asyncio.Lock, repr=False)

    async def upsert(self, binding: CatalogBinding) -> CatalogBinding:
        key = (binding.tenant_id, binding.feature_id, binding.task_id)
        async with self._lock:
            existing = self._by_key.get(key)
            if existing is not None and existing.disabled_at is None:
                raise ConeAlreadyExists(
                    f"binding already exists for {key} (binding_id={existing.binding_id})",
                )
            self._by_key[key] = binding
            self._by_id[binding.binding_id] = binding
            return binding

    async def get(
        self,
        *,
        tenant_id: str,
        feature_id: str,
        task_id: str | None = None,
    ) -> CatalogBinding:
        key = (tenant_id, feature_id, task_id)
        async with self._lock:
            binding = self._by_key.get(key)
        if binding is None or binding.disabled_at is not None:
            raise ConeRefNotBound(
                f"no active cone bound for {key}",
                detail={"tenant_id": tenant_id, "feature_id": feature_id, "task_id": task_id},
            )
        return binding

    async def list(self, *, tenant_id: str) -> list[CatalogBinding]:
        async with self._lock:
            return [
                b for b in self._by_key.values()
                if b.tenant_id == tenant_id and b.disabled_at is None
            ]

    async def disable(self, binding_id: str, *, now: datetime) -> bool:
        async with self._lock:
            existing = self._by_id.get(binding_id)
            if existing is None or existing.disabled_at is not None:
                return False
            updated = CatalogBinding(
                binding_id=existing.binding_id,
                tenant_id=existing.tenant_id,
                feature_id=existing.feature_id,
                task_id=existing.task_id,
                cone_ref=existing.cone_ref,
                embedder_family=existing.embedder_family,
                embedder_version=existing.embedder_version,
                anchor_count=existing.anchor_count,
                labels=existing.labels,
                created_at=existing.created_at,
                disabled_at=now,
                state_version=existing.state_version + 1,
            )
            self._by_id[binding_id] = updated
            key = (existing.tenant_id, existing.feature_id, existing.task_id)
            self._by_key[key] = updated
            return True


_ = uuid  # keep import
