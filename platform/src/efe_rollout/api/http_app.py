"""FastAPI factory — builds the rollout HTTP surface.

Routes exposed (mirrors M08 §16.3) :

* ``POST /v1/experiments``             — create a draft experiment
* ``POST /v1/experiments/{id}/design`` — move DRAFT → DESIGNED
* ``POST /v1/experiments/{id}/pre-register`` — seal + move to REGISTERED
* ``POST /v1/experiments/{id}/start``  — REGISTERED → RUNNING
* ``POST /v1/experiments/{id}/abort``  — RUNNING → ABORTED (manual)
* ``POST /v1/experiments/{id}/pause``  — RUNNING → PAUSED
* ``POST /v1/experiments/{id}/resume`` — PAUSED → RUNNING
* ``GET``  ``/v1/experiments/{id}``     — summary
* ``POST`` ``/v1/rollout/assign``       — ``get_variant``
* ``POST`` ``/v1/rollout/outcomes``     — record one outcome
* ``GET``  ``/healthz``                 — liveness

FastAPI is imported lazily so ``efe_rollout`` is importable even in
environments without the ``api`` extra installed. Route handlers are
registered via ``add_api_route`` (imperative) rather than decorators
so mypy --strict doesn't complain about "untyped decorator makes
function untyped" — the same pattern used by
``efe_platform/api/http_app.py``.
"""

from __future__ import annotations

from typing import Any

from efe_rollout.adapter import RolloutClient
from efe_rollout.api.schemas import (
    AbortRequest,
    AckResponse,
    AssignReply,
    AssignRequest,
    CreateExperimentRequest,
    ExperimentSummaryResponse,
    OutcomeRequest,
    VariantSchema,
)
from efe_rollout.errors import RolloutError
from efe_rollout.experiment import (
    ExperimentSpec,
    GuardrailSpec,
    Variant,
)


def build_app(client: RolloutClient) -> Any:
    try:
        from fastapi import FastAPI, Request
        from fastapi.responses import JSONResponse
    except ImportError as exc:  # pragma: no cover
        raise RuntimeError(
            "FastAPI not installed; cannot build HTTP app",
        ) from exc

    app = FastAPI(
        title="efe-rollout",
        version="0.1.0",
        openapi_url="/openapi.json",
    )

    async def _err_handler(request: Request, exc: Exception) -> Any:
        # The handler is registered only for :class:`RolloutError`
        # subclasses but FastAPI's type signature widens to
        # :class:`Exception`; we narrow defensively.
        if not isinstance(exc, RolloutError):
            return JSONResponse(
                status_code=500,
                content={"code": "unexpected", "message": str(exc)},
            )
        return JSONResponse(
            status_code=exc.http_status, content=exc.to_dict(),
        )

    app.add_exception_handler(RolloutError, _err_handler)

    async def healthz() -> dict[str, str]:
        return {"status": "ok"}

    app.add_api_route("/healthz", healthz, methods=["GET"])

    _register_experiment_routes(app, client)
    _register_rollout_routes(app, client)
    return app


