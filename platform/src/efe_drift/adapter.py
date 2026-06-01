"""Top-level SDK — ``DriftClient``.

One-stop façade that wires catalog + extractors + ingestion pipeline +
baseline manager + L4 evaluator + L5 router for in-process use.
Application code constructs a :class:`DriftClient` via
:meth:`in_memory` or :meth:`from_settings` and calls :

* :meth:`register_feature`
* :meth:`ingest_event`
* :meth:`rollup_samples_to_snapshots`
* :meth:`push_snapshots` — make snapshots visible to the evaluator
* :meth:`evaluate_feature` — run the L4 drift detectors
* :meth:`handle_drift_results` — pipe through the L5 router
* :meth:`baseline_manager` / :meth:`rule_store` / :meth:`incident_store`

The client is **process-local**. Production wires one HTTP endpoint
per pod and the SDK talks to it over the wire.
"""

from __future__ import annotations

from datetime import datetime

from efe_drift.aggregation.rollup import FrozenBinSet, rollup_samples
from efe_drift.alerting.channels import ChannelRegistry
from efe_drift.alerting.incident import (
    AlertIncidentStore,
    InMemoryAlertIncidentStore,
)
from efe_drift.alerting.router import AlertRouter, RoutingDecision
from efe_drift.alerting.rule import AlertRuleStore, InMemoryAlertRuleStore
from efe_drift.baseline.manager import BaselineManager
from efe_drift.baseline.store import BaselineStore, InMemoryBaselineStore
from efe_drift.catalog.feature import FeatureSpec
from efe_drift.catalog.registry import FeatureCatalog
from efe_drift.clock import Clock, SystemClock
from efe_drift.config import DriftSettings
from efe_drift.detection import (
    DriftEvaluator,
    DriftResult,
    EvaluationSweep,
    InMemorySnapshotSource,
    MultiScaleWindow,
    SeverityThresholds,
    SnapshotSource,
)
from efe_drift.detection import (
    build_default_registry as build_default_detectors,
)
from efe_drift.extraction import build_default_registry
from efe_drift.extraction.registry import ExtractorRegistry
from efe_drift.ingestion.pipeline import IngestionPipeline
from efe_drift.ingestion.sink import InMemoryRawEventSink, RawEventSink
from efe_drift.nats.publisher import InMemoryPublisher, Publisher
from efe_drift.records import (
    BucketKind,
    FeatureEntry,
    FeatureSample,
    FeatureSnapshot,
    InferenceEvent,
)


