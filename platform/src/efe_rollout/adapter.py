"""Top-level SDK — ``RolloutClient``.

The :class:`RolloutClient` is the single entry point for application
code that wants to :

* ask "what variant should this user get?"
* record an exposure + outcome,
* request an on-demand analysis.

It wires together :mod:`efe_rollout.assignment`,
:mod:`efe_rollout.metrics`, and :mod:`efe_rollout.analysis` with
sensible defaults. Tests construct a fully in-memory client via
:meth:`RolloutClient.in_memory`; production builds one via
:meth:`RolloutClient.from_settings`.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from efe_rollout.analysis.analyzer import AnalysisInputs, ExperimentAnalyzer
from efe_rollout.assignment.assigner import AssignmentDecision, HashAssigner
from efe_rollout.assignment.cache import AssignmentCache, InMemoryAssignmentCache
from efe_rollout.assignment.forced import (
    ForcedAssignmentStore,
    InMemoryForcedAssignmentStore,
)
from efe_rollout.assignment.holdout import HoldoutFilter
from efe_rollout.clock import Clock, SystemClock
from efe_rollout.config import RolloutSettings
from efe_rollout.errors import ExperimentNotRunning
from efe_rollout.layers import LayerRegistry
from efe_rollout.metrics.exposure import (
    ExposureContext,
    ExposureRecorder,
    ExposureSink,
    InMemoryExposureSink,
)
from efe_rollout.metrics.outcome import (
    InMemoryOutcomeSink,
    OutcomeContext,
    OutcomeRecorder,
    OutcomeSink,
)
from efe_rollout.records import AnalysisResult, Exposure, Outcome
from efe_rollout.registry import ExperimentRegistry, InMemoryRegistryStore


@dataclass(frozen=True, slots=True)
class VariantResolution:
    """What the client returns from :meth:`RolloutClient.get_variant`.

    ``variant_id`` is ``None`` for holdout / outside-traffic users so
    the caller knows to fall back on the reference model.
    """

    variant_id: str | None
    variant_name: str | None
    variant_payload: dict[str, Any]
    holdout: bool
    forced: bool


class RolloutClient:
    """High-level façade over the rollout module.

    Instances are cheap and safe to share across a process.
    """

    __slots__ = (
        "_analyzer",
        "_assigner",
        "_clock",
        "_exposures",
        "_outcomes",
        "_registry",
        "_settings",
    )

    def __init__(
        self,
        *,
        registry: ExperimentRegistry,
        assigner: HashAssigner,
        exposures: ExposureRecorder,
        outcomes: OutcomeRecorder,
        analyzer: ExperimentAnalyzer,
        settings: RolloutSettings,
        clock: Clock | None = None,
    ) -> None:
        self._registry = registry
        self._assigner = assigner
        self._exposures = exposures
        self._outcomes = outcomes
        self._analyzer = analyzer
        self._settings = settings
        self._clock = clock or SystemClock()

    # ------------------------------------------------------------------
    # Factories
    # ------------------------------------------------------------------

    @classmethod
    def in_memory(
        cls,
        *,
        settings: RolloutSettings | None = None,
        clock: Clock | None = None,
        exposure_sink: ExposureSink | None = None,
        outcome_sink: OutcomeSink | None = None,
    ) -> RolloutClient:
        settings = settings or RolloutSettings()
        clk = clock or SystemClock()
        layers = LayerRegistry.from_settings(settings.layers)
        holdout = HoldoutFilter(settings.assignment.holdout_basis_points)
        forced: ForcedAssignmentStore = InMemoryForcedAssignmentStore()
        cache: AssignmentCache = InMemoryAssignmentCache()
        assigner = HashAssigner(
            layers=layers,
            holdout=holdout,
            forced=forced,
            cache=cache,
        )
        store = InMemoryRegistryStore()
        prereg_key = bytes.fromhex(settings.prereg.signing_key_hex)
        registry = ExperimentRegistry(
            store=store,
            signing_key=prereg_key,
            signing_key_id=settings.prereg.signing_key_id,
            clock=clk,
        )
        exposure_key = bytes.fromhex(settings.ingest.exposure_hmac_key_hex)
        exposure_recorder = ExposureRecorder(
            sinks=[exposure_sink or InMemoryExposureSink()],
            hmac_key=exposure_key,
            clock=clk,
        )
        outcome_recorder = OutcomeRecorder(
            sinks=[outcome_sink or InMemoryOutcomeSink()],
            clock=clk,
            strict_itt=settings.ingest.strict_itt,
        )
        analyzer = ExperimentAnalyzer(clock=clk)
        return cls(
            registry=registry,
            assigner=assigner,
            exposures=exposure_recorder,
            outcomes=outcome_recorder,
            analyzer=analyzer,
            settings=settings,
            clock=clk,
        )

    # ------------------------------------------------------------------
    # High-level operations
    # ------------------------------------------------------------------

    @property
    def registry(self) -> ExperimentRegistry:
        return self._registry

    async def get_variant(
        self,
        *,
        user_id: str,
        experiment_id: str,
        record_exposure: bool = True,
        request_id: str | None = None,
    ) -> VariantResolution:
        exp = await self._registry.get(experiment_id)
        if exp.state != "running":
            raise ExperimentNotRunning(
                f"experiment {experiment_id!r} is {exp.state!r}, "
                "not running",
            )
        decision = await self._assigner.assign(
            user_id=user_id,
            experiment_id=experiment_id,
            spec=exp.spec,
        )
        if record_exposure:
            await self._exposures.record(
                ExposureContext(
                    tenant_id=exp.spec.tenant_id,
                    experiment_id=experiment_id,
                    user_id=user_id,
                    layer=exp.spec.layer,
                    request_id=request_id,
                ),
                decision,
            )
        return _variant_resolution(decision)

    async def record_outcome(
        self,
        *,
        experiment_id: str,
        user_id: str,
        metric_name: str,
        metric_value: float,
        variant_id: str | None = None,
        request_id: str | None = None,
        covariate: dict[str, Any] | None = None,
    ) -> Outcome:
        exp = await self._registry.get(experiment_id)
        return await self._outcomes.record(
            OutcomeContext(
                tenant_id=exp.spec.tenant_id,
                experiment_id=experiment_id,
                user_id=user_id,
                variant_id=variant_id,
                metric_name=metric_name,
                metric_value=metric_value,
                request_id=request_id,
                covariate=covariate or {},
            ),
        )

    async def analyze(
        self,
        experiment_id: str,
        *,
        exposures: tuple[Exposure, ...],
        outcomes: tuple[Outcome, ...],
        primary_metric: str | None = None,
        apply_cuped_covariate: str | None = None,
        kind: str = "final",
    ) -> AnalysisResult:
        exp = await self._registry.get(experiment_id)
        inputs = AnalysisInputs(
            spec=exp.spec,
            exposures=exposures,
            outcomes=outcomes,
            primary_metric=primary_metric or exp.spec.primary_metric,
            kind=kind,  # type: ignore[arg-type]
            apply_cuped_covariate=apply_cuped_covariate,
            apply_cluster_robust=self._settings.analysis.apply_cluster_robust_se,
            bayesian_n_samples=self._settings.analysis.bayesian_posterior_samples,
            alpha=self._settings.analysis.default_alpha,
        )
        return self._analyzer.analyze(inputs)


def _variant_resolution(decision: AssignmentDecision) -> VariantResolution:
    if decision.variant is None:
        return VariantResolution(
            variant_id=None,
            variant_name=None,
            variant_payload={},
            holdout=decision.holdout,
            forced=decision.forced,
        )
    return VariantResolution(
        variant_id=decision.variant.variant_id,
        variant_name=decision.variant.name,
        variant_payload=dict(decision.variant.payload),
        holdout=False,
        forced=decision.forced,
    )


__all__ = ["RolloutClient", "VariantResolution"]