def _register_experiment_routes(app: Any, client: RolloutClient) -> None:
    async def create_experiment(
        req: CreateExperimentRequest,
    ) -> ExperimentSummaryResponse:
        variants = tuple(
            Variant(
                name=v.name,
                allocation_pct=v.allocation_pct,
                payload=v.payload,
                is_control=v.is_control,
            )
            for v in req.variants
        )
        guardrails = tuple(
            GuardrailSpec(
                metric_name=g.metric_name,
                direction=g.direction,
                threshold_pct=g.threshold_pct,
                window_s=g.window_s,
                consecutive_breaches=g.consecutive_breaches,
            )
            for g in req.guardrails
        )
        spec = ExperimentSpec(
            name=req.name,
            tenant_id=req.tenant_id,
            task_id=req.task_id,
            layer=req.layer,
            kind=req.kind,
            owner=req.owner,
            hypothesis=req.hypothesis,
            primary_metric=req.primary_metric,
            variants=variants,
            guardrails=guardrails,
            alpha=req.alpha,
            power_target=req.power_target,
            minimum_detectable_effect=req.minimum_detectable_effect,
            stopping_rule=req.stopping_rule,
            planned_sample_size=req.planned_sample_size,
            planned_duration_days=req.planned_duration_days,
            traffic_allocation_bps=req.traffic_allocation_bps,
            covariates_for_cuped=tuple(req.covariates_for_cuped),
            notes=req.notes,
        )
        exp = await client.registry.create(spec)
        return _summary(exp)

    async def design(experiment_id: str) -> ExperimentSummaryResponse:
        exp = await client.registry.transition(
            experiment_id, to_state="designed", actor_role="owner",
        )
        return _summary(exp)

    async def pre_register(experiment_id: str) -> dict[str, Any]:
        prereg = await client.registry.pre_register(experiment_id)
        return {
            "experiment_id": prereg.experiment_id,
            "spec_hash": prereg.spec_hash.hex(),
            "signature": prereg.signature.hex(),
            "signing_key_id": prereg.signing_key_id,
            "created_at": prereg.created_at.isoformat(),
        }

    async def start(experiment_id: str) -> ExperimentSummaryResponse:
        exp = await client.registry.transition(
            experiment_id, to_state="running", actor_role="owner",
        )
        return _summary(exp)

    async def abort(
        experiment_id: str, req: AbortRequest,
    ) -> ExperimentSummaryResponse:
        _ = req  # reason logged elsewhere
        exp = await client.registry.transition(
            experiment_id, to_state="aborted", actor_role="admin",
        )
        return _summary(exp)

    async def pause(experiment_id: str) -> ExperimentSummaryResponse:
        exp = await client.registry.transition(
            experiment_id, to_state="paused", actor_role="owner",
        )
        return _summary(exp)

    async def resume(experiment_id: str) -> ExperimentSummaryResponse:
        exp = await client.registry.transition(
            experiment_id, to_state="running", actor_role="owner",
        )
        return _summary(exp)

    async def get_experiment(
        experiment_id: str,
    ) -> ExperimentSummaryResponse:
        exp = await client.registry.get(experiment_id)
        return _summary(exp)

    app.add_api_route(
        "/v1/experiments", create_experiment, methods=["POST"],
        response_model=ExperimentSummaryResponse,
    )
    app.add_api_route(
        "/v1/experiments/{experiment_id}/design", design,
        methods=["POST"],
        response_model=ExperimentSummaryResponse,
    )
    app.add_api_route(
        "/v1/experiments/{experiment_id}/pre-register", pre_register,
        methods=["POST"],
    )
    app.add_api_route(
        "/v1/experiments/{experiment_id}/start", start,
        methods=["POST"],
        response_model=ExperimentSummaryResponse,
    )
    app.add_api_route(
        "/v1/experiments/{experiment_id}/abort", abort,
        methods=["POST"],
        response_model=ExperimentSummaryResponse,
    )
    app.add_api_route(
        "/v1/experiments/{experiment_id}/pause", pause,
        methods=["POST"],
        response_model=ExperimentSummaryResponse,
    )
    app.add_api_route(
        "/v1/experiments/{experiment_id}/resume", resume,
        methods=["POST"],
        response_model=ExperimentSummaryResponse,
    )
    app.add_api_route(
        "/v1/experiments/{experiment_id}", get_experiment,
        methods=["GET"],
        response_model=ExperimentSummaryResponse,
    )


def _register_rollout_routes(app: Any, client: RolloutClient) -> None:
    async def assign(req: AssignRequest) -> AssignReply:
        res = await client.get_variant(
            user_id=req.user_id,
            experiment_id=req.experiment_id,
            record_exposure=req.record_exposure,
            request_id=req.request_id,
        )
        return AssignReply(
            variant_id=res.variant_id,
            variant_name=res.variant_name,
            payload=res.variant_payload,
            holdout=res.holdout,
            forced=res.forced,
        )

    async def record_outcome(req: OutcomeRequest) -> AckResponse:
        outcome = await client.record_outcome(
            experiment_id=req.experiment_id,
            user_id=req.user_id,
            metric_name=req.metric_name,
            metric_value=req.metric_value,
            variant_id=req.variant_id,
            request_id=req.request_id,
            covariate=req.covariate,
        )
        return AckResponse(ok=True, id=outcome.outcome_id)

    app.add_api_route(
        "/v1/rollout/assign", assign, methods=["POST"],
        response_model=AssignReply,
    )
    app.add_api_route(
        "/v1/rollout/outcomes", record_outcome, methods=["POST"],
        response_model=AckResponse,
    )


def _summary(exp: Any) -> ExperimentSummaryResponse:
    return ExperimentSummaryResponse(
        experiment_id=exp.experiment_id,
        name=exp.spec.name,
        tenant_id=exp.spec.tenant_id,
        task_id=exp.spec.task_id,
        layer=exp.spec.layer,
        kind=exp.spec.kind,
        state=exp.state,
        state_version=exp.state_version,
        owner=exp.spec.owner,
        primary_metric=exp.spec.primary_metric,
        alpha=exp.spec.alpha,
        power_target=exp.spec.power_target,
        variants=[
            VariantSchema(
                name=v.name,
                allocation_pct=v.allocation_pct,
                payload=v.payload,
                is_control=v.is_control,
            )
            for v in exp.spec.variants
        ],
    )


__all__ = ["build_app"]
