"""FastAPI factory — the drift HTTP surface.

Routes exposed (see M09 §16.3) :

* ``POST /v1/drift/samples``                  — ingest one inference event
* ``POST /v1/drift/samples/bulk``             — bulk ingest
* ``GET  /v1/drift/features``                 — list registered features
* ``POST /v1/drift/features``                 — register new feature
* ``GET  /v1/drift/baselines``                — list baselines
* ``POST /v1/drift/baselines/{id}/promote``   — candidate → active
* ``POST /v1/drift/baselines/{id}/retire``    — emergency retire
* ``GET  /v1/drift/evaluations``              — latest drift results
* ``POST /v1/drift/evaluate/{feature_id}``    — run the evaluator
* ``GET  /v1/drift/incidents``                — list incidents
* ``POST /v1/drift/incidents/{id}/ack``       — ack incident
* ``GET  /v1/drift/rules``                    — list alert rules
* ``POST /v1/drift/rules``                    — create alert rule
* ``DELETE /v1/drift/rules/{id}``             — delete alert rule
* ``GET /healthz``                            — liveness

FastAPI is imported lazily to keep :mod:`efe_drift` importable
without the ``api`` extra. Routes are registered with
``add_api_route`` to avoid the ``Untyped decorator`` pattern that
caused headaches in other modules.
"""

from __future__ import annotations

from datetime import timezone
from typing import Any

from efe_drift.adapter import DriftClient
from efe_drift.alerting import (
    AlertRouter,
    AlertRule,
    AlertRuleStore,
    ChannelRef,
    InMemoryAlertRuleStore,
)
from efe_drift.alerting.incident import AlertIncidentStore
from efe_drift.api.schemas import (
    AckIncidentRequest,
    AlertIncidentSchema,
    AlertRuleCreateRequest,
    AlertRuleSchema,
    BaselineSchema,
    ChannelRefSchema,
    DriftResultSchema,
    EmergencyRetireRequest,
    FeatureEntrySchema,
    FeatureSpecSchema,
    InferenceEventSchema,
    IngestAck,
    ThresholdsSchema,
)
from efe_drift.catalog.feature import FeatureSpec
from efe_drift.clock import Clock, SystemClock
from efe_drift.detection import (
    DriftEvaluator,
    DriftResult,
    SeverityThresholds,
)
from efe_drift.errors import DriftError
from efe_drift.records import Baseline, FeatureEntry, InferenceEvent

# ---------------------------------------------------------------------------
# AppContext — dependency container
# ---------------------------------------------------------------------------


class AppContext:
    """Container shared across route handlers."""

    __slots__ = (
        "_clock",
        "client",
        "evaluator",
        "incidents",
        "router",
        "rules",
    )

    def __init__(
        self,
        *,
        client: DriftClient,
        evaluator: DriftEvaluator | None,
        router: AlertRouter | None,
        rules: AlertRuleStore | None = None,
        incidents: AlertIncidentStore | None = None,
        clock: Clock | None = None,
    ) -> None:
        self.client = client
        self.evaluator = evaluator
        self.router = router
        self.rules = rules or InMemoryAlertRuleStore(clock=clock)
        self.incidents = incidents
        self._clock = clock or SystemClock()

    @property
    def clock(self) -> Clock:
        return self._clock


# ---------------------------------------------------------------------------
# Factory
# ---------------------------------------------------------------------------


def build_app(ctx: AppContext) -> Any:
    try:
        from fastapi import FastAPI, Request
        from fastapi.responses import JSONResponse
    except ImportError as exc:  # pragma: no cover
        raise RuntimeError(
            "FastAPI not installed; cannot build HTTP app",
        ) from exc

    app = FastAPI(
        title="efe-drift",
        version="0.1.0",
        openapi_url="/openapi.json",
    )

    async def _err_handler(request: Request, exc: Exception) -> Any:
        if isinstance(exc, DriftError):
            return JSONResponse(
                status_code=exc.http_status, content=exc.to_dict(),
            )
        return JSONResponse(
            status_code=500,
            content={"code": "unexpected", "message": str(exc)},
        )

    app.add_exception_handler(DriftError, _err_handler)

    async def healthz() -> dict[str, str]:
        return {"status": "ok"}

    app.add_api_route("/healthz", healthz, methods=["GET"])

    _register_feature_routes(app, ctx)
    _register_ingest_routes(app, ctx)
    _register_baseline_routes(app, ctx)
    _register_evaluation_routes(app, ctx)
    _register_rule_routes(app, ctx)
    _register_incident_routes(app, ctx)
    return app


