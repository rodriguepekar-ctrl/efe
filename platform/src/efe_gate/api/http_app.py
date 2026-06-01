"""FastAPI HTTP surface for the Evaluation Gate.

Endpoints (per M05 architecture §13):

    POST /v1/eval/submit                — submit a candidate
    GET  /v1/eval/{id}                  — fetch one
    GET  /v1/eval                       — list paginated
    POST /v1/eval/{id}:review           — dual-signoff review
    POST /v1/eval/{id}:process          — drive to decision (internal)
    GET  /v1/champions/{t}/{task}       — current champion
    POST /v1/champions/{t}/{task}:rollback — manual rollback
    GET  /v1/gates/{t}/{task}           — get rule
    PUT  /v1/gates/{t}/{task}           — upsert rule
    POST /v1/golden                     — create golden set
    GET  /v1/golden/{id}                — fetch golden set
    GET  /v1/metrics/catalog            — list known metrics
    POST /v1/admin/rollback/tick        — run one rollback daemon tick
    GET  /healthz                       — liveness
"""

from __future__ import annotations

from typing import Any

from efe_gate.api.context import GateContext, get_context
from efe_gate.api.schemas import (
    ConfirmReviewRequest,
    CreateGoldenSetRequest,
    RollbackRequest,
    SubmitEvalRequest,
    UpsertGateRequest,
)
from efe_gate.champion.audit import AuditChain
from efe_gate.champion.registry import ChampionRegistry
from efe_gate.decision.rule import PromotionRule
from efe_gate.errors import GateError
from efe_gate.golden.manager import GoldenSetManager
from efe_gate.rollback.daemon import (
    RollbackDaemon,
    StaticLiveMetricsProvider,
)
from efe_gate.services import (
    EvalService,
    GateService,
    ReviewService,
    SubmitEvalInput,
)
from efe_gate.state_machine import EvalState, StateMachine
from efe_gate.version import VERSION


