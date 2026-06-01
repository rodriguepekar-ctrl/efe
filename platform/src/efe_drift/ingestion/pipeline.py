"""Ingestion pipeline — glue between ingestion sources + extractors + sink.

Given a :class:`~efe_drift.records.InferenceEvent`, the pipeline :

1. Looks up **every** enabled feature in the catalog for the
   ``(tenant, task)`` scope of the event.
2. For each feature, resolves its extractor, runs it, and — if the
   result is not ``None`` — pairs the :class:`FeatureValue` with the
   observation context to build a :class:`FeatureSample`.
3. Validates the produced :class:`FeatureValue` matches the declared
   ``kind`` (raises :class:`ExtractorTypeMismatch` otherwise).
4. Pushes the batch to one or more sinks.

The pipeline is **idempotent** and **order-preserving** per event —
no async fan-out that might re-order samples.

Because feature evaluation is bounded by the size of the catalog (a
few dozen features in practice), the pipeline runs in a tight
synchronous loop. Only the sink write is awaited.
"""

from __future__ import annotations

from collections.abc import Iterable

from efe_drift.catalog.registry import FeatureCatalog
from efe_drift.clock import Clock, SystemClock
from efe_drift.errors import ExtractorMissing
from efe_drift.extraction.base import assert_extractor_produces
from efe_drift.extraction.registry import ExtractorRegistry
from efe_drift.hashing import hash_user_id
from efe_drift.ids import new_uuid
from efe_drift.ingestion.sink import RawEventSink
from efe_drift.records import FeatureSample, InferenceEvent


class IngestionPipeline:
    """Feature-extraction + sink fan-out for a single event.

    The pipeline is stateless besides its dependencies : the catalog,
    the extractor registry, and the sinks. This makes it trivially
    horizontally scalable — one pipeline instance per pod, shared
    across tasks.
    """

    __slots__ = (
        "_catalog",
        "_clock",
        "_extractors",
        "_sinks",
        "_user_id_salt",
    )

    def __init__(
        self,
        *,
        catalog: FeatureCatalog,
        extractors: ExtractorRegistry,
        sinks: Iterable[RawEventSink],
        user_id_salt: bytes,
        clock: Clock | None = None,
    ) -> None:
        self._catalog = catalog
        self._extractors = extractors
        self._sinks = tuple(sinks)
        if len(user_id_salt) < 16:
            raise ValueError(
                f"user_id_salt too short ({len(user_id_salt)} bytes, "
                "need 16+)",
            )
        self._user_id_salt = bytes(user_id_salt)
        self._clock = clock or SystemClock()

    # ---- Public API ----------------------------------------------------

    async def process(self, event: InferenceEvent) -> tuple[FeatureSample, ...]:
        """Run the full pipeline for a single event.

        Returns the list of :class:`FeatureSample` rows persisted. The
        list is empty if no feature matched (e.g. the event is for an
        unknown tenant / task).
        """
        samples = tuple(self._build_samples(event))
        if samples:
            for sink in self._sinks:
                await sink.write_batch(samples)
        return samples

    async def process_batch(
        self, events: tuple[InferenceEvent, ...],
    ) -> tuple[FeatureSample, ...]:
        """Micro-batched version — one sink call per batch."""
        all_samples: list[FeatureSample] = []
        for event in events:
            all_samples.extend(self._build_samples(event))
        samples = tuple(all_samples)
        if samples:
            for sink in self._sinks:
                await sink.write_batch(samples)
        return samples

    # ---- Internals -----------------------------------------------------

    def _build_samples(
        self, event: InferenceEvent,
    ) -> Iterable[FeatureSample]:
        entries = self._catalog.list(
            tenant_id=event.tenant_id,
            task_id=event.task_id,
            enabled_only=True,
        )
        if not entries:
            return ()
        user_hash = (
            hash_user_id(event.user_id, salt=self._user_id_salt)
            if event.user_id else None
        )
        samples: list[FeatureSample] = []
        for entry in entries:
            try:
                extractor = self._extractors.get(entry.extractor)
            except ExtractorMissing:
                # This *should* be caught at register time. If it
                # slips through (registry cleared post-register) we
                # surface it loudly rather than silently dropping.
                raise
            value = extractor.extract(event)
            if value is None:
                continue
            # Guardrail — catch extractor kind drift.
            assert_extractor_produces(value, entry.kind)
            samples.append(
                FeatureSample(
                    sample_id=new_uuid(),
                    tenant_id=event.tenant_id,
                    task_id=event.task_id,
                    feature_id=entry.feature_id,
                    feature_name=entry.name,
                    observed_at=event.observed_at,
                    value=value,
                    user_id_hash=user_hash,
                    request_id=event.request_id,
                ),
            )
        return samples


__all__ = ["IngestionPipeline"]