# ---------------------------------------------------------------------------
# Routes — features
# ---------------------------------------------------------------------------


def _register_feature_routes(app: Any, ctx: AppContext) -> None:
    async def list_features(
        tenant_id: str | None = None, task_id: str | None = None,
    ) -> list[FeatureEntrySchema]:
        rows = ctx.client.catalog.list(
            tenant_id=tenant_id, task_id=task_id,
        )
        return [_feature_to_schema(r) for r in rows]

    async def register_feature(
        req: FeatureSpecSchema,
    ) -> FeatureEntrySchema:
        spec = FeatureSpec(
            tenant_id=req.tenant_id,
            task_id=req.task_id,
            name=req.name,
            kind=req.kind,
            extractor=req.extractor,
            binning_strategy=req.binning_strategy,
            version=req.version,
            expected_cardinality=req.expected_cardinality,
            retention_days=req.retention_days,
            notes=req.notes,
        )
        entry = await ctx.client.register_feature(spec)
        return _feature_to_schema(entry)

    app.add_api_route(
        "/v1/drift/features", list_features, methods=["GET"],
        response_model=list[FeatureEntrySchema],
    )
    app.add_api_route(
        "/v1/drift/features", register_feature, methods=["POST"],
        response_model=FeatureEntrySchema,
    )


# ---------------------------------------------------------------------------
# Routes — ingest
# ---------------------------------------------------------------------------


def _register_ingest_routes(app: Any, ctx: AppContext) -> None:
    async def ingest_event(req: InferenceEventSchema) -> IngestAck:
        event = _event_from_schema(req)
        samples = await ctx.client.ingest_event(event)
        return IngestAck(
            samples_written=len(samples),
            features_matched=len(samples),
        )

    async def ingest_bulk(
        req: list[InferenceEventSchema],
    ) -> IngestAck:
        events = tuple(_event_from_schema(r) for r in req)
        samples = await ctx.client.ingest_events(events)
        return IngestAck(
            samples_written=len(samples),
            features_matched=len(samples),
        )

    app.add_api_route(
        "/v1/drift/samples", ingest_event, methods=["POST"],
        response_model=IngestAck,
    )
    app.add_api_route(
        "/v1/drift/samples/bulk", ingest_bulk, methods=["POST"],
        response_model=IngestAck,
    )


# ---------------------------------------------------------------------------
# Routes — baselines
# ---------------------------------------------------------------------------


def _register_baseline_routes(app: Any, ctx: AppContext) -> None:
    async def list_baselines(
        tenant_id: str | None = None,
        task_id: str | None = None,
        state: str | None = None,
    ) -> list[BaselineSchema]:
        rows = await ctx.client.baseline_manager.list(
            tenant_id=tenant_id,
            task_id=task_id,
            state=state if state is None else _coerce_state(state),
        )
        return [_baseline_to_schema(r) for r in rows]

    async def promote_baseline(
        baseline_id: str,
    ) -> BaselineSchema:
        active, _, _ = await ctx.client.baseline_manager.promote(
            baseline_id,
        )
        return _baseline_to_schema(active)

    async def retire_baseline(
        baseline_id: str, req: EmergencyRetireRequest,
    ) -> BaselineSchema:
        retired = await ctx.client.baseline_manager.emergency_retire(
            baseline_id, actor=req.actor, reason=req.reason,
        )
        return _baseline_to_schema(retired)

    app.add_api_route(
        "/v1/drift/baselines", list_baselines, methods=["GET"],
        response_model=list[BaselineSchema],
    )
    app.add_api_route(
        "/v1/drift/baselines/{baseline_id}/promote", promote_baseline,
        methods=["POST"], response_model=BaselineSchema,
    )
    app.add_api_route(
        "/v1/drift/baselines/{baseline_id}/retire", retire_baseline,
        methods=["POST"], response_model=BaselineSchema,
    )


# ---------------------------------------------------------------------------
# Routes — evaluations
# ---------------------------------------------------------------------------