class DriftClient:
    """Application-facing façade over :mod:`efe_drift`."""

    __slots__ = (
        "_baseline_manager",
        "_catalog",
        "_channels",
        "_clock",
        "_evaluator",
        "_extractors",
        "_incidents",
        "_pipeline",
        "_publisher",
        "_router",
        "_rules",
        "_settings",
        "_sink",
        "_snapshot_source",
    )

    def __init__(
        self,
        *,
        catalog: FeatureCatalog,
        extractors: ExtractorRegistry,
        pipeline: IngestionPipeline,
        baseline_manager: BaselineManager,
        sink: RawEventSink,
        publisher: Publisher,
        settings: DriftSettings,
        snapshot_source: SnapshotSource | None = None,
        evaluator: DriftEvaluator | None = None,
        rules: AlertRuleStore | None = None,
        incidents: AlertIncidentStore | None = None,
        channels: ChannelRegistry | None = None,
        router: AlertRouter | None = None,
        clock: Clock | None = None,
    ) -> None:
        self._catalog = catalog
        self._extractors = extractors
        self._pipeline = pipeline
        self._baseline_manager = baseline_manager
        self._sink = sink
        self._publisher = publisher
        self._settings = settings
        self._clock = clock or SystemClock()
        self._snapshot_source = snapshot_source or InMemorySnapshotSource()
        self._evaluator = evaluator or DriftEvaluator(
            catalog=catalog,
            baseline_manager=baseline_manager,
            snapshots=self._snapshot_source,
            detectors=build_default_detectors(),
            clock=self._clock,
        )
        self._rules = rules or InMemoryAlertRuleStore(clock=self._clock)
        self._incidents = incidents or InMemoryAlertIncidentStore(
            clock=self._clock,
        )
        self._channels = channels or ChannelRegistry()
        self._router = router or AlertRouter(
            rules=self._rules,
            incidents=self._incidents,
            channels=self._channels,
            clock=self._clock,
        )

    # ---- Factories -----------------------------------------------------

    @classmethod
    def in_memory(
        cls,
        *,
        settings: DriftSettings | None = None,
        clock: Clock | None = None,
        sink: RawEventSink | None = None,
        publisher: Publisher | None = None,
        baseline_store: BaselineStore | None = None,
        extractors: ExtractorRegistry | None = None,
    ) -> DriftClient:
        settings = settings or DriftSettings()
        clk = clock or SystemClock()
        extractors = extractors or build_default_registry()
        catalog = FeatureCatalog(
            extractor_lookup=extractors,
            clock=clk,
        )
        raw_sink: RawEventSink = sink or InMemoryRawEventSink()
        salt = bytes.fromhex(settings.extraction.user_id_salt_hex)
        if all(b == 0 for b in salt):
            # Tests often use the default zero salt — pad to non-zero
            # so :func:`hash_user_id` doesn't complain.
            salt = b"\x01" + salt[1:]
        pipeline = IngestionPipeline(
            catalog=catalog,
            extractors=extractors,
            sinks=[raw_sink],
            user_id_salt=salt,
            clock=clk,
        )
        store = baseline_store or InMemoryBaselineStore()
        pub = publisher or InMemoryPublisher()
        signing_key = bytes.fromhex(settings.baseline.signing_key_hex)
        if all(b == 0 for b in signing_key):
            signing_key = b"\x01" + signing_key[1:]
        manager = BaselineManager(
            store=store,
            signing_key=signing_key,
            signing_key_id=settings.baseline.signing_key_id,
            default_overlap_days=settings.baseline.default_overlap_days,
            clock=clk,
            publisher=pub,
        )
        return cls(
            catalog=catalog,
            extractors=extractors,
            pipeline=pipeline,
            baseline_manager=manager,
            sink=raw_sink,
            publisher=pub,
            settings=settings,
            clock=clk,
        )

    # ---- Accessors -----------------------------------------------------

    @property
    def catalog(self) -> FeatureCatalog:
        return self._catalog

    @property
    def extractors(self) -> ExtractorRegistry:
        return self._extractors

    @property
    def baseline_manager(self) -> BaselineManager:
        return self._baseline_manager

    @property
    def sink(self) -> RawEventSink:
        return self._sink

    @property
    def publisher(self) -> Publisher:
        return self._publisher

    @property
    def evaluator(self) -> DriftEvaluator:
        return self._evaluator

    @property
    def snapshot_source(self) -> SnapshotSource:
        return self._snapshot_source

    @property
    def rule_store(self) -> AlertRuleStore:
        return self._rules

    @property
    def incident_store(self) -> AlertIncidentStore:
        return self._incidents

    @property
    def channel_registry(self) -> ChannelRegistry:
        return self._channels

    @property
    def router(self) -> AlertRouter:
        return self._router

    # ---- High-level operations ----------------------------------------

    async def register_feature(
        self, spec: FeatureSpec, *, created_by: str = "system",
    ) -> FeatureEntry:
        return await self._catalog.register(spec, created_by=created_by)

    async def ingest_event(
        self, event: InferenceEvent,
    ) -> tuple[FeatureSample, ...]:
        return await self._pipeline.process(event)

    async def ingest_events(
        self, events: tuple[InferenceEvent, ...],
    ) -> tuple[FeatureSample, ...]:
        return await self._pipeline.process_batch(events)

    def rollup_samples_to_snapshots(
        self,
        samples: tuple[FeatureSample, ...],
        *,
        bucket_kind: BucketKind = "minute",
        frozen: FrozenBinSet | None = None,
    ) -> tuple[FeatureSnapshot, ...]:
        return rollup_samples(
            samples,
            catalog=self._catalog,
            bucket_kind=bucket_kind,
            frozen=frozen,
            top_k_categorical=self._settings.aggregation.top_k_categorical,
            reservoir_size=self._settings.aggregation.embedding_reservoir_size,
        )

    def push_snapshots(
        self, snapshots: tuple[FeatureSnapshot, ...],
    ) -> None:
        """Make snapshots visible to the evaluator's snapshot source.

        Only works when the default :class:`InMemorySnapshotSource`
        is in use. Production deployments write snapshots to
        TimescaleDB directly and the evaluator reads from there.
        """
        source = self._snapshot_source
        if not isinstance(source, InMemorySnapshotSource):
            raise TypeError(
                "push_snapshots only available with InMemorySnapshotSource ; "
                "write to the DB-backed source via your ingestion pipeline instead",
            )
        source.push_many(snapshots)

    async def evaluate_feature(
        self,
        feature: FeatureEntry | str,
        *,
        now: datetime | None = None,
        scales: tuple[MultiScaleWindow, ...] = ("1h", "1d", "7d"),
        thresholds: SeverityThresholds | None = None,
    ) -> EvaluationSweep:
        """Run the L4 evaluator over one feature's active baseline.

        ``feature`` may be a :class:`FeatureEntry` or the
        ``feature_id`` — the catalog resolves strings automatically.
        """
        entry = (
            feature if isinstance(feature, FeatureEntry)
            else self._catalog.get(feature)
        )
        return await self._evaluator.evaluate_feature(
            entry, now=now, scales=scales, thresholds=thresholds,
        )

    async def handle_drift_results(
        self, results: tuple[DriftResult, ...],
    ) -> tuple[RoutingDecision, ...]:
        """Pipe a batch of drift results through the L5 router."""
        return await self._router.handle_many(results)

    async def ack_incident(
        self, incident_id: str, *, actor: str,
    ) -> None:
        """Acknowledge a firing incident via the incident store."""
        await self._incidents.ack(
            incident_id, actor=actor, now=self._clock.now(),
        )


__all__ = ["DriftClient"]
