"""Ingestion sink — persists :class:`FeatureSample` rows.

Sinks have the same shape as :class:`efe_rollout.metrics.ExposureSink` :
a single ``async write(sample)`` method. Production wires a DB-backed
:class:`DbRawEventSink`; tests use :class:`InMemoryRawEventSink`.
"""

from __future__ import annotations

from typing import Protocol

from efe_drift.records import FeatureSample


class RawEventSink(Protocol):
    async def write(
        self, sample: FeatureSample,
    ) -> None:  # pragma: no cover
        ...

    async def write_batch(
        self, samples: tuple[FeatureSample, ...],
    ) -> None:  # pragma: no cover
        ...


class InMemoryRawEventSink:
    """Append-only sink used in unit tests."""

    __slots__ = ("_rows",)

    def __init__(self) -> None:
        self._rows: list[FeatureSample] = []

    async def write(self, sample: FeatureSample) -> None:
        self._rows.append(sample)

    async def write_batch(self, samples: tuple[FeatureSample, ...]) -> None:
        self._rows.extend(samples)

    def all(self) -> tuple[FeatureSample, ...]:
        return tuple(self._rows)

    def for_feature(self, feature_id: str) -> tuple[FeatureSample, ...]:
        return tuple(s for s in self._rows if s.feature_id == feature_id)

    def for_tenant(self, tenant_id: str) -> tuple[FeatureSample, ...]:
        return tuple(s for s in self._rows if s.tenant_id == tenant_id)

    def clear(self) -> None:
        self._rows.clear()

    def __len__(self) -> int:
        return len(self._rows)


__all__ = [
    "InMemoryRawEventSink",
    "RawEventSink",
]