def build_app(context: GateContext) -> Any:
    try:
        from fastapi import Depends, FastAPI, Query
        from fastapi.responses import JSONResponse
    except ImportError as exc:  # pragma: no cover
        raise RuntimeError(
            "FastAPI is required for efe_gate.api.http_app",
        ) from exc

    app = FastAPI(
        title="EFE Evaluation Gate — M05",
        version=VERSION,
        docs_url="/v1/docs",
        openapi_url="/v1/openapi.json",
    )

    async def _session() -> Any:
        async with context.sessionmaker() as s:
            yield s

    def _audit(s: Any = Depends(_session),
               ctx: GateContext = Depends(get_context)) -> AuditChain:
        # One audit-chain per tenant; we default to the caller-supplied
        # tenant via ``audit_for()``-style factory created on-demand.
        return AuditChain(
            s, tenant_id="_pending",
            signing=ctx.settings.signature, clock=ctx.clock,
        )

    # Audit chains are per-tenant; build on-demand inside the endpoint.
    def _audit_for(
        s: Any, ctx: GateContext, tenant_id: str,
    ) -> AuditChain:
        return AuditChain(
            s, tenant_id=tenant_id,
            signing=ctx.settings.signature, clock=ctx.clock,
        )

    def _state_machine(
        s: Any = Depends(_session),
        ctx: GateContext = Depends(get_context),
    ) -> StateMachine:
        return StateMachine(s, clock=ctx.clock)

    def _registry(
        tenant_id: str, s: Any, ctx: GateContext,
    ) -> ChampionRegistry:
        return ChampionRegistry(
            s,
            audit_chain=_audit_for(s, ctx, tenant_id),
            clock=ctx.clock,
        )

    def _eval_service(
        tenant_id: str,
        s: Any,
        sm: StateMachine,
        ctx: GateContext,
    ) -> EvalService:
        registry = _registry(tenant_id, s, ctx)
        return EvalService(
            s, state_machine=sm,
            registry=registry,
            audit_chain=_audit_for(s, ctx, tenant_id),
            scorer=ctx.scorer,
            decision_settings=ctx.settings.decision,
            stats_settings=ctx.settings.stats,
            rollback_settings=ctx.settings.rollback,
            review_settings=ctx.settings.review,
            clock=ctx.clock,
        )

    # --------------------- health + catalog --------------------------

    @app.get("/healthz")
    async def healthz() -> dict[str, str]:
        return {"status": "ok", "version": VERSION}

    @app.get("/v1/metrics/catalog")
    async def metrics_catalog() -> dict[str, Any]:
        from efe_gate.metrics.catalog import CATALOG
        return {
            "metrics": [
                {
                    "name": md.name,
                    "role": md.role.value,
                    "higher_is_better": md.higher_is_better,
                    "default_test": md.default_test,
                    "task_types": sorted(md.task_types),
                    "description": md.description,
                }
                for md in sorted(CATALOG.values(), key=lambda x: x.name)
            ],
        }

    # -------------------------- eval ---------------------------------

    @app.post("/v1/eval/submit")
    async def submit_eval(
        body: SubmitEvalRequest,
        s: Any = Depends(_session),
        sm: StateMachine = Depends(_state_machine),
        ctx: GateContext = Depends(get_context),
    ) -> dict[str, Any]:
        svc = _eval_service(body.tenant_id, s, sm, ctx)
        view = await svc.submit(SubmitEvalInput(
            tenant_id=body.tenant_id,
            task_id=body.task_id,
            task_type=body.task_type,
            candidate_run_id=body.candidate_run_id,
            candidate_artifact_uri=body.candidate_artifact_uri,
            submitted_by=body.submitted_by,
            golden_set_id=body.golden_set_id,
            metric_names=tuple(body.metric_names),
            tags=dict(body.tags),
        ))
        await s.commit()
        return view.as_dict()

    @app.get("/v1/eval/{eval_id}")
    async def get_eval(
        eval_id: str,
        tenant_id: str | None = Query(default=None),
        s: Any = Depends(_session),
        sm: StateMachine = Depends(_state_machine),
        ctx: GateContext = Depends(get_context),
    ) -> dict[str, Any]:
        svc = _eval_service(tenant_id or "_any", s, sm, ctx)
        view = await svc.get(eval_id, tenant_id=tenant_id)
        return view.as_dict()

    @app.get("/v1/eval")
    async def list_evals(
        tenant_id: str = Query(...),
        task_id: str | None = Query(default=None),
        state: str | None = Query(default=None),
        page_size: int = Query(default=50, ge=1, le=500),
        page_token: str | None = Query(default=None),
        s: Any = Depends(_session),
        sm: StateMachine = Depends(_state_machine),
        ctx: GateContext = Depends(get_context),
    ) -> dict[str, Any]:
        svc = _eval_service(tenant_id, s, sm, ctx)
        state_enum = EvalState(state) if state else None
        rows, next_token = await svc.list(
            tenant_id=tenant_id,
            task_id=task_id,
            state=state_enum,
            page_size=page_size,
            page_token=page_token,
        )
        return {
            "items": [r.as_dict() for r in rows],
            "next_token": next_token,
        }

    @app.post("/v1/eval/{eval_id}:process")
    async def process_eval(
        eval_id: str,
        tenant_id: str | None = Query(default=None),
        s: Any = Depends(_session),
        sm: StateMachine = Depends(_state_machine),
        ctx: GateContext = Depends(get_context),
    ) -> dict[str, Any]:
        svc = _eval_service(tenant_id or "_any", s, sm, ctx)
        view = await svc.process(eval_id=eval_id)
        await s.commit()
        return view.as_dict()

    @app.post("/v1/eval/{eval_id}:review")
    async def confirm_review(
        eval_id: str,
        body: ConfirmReviewRequest,
        tenant_id: str | None = Query(default=None),
        s: Any = Depends(_session),
        sm: StateMachine = Depends(_state_machine),
        ctx: GateContext = Depends(get_context),
    ) -> dict[str, Any]:
        svc = _eval_service(tenant_id or "_any", s, sm, ctx)
        view = await svc.confirm_review(
            eval_id=eval_id,
            reviewer_id=body.reviewer_id,
            approved=body.approved,
            reason=body.reason,
            tenant_id=tenant_id,
        )
        await s.commit()
        return view.as_dict()

    # -------------------- champions + rollback -----------------------

    @app.get("/v1/champions/{tenant_id}/{task_id}")
    async def get_champion(
        tenant_id: str, task_id: str,
        s: Any = Depends(_session),
        ctx: GateContext = Depends(get_context),
    ) -> dict[str, Any]:
        registry = _registry(tenant_id, s, ctx)
        champ = await registry.get(tenant_id, task_id)
        return champ.as_dict()

    @app.post("/v1/champions/{tenant_id}/{task_id}:rollback")
    async def rollback_champion(
        tenant_id: str, task_id: str,
        body: RollbackRequest,
        s: Any = Depends(_session),
        ctx: GateContext = Depends(get_context),
    ) -> dict[str, Any]:
        registry = _registry(tenant_id, s, ctx)
        champ = await registry.rollback(
            tenant_id=tenant_id, task_id=task_id,
            actor=body.actor, reason=body.reason,
        )
        await s.commit()
        return champ.as_dict()

    # ---------------------------- gates ------------------------------

    @app.get("/v1/gates/{tenant_id}/{task_id}")
    async def get_gate(
        tenant_id: str, task_id: str,
        s: Any = Depends(_session),
        ctx: GateContext = Depends(get_context),
    ) -> dict[str, Any]:
        gsvc = GateService(s, clock=ctx.clock)
        info = await gsvc.get(tenant_id, task_id)
        return info.as_dict()

    @app.put("/v1/gates/{tenant_id}/{task_id}")
    async def upsert_gate(
        tenant_id: str, task_id: str,
        body: UpsertGateRequest,
        s: Any = Depends(_session),
        ctx: GateContext = Depends(get_context),
    ) -> dict[str, Any]:
        gsvc = GateService(s, clock=ctx.clock)
        rule = PromotionRule.from_dict(body.rule)
        info = await gsvc.upsert(
            tenant_id=tenant_id, task_id=task_id,
            rule=rule, updated_by=body.updated_by,
        )
        await s.commit()
        # Record audit entry.
        chain = _audit_for(s, ctx, tenant_id)
        from efe_gate.champion.audit import AuditKind
        await chain.record(
            kind=AuditKind.GATE_UPDATED,
            actor=body.updated_by, subject_id=None,
            payload={"task_id": task_id, "rule": rule.as_dict()},
        )
        await s.commit()
        return info.as_dict()

    # ------------------------- golden --------------------------------

    @app.post("/v1/golden")
    async def create_golden(
        body: CreateGoldenSetRequest,
        s: Any = Depends(_session),
        ctx: GateContext = Depends(get_context),
    ) -> dict[str, Any]:
        mgr = GoldenSetManager(s, clock=ctx.clock)
        info = await mgr.create(
            tenant_id=body.tenant_id,
            task_id=body.task_id,
            snapshot_uri=body.snapshot_uri,
            content_hash=body.content_hash,
            n_samples=body.n_samples,
            label_balance=dict(body.label_balance),
            fairness_attrs=dict(body.fairness_attrs),
            created_by=body.created_by,
        )
        await s.commit()
        return {
            "id": info.id,
            "tenant_id": info.tenant_id,
            "task_id": info.task_id,
            "content_hash": info.content_hash,
            "n_samples": info.n_samples,
            "locked_at": info.locked_at.isoformat(),
            "deprecated": info.deprecated,
        }

    @app.get("/v1/golden/{gs_id}")
    async def get_golden(
        gs_id: str,
        s: Any = Depends(_session),
        ctx: GateContext = Depends(get_context),
    ) -> dict[str, Any]:
        mgr = GoldenSetManager(s, clock=ctx.clock)
        info = await mgr.get(gs_id)
        return {
            "id": info.id,
            "tenant_id": info.tenant_id,
            "task_id": info.task_id,
            "content_hash": info.content_hash,
            "n_samples": info.n_samples,
            "label_balance": dict(info.label_balance),
            "fairness_attrs": dict(info.fairness_attrs),
            "locked_at": info.locked_at.isoformat(),
            "deprecated": info.deprecated,
            "created_by": info.created_by,
        }

    # -------------------------- reviews ------------------------------

    @app.get("/v1/reviews")
    async def list_reviews(
        tenant_id: str = Query(...),
        s: Any = Depends(_session),
    ) -> dict[str, Any]:
        rsvc = ReviewService(s)
        rows = await rsvc.list_pending(tenant_id)
        return {"items": [r.as_dict() for r in rows]}

    # ---------------------- admin rollback tick ----------------------

    @app.post("/v1/admin/rollback/tick")
    async def admin_tick(
        s: Any = Depends(_session),
        ctx: GateContext = Depends(get_context),
    ) -> dict[str, Any]:
        # A minimal tick — uses a static live-metrics provider that
        # returns empty arrays. Real deployments wire a live provider.
        lp = StaticLiveMetricsProvider()
        # We need a registry keyed to the tenant of each monitor,
        # but AuditChain requires a tenant_id. We defer the common
        # case where one replica runs per-cluster and accept that
        # rollbacks record the chain under "_admin". In production
        # the daemon uses a proper per-tenant chain.
        registry = ChampionRegistry(
            s,
            audit_chain=_audit_for(s, ctx, "_admin"),
            clock=ctx.clock,
        )
        daemon = RollbackDaemon(
            session=s,
            registry=registry,
            live_metrics=lp,
            settings=ctx.settings.rollback,
            clock=ctx.clock,
        )
        report = await daemon.tick()
        await s.commit()
        return {
            "scanned": report.scanned,
            "rolled_back": list(report.rolled_back),
            "expired": list(report.expired),
            "still_watching": list(report.still_watching),
            "skipped": list(report.skipped),
        }

    # ------------------------- error mapping -------------------------

    @app.exception_handler(GateError)
    async def _on_error(_req: Any, exc: GateError) -> Any:
        return JSONResponse(
            status_code=exc.http_status, content=exc.to_dict(),
        )

    _ = _audit  # silence unused helper warning
    return app


__all__ = ["build_app"]