def _register_evaluation_routes(app: Any, ctx: AppContext) -> None:
    async def run_evaluation(
        feature_id: str,
    ) -> list[DriftResultSchema]:
        if ctx.evaluator is None:
            raise DriftError(
                "no evaluator configured",
                detail={"hint": "inject DriftEvaluator in AppContext"},
            )
        feature = ctx.client.catalog.get(feature_id)
        sweep = await ctx.evaluator.evaluate_feature(feature)
        rows = [_result_to_schema(r) for r in sweep.results]
        # Pipe through router if configured.
        if ctx.router is not None:
            await ctx.router.handle_many(sweep.results)
        return rows

    app.add_api_route(
        "/v1/drift/evaluate/{feature_id}", run_evaluation,
        methods=["POST"], response_model=list[DriftResultSchema],
    )


# ---------------------------------------------------------------------------
# Routes — alert rules
# ---------------------------------------------------------------------------


def _register_rule_routes(app: Any, ctx: AppContext) -> None:
    async def list_rules(
        tenant_id: str | None = None,
    ) -> list[AlertRuleSchema]:
        rules = await ctx.rules.list(tenant_id=tenant_id)
        return [_rule_to_schema(r) for r in rules]

    async def create_rule(
        req: AlertRuleCreateRequest,
    ) -> AlertRuleSchema:
        thresholds = SeverityThresholds(
            warning=req.thresholds.warning,
            major=req.thresholds.major,
            critical=req.thresholds.critical,
            orientation=req.thresholds.orientation,
        )
        channels = tuple(
            ChannelRef(
                channel=c.channel, min_severity=c.min_severity,
            )
            for c in req.channels
        )
        store = ctx.rules
        if not isinstance(store, InMemoryAlertRuleStore):
            raise DriftError("rule store does not support create")
        rule = await store.create(
            tenant_id=req.tenant_id,
            method=req.method,  # type: ignore[arg-type]
            thresholds=thresholds,
            channels=channels,
            task_id=req.task_id,
            feature_id=req.feature_id,
            window_kind=req.window_kind,
            consecutive_breaches=req.consecutive_breaches,
            suppression_s=req.suppression_s,
            name=req.name,
            notes=req.notes,
        )
        return _rule_to_schema(rule)

    async def delete_rule(rule_id: str) -> dict[str, Any]:
        await ctx.rules.delete(rule_id)
        return {"ok": True}

    app.add_api_route(
        "/v1/drift/rules", list_rules, methods=["GET"],
        response_model=list[AlertRuleSchema],
    )
    app.add_api_route(
        "/v1/drift/rules", create_rule, methods=["POST"],
        response_model=AlertRuleSchema,
    )
    app.add_api_route(
        "/v1/drift/rules/{rule_id}", delete_rule, methods=["DELETE"],
    )


# ---------------------------------------------------------------------------
# Routes — incidents
# ---------------------------------------------------------------------------


def _register_incident_routes(app: Any, ctx: AppContext) -> None:
    async def list_incidents(
        tenant_id: str | None = None,
        state: str | None = None,
    ) -> list[AlertIncidentSchema]:
        if ctx.incidents is None:
            return []
        rows = await ctx.incidents.list(
            tenant_id=tenant_id,
            state=state if state is None else _coerce_incident_state(state),
        )
        return [_incident_to_schema(r) for r in rows]

    async def ack_incident(
        incident_id: str, req: AckIncidentRequest,
    ) -> AlertIncidentSchema:
        if ctx.incidents is None:
            raise DriftError("no incident store configured")
        acked = await ctx.incidents.ack(
            incident_id,
            actor=req.acked_by,
            now=ctx.clock.now(),
        )
        return _incident_to_schema(acked)

    app.add_api_route(
        "/v1/drift/incidents", list_incidents, methods=["GET"],
        response_model=list[AlertIncidentSchema],
    )
    app.add_api_route(
        "/v1/drift/incidents/{incident_id}/ack", ack_incident,
        methods=["POST"], response_model=AlertIncidentSchema,
    )


# ---------------------------------------------------------------------------
# Converters
# ---------------------------------------------------------------------------


def _feature_to_schema(entry: FeatureEntry) -> FeatureEntrySchema:
    return FeatureEntrySchema(
        feature_id=entry.feature_id,
        tenant_id=entry.tenant_id,
        task_id=entry.task_id,
        name=entry.name,
        version=entry.version,
        kind=entry.kind,
        extractor=entry.extractor,
        binning_strategy=entry.binning_strategy,
        expected_cardinality=entry.expected_cardinality,
        retention_days=entry.retention_days,
        enabled=entry.enabled,
        notes=entry.notes,
    )


