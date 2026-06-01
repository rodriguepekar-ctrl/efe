"""Feature catalog — the authoritative registry of monitored features.

Responsibilities :

* Accept :class:`FeatureSpec` registrations and store them by natural
  key.
* Return :class:`FeatureEntry` records with an assigned ``feature_id``.
* Enforce **uniqueness** per ``(tenant, task, name, version)``.
* Enforce that the declared ``extractor`` is registered in
  :mod:`efe_drift.extraction.registry` (catches typos at register
  time, not at ingestion time).
* Support **versioning** : re-registering under a higher version is
  allowed; re-registering under the same version is refused.
* Support **soft delete** via :meth:`disable`.

Thread / async safety : mutation is guarded by an :class:`asyncio.Lock`
identical to the pattern used in :mod:`efe_rollout.registry`.
"""

from __future__ import annotations

import asyncio
from dataclasses import replace
from typing import Protocol

from efe_drift.catalog.feature import FeatureSpec
from efe_drift.clock import Clock, SystemClock
from efe_drift.errors import (
    DuplicateFeature,
    FeatureNotFound,
    UnknownExtractor,
)
from efe_drift.ids import new_uuid
from efe_drift.records import FeatureEntry

# ---------------------------------------------------------------------------
# ExtractorLookup — injected to avoid a circular import from extraction
# ---------------------------------------------------------------------------


class ExtractorLookup(Protocol):
    """Shim over :class:`ExtractorRegistry.has`.

    We depend on the *capability* "does this extractor name exist?"
    but we do NOT want to import the :mod:`efe_drift.extraction.registry`
    module here (it depends on :mod:`efe_drift.records`, which would
    create an import cycle with :mod:`efe_drift.catalog.feature` if the
    modules grow in the future). A tiny Protocol keeps us honest.
    """

    def has(self, name: str) -> bool:  # pragma: no cover
        ...


class _AlwaysTrueLookup:
    """Default — accept any extractor name. Used in tests."""

    def has(self, name: str) -> bool:
        return True


# ---------------------------------------------------------------------------
# FeatureCatalog
# ---------------------------------------------------------------------------


class FeatureCatalog:
    """In-memory registry of :class:`FeatureEntry` records.

    For production a :class:`SqlFeatureCatalog` subclass persists into
    the ``feature_catalog`` table via SQLAlchemy — the Protocol
    contract below is identical.

    Every mutation path holds an :class:`asyncio.Lock` so concurrent
    ``register`` calls cannot race on the uniqueness check.
    """

    __slots__ = (
        "_by_id",
        "_by_key",
        "_clock",
        "_lock",
        "_lookup",
    )

    def __init__(
        self,
        *,
        extractor_lookup: ExtractorLookup | None = None,
        clock: Clock | None = None,
    ) -> None:
        self._by_id: dict[str, FeatureEntry] = {}
        self._by_key: dict[tuple[str, str, str, int], str] = {}
        self._lookup = extractor_lookup or _AlwaysTrueLookup()
        self._clock = clock or SystemClock()
        self._lock = asyncio.Lock()

    # ---- create / update -----------------------------------------------

    async def register(
        self, spec: FeatureSpec, *, created_by: str = "system",
    ) -> FeatureEntry:
        if not self._lookup.has(spec.extractor):
            raise UnknownExtractor(
                f"extractor {spec.extractor!r} not registered — register "
                "it before declaring the feature",
            )
        async with self._lock:
            if spec.natural_key in self._by_key:
                raise DuplicateFeature(
                    f"feature {spec.name!r} version {spec.version} already "
                    f"registered for tenant={spec.tenant_id!r}, "
                    f"task={spec.task_id!r}",
                )
            feature_id = new_uuid()
            entry = FeatureEntry(
                feature_id=feature_id,
                tenant_id=spec.tenant_id,
                task_id=spec.task_id,
                name=spec.name,
                version=spec.version,
                kind=spec.kind,
                extractor=spec.extractor,
                binning_strategy=spec.binning_strategy,
                expected_cardinality=spec.expected_cardinality,
                retention_days=spec.retention_days,
                created_at=self._clock.now(),
                created_by=created_by,
                enabled=True,
                notes=spec.notes,
            )
            self._by_id[feature_id] = entry
            self._by_key[spec.natural_key] = feature_id
            return entry

    async def disable(self, feature_id: str) -> FeatureEntry:
        async with self._lock:
            entry = self._by_id.get(feature_id)
            if entry is None:
                raise FeatureNotFound(
                    f"unknown feature_id {feature_id!r}",
                )
            if not entry.enabled:
                return entry
            updated = replace(entry, enabled=False)
            self._by_id[feature_id] = updated
            return updated

    async def enable(self, feature_id: str) -> FeatureEntry:
        async with self._lock:
            entry = self._by_id.get(feature_id)
            if entry is None:
                raise FeatureNotFound(
                    f"unknown feature_id {feature_id!r}",
                )
            if entry.enabled:
                return entry
            updated = replace(entry, enabled=True)
            self._by_id[feature_id] = updated
            return updated

    # ---- read -----------------------------------------------------------

    def get(self, feature_id: str) -> FeatureEntry:
        entry = self._by_id.get(feature_id)
        if entry is None:
            raise FeatureNotFound(f"unknown feature_id {feature_id!r}")
        return entry

    def get_by_name(
        self,
        *,
        tenant_id: str,
        task_id: str,
        name: str,
        version: int | None = None,
    ) -> FeatureEntry:
        if version is not None:
            eid = self._by_key.get((tenant_id, task_id, name, version))
            if eid is None:
                raise FeatureNotFound(
                    f"feature {name!r} version {version} not found "
                    f"for tenant={tenant_id!r}, task={task_id!r}",
                )
            return self._by_id[eid]
        # Latest enabled version for (tenant, task, name).
        candidates = [
            e for e in self._by_id.values()
            if e.tenant_id == tenant_id
            and e.task_id == task_id
            and e.name == name
            and e.enabled
        ]
        if not candidates:
            raise FeatureNotFound(
                f"no enabled feature {name!r} for tenant={tenant_id!r}, "
                f"task={task_id!r}",
            )
        return max(candidates, key=lambda e: e.version)

    def list(
        self,
        *,
        tenant_id: str | None = None,
        task_id: str | None = None,
        enabled_only: bool = False,
    ) -> tuple[FeatureEntry, ...]:
        rows = tuple(self._by_id.values())
        if tenant_id is not None:
            rows = tuple(e for e in rows if e.tenant_id == tenant_id)
        if task_id is not None:
            rows = tuple(e for e in rows if e.task_id == task_id)
        if enabled_only:
            rows = tuple(e for e in rows if e.enabled)
        return rows

    def exists(
        self,
        *,
        tenant_id: str,
        task_id: str,
        name: str,
        version: int,
    ) -> bool:
        return (tenant_id, task_id, name, version) in self._by_key


__all__ = ["ExtractorLookup", "FeatureCatalog"]