def _event_from_schema(req: InferenceEventSchema) -> InferenceEvent:
    observed_at = req.observed_at
    if observed_at.tzinfo is None:
        observed_at = observed_at.replace(tzinfo=timezone.utc)
    return InferenceEvent(
        event_id=req.event_id,
        tenant_id=req.tenant_id,
        task_id=req.task_id,
        user_id=req.user_id,
        observed_at=observed_at,
        request_id=req.request_id,
        prompt_token_count=req.prompt_token_count,
        response_token_count=req.response_token_count,
        latency_ms=req.latency_ms,
        cost_usd=req.cost_usd,
        model_id=req.model_id,
        backend_kind=req.backend_kind,
        temperature=req.temperature,
        top_p=req.top_p,
        prompt_text=req.prompt_text,
        response_text=req.response_text,
        covariates=req.covariates,
    )


def _baseline_to_schema(b: Baseline) -> BaselineSchema:
    return BaselineSchema(
        baseline_id=b.baseline_id,
        tenant_id=b.spec.tenant_id,
        task_id=b.spec.task_id,
        feature_name=b.spec.feature_name,
        feature_kind=b.spec.feature_kind,
        state=b.state,
        state_version=b.state_version,
        window_start=b.spec.window_start,
        window_end=b.spec.window_end,
        sample_count=b.spec.sample_count,
        promoted_at=b.promoted_at,
        retired_at=b.retired_at,
        planned_end_at=b.planned_end_at,
    )


def _result_to_schema(r: DriftResult) -> DriftResultSchema:
    return DriftResultSchema(
        evaluation_id=r.evaluation_id,
        tenant_id=r.tenant_id,
        task_id=r.task_id,
        feature_id=r.feature_id,
        feature_name=r.feature_name,
        baseline_id=r.baseline_id,
        method=r.method,
        score=r.score,
        severity=r.severity,
        computed_at=r.computed_at,
        window_start=r.window_start,
        window_end=r.window_end,
        p_value=r.p_value,
        multi_scale_window=r.multi_scale_window,
        details=r.details,
        sample_count_current=r.sample_count_current,
        sample_count_baseline=r.sample_count_baseline,
    )


def _rule_to_schema(rule: AlertRule) -> AlertRuleSchema:
    return AlertRuleSchema(
        rule_id=rule.rule_id,
        tenant_id=rule.tenant_id,
        task_id=rule.task_id,
        feature_id=rule.feature_id,
        method=rule.method,
        window_kind=rule.window_kind,
        consecutive_breaches=rule.consecutive_breaches,
        suppression_s=rule.suppression_s,
        enabled=rule.enabled,
        thresholds=ThresholdsSchema(
            warning=rule.thresholds.warning,
            major=rule.thresholds.major,
            critical=rule.thresholds.critical,
            orientation=rule.thresholds.orientation,
        ),
        channels=[
            ChannelRefSchema.model_validate({
                "channel": c.channel,
                "min_severity": c.min_severity,
            })
            for c in rule.channels
        ],
        name=rule.name,
        notes=rule.notes,
    )


def _incident_to_schema(inc: Any) -> AlertIncidentSchema:
    return AlertIncidentSchema(
        incident_id=inc.incident_id,
        rule_id=inc.rule_id,
        tenant_id=inc.tenant_id,
        feature_id=inc.feature_id,
        feature_name=inc.feature_name,
        method=inc.method,
        severity=inc.severity,
        score_at_fire=inc.score_at_fire,
        state=inc.state,
        fired_at=inc.fired_at,
        acked_at=inc.acked_at,
        acked_by=inc.acked_by,
        resolved_at=inc.resolved_at,
        window_kind=inc.window_kind,
        dispatch_results=inc.dispatch_results,
    )


def _coerce_state(state: str) -> Any:
    allowed = {
        "planned", "accumulating", "candidate", "active",
        "retiring", "retired", "rejected",
    }
    if state not in allowed:
        raise DriftError(
            f"unknown baseline state {state!r}",
            detail={"allowed": sorted(allowed)},
        )
    return state


def _coerce_incident_state(state: str) -> Any:
    allowed = {"firing", "acked", "resolved"}
    if state not in allowed:
        raise DriftError(
            f"unknown incident state {state!r}",
            detail={"allowed": sorted(allowed)},
        )
    return state


__all__ = ["AppContext", "build_app"]
