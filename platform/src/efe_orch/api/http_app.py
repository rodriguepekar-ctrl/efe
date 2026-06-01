"""FastAPI app — HTTP surface for M03.

Endpoints:

    POST /v1/orch/runs                        — submit a new run
    GET  /v1/orch/runs/:run_id                — fetch one run
    GET  /v1/orch/tenants/:tenant/runs        — list runs (paginated)
    POST /v1/orch/runs/:run_id:dispatch       — force dispatch (admin)
    POST /v1/orch/runs/:run_id:cancel         — cancel
    POST /v1/orch/workers/heartbeat           — worker heartbeat
    POST /v1/orch/workers/checkpoint          — worker checkpoint
    POST /v1/orch/workers/result              — worker result
    POST /v1/orch/workers/error               — worker error
    POST /v1/orch/admin/reap                  — run one reaper cycle
    GET  /healthz                             — liveness probe
    GET  /v1/orch/pools                       — list configured pools

FastAPI is imported inside :func:`build_app` so installation without
the ``api`` extra does not fail at module-load time.
"""

from __future__ import annotations

from typing import Any

from efe_orch.api.context import OrchContext, get_context
from efe_orch.api.schemas import (
    CancelRunRequest,
    CheckpointRequest,
    ErrorRequest,
    HeartbeatRequest,
    ResultRequest,
    SubmitRunRequest,
)
from efe_orch.dispatcher import KubernetesClient
from efe_orch.errors import OrchError
from efe_orch.heartbeat.reaper import HeartbeatReaper
from efe_orch.heartbeat.recorder import HeartbeatInput
from efe_orch.scheduler import SchedulerService
from efe_orch.services import (
    CheckpointReport,
    ErrorReport,
    ListRunsFilter,
    ResultReport,
    RunService,
    SubmitRunInput,
    WorkerService,
)
from efe_orch.state_machine.states import RunState
from efe_orch.state_machine.transitions import StateMachine
from efe_orch.version import VERSION


def build_app(context: OrchContext) -> Any:
    """Return a FastAPI app bound to ``context``."""
    try:
        from fastapi import Depends, FastAPI, Query
        from fastapi.responses import JSONResponse
    except ImportError as exc:  # pragma: no cover — optional dep
        raise RuntimeError(
            "FastAPI is required for efe_orch.api.http_app",
        ) from exc

    app = FastAPI(
        title="EFE Training Orchestrator — M03",
        version=VERSION,
        docs_url="/v1/docs",
        openapi_url="/v1/openapi.json",
    )

    # -------------------- dependency wiring (per-request) --------------

    async def _session() -> Any:
        async with context.sessionmaker() as s:
            yield s

    def _scheduler(
        s: Any = Depends(_session), ctx: OrchContext = Depends(get_context),
    ) -> SchedulerService:
        return SchedulerService(
            s,
            scheduler_settings=ctx.settings.scheduler,
            quota_settings=ctx.settings.quota,
        )

    def _sm(
        s: Any = Depends(_session), ctx: OrchContext = Depends(get_context),
    ) -> StateMachine:
        return StateMachine(
            s, lock_backend=ctx.lock_backend, clock=ctx.clock,
        )

    def _k8s(ctx: OrchContext = Depends(get_context)) -> KubernetesClient:
        return ctx.k8s_client

    def _run_svc(
        s: Any = Depends(_session),
        sched: SchedulerService = Depends(_scheduler),
        sm: StateMachine = Depends(_sm),
        k8s: KubernetesClient = Depends(_k8s),
        ctx: OrchContext = Depends(get_context),
    ) -> RunService:
        return RunService(
            s,
            scheduler=sched,
            state_machine=sm,
            k8s_client=k8s,
            k8s_settings=ctx.settings.k8s,
            retry_settings=ctx.settings.retry,
            clock=ctx.clock,
        )

    def _worker_svc(
        s: Any = Depends(_session),
        sched: SchedulerService = Depends(_scheduler),
        sm: StateMachine = Depends(_sm),
        run_svc: RunService = Depends(_run_svc),
        ctx: OrchContext = Depends(get_context),
    ) -> WorkerService:
        return WorkerService(
            s,
            state_machine=sm,
            run_service=run_svc,
            scheduler=sched,
            clock=ctx.clock,
        )

    # -------------------------- health --------------------------------

    @app.get("/healthz")
    async def healthz() -> dict[str, str]:
        return {"status": "ok", "version": VERSION}

    @app.get("/v1/orch/pools")
    async def list_pools(
        ctx: OrchContext = Depends(get_context),
    ) -> dict[str, Any]:
        return {
            "pools": [
                {
                    "name": p.name,
                    "label": p.label,
                    "max_concurrent_runs": p.max_concurrent_runs,
                    "gpu": p.gpu,
                    "gpu_count": p.gpu_count,
                    "cost_score": p.cost_score,
                    "max_dataset_rows": p.max_dataset_rows,
                }
                for p in ctx.settings.scheduler.pools
            ],
        }

    # --------------------------- runs ---------------------------------

    @app.post("/v1/orch/runs")
    async def submit_run(
        body: SubmitRunRequest,
        svc: RunService = Depends(_run_svc),
        s: Any = Depends(_session),
    ) -> dict[str, Any]:
        inp = SubmitRunInput(**body.model_dump())
        view = await svc.submit(inp)
        await s.commit()
        return _serialise_view(view)

    @app.get("/v1/orch/runs/{run_id}")
    async def get_run(
        run_id: str,
        tenant_id: str | None = Query(default=None),
        svc: RunService = Depends(_run_svc),
    ) -> dict[str, Any]:
        view = await svc.get(run_id=run_id, tenant_id=tenant_id)
        return _serialise_view(view)

    @app.get("/v1/orch/tenants/{tenant}/runs")
    async def list_runs(
        tenant: str,
        task_id: str | None = Query(default=None),
        state: str | None = Query(default=None),
        submitted_by: str | None = Query(default=None),
        page_size: int = Query(default=50, ge=1, le=500),
        page_token: str | None = Query(default=None),
        svc: RunService = Depends(_run_svc),
    ) -> dict[str, Any]:
        state_enum = RunState(state) if state else None
        filt = ListRunsFilter(
            tenant_id=tenant, task_id=task_id, state=state_enum,
            submitted_by=submitted_by,
            page_size=page_size, page_token=page_token,
        )
        rows, next_token = await svc.list(filt)
        return {
            "items": [_serialise_view(r) for r in rows],
            "next_token": next_token,
        }

    @app.post("/v1/orch/runs/{run_id}:dispatch")
    async def dispatch_run(
        run_id: str,
        tenant_id: str | None = Query(default=None),
        svc: RunService = Depends(_run_svc),
        s: Any = Depends(_session),
    ) -> dict[str, Any]:
        view = await svc.dispatch(run_id=run_id, tenant_id=tenant_id)
        await s.commit()
        return _serialise_view(view)

    @app.post("/v1/orch/runs/{run_id}:cancel")
    async def cancel_run(
        run_id: str,
        body: CancelRunRequest,
        tenant_id: str | None = Query(default=None),
        svc: RunService = Depends(_run_svc),
        s: Any = Depends(_session),
    ) -> dict[str, Any]:
        view = await svc.cancel(
            run_id=run_id,
            actor=body.actor, reason=body.reason,
            tenant_id=tenant_id,
        )
        await s.commit()
        return _serialise_view(view)

    # -------------------------- worker --------------------------------

    @app.post("/v1/orch/workers/heartbeat")
    async def hb(
        body: HeartbeatRequest,
        svc: WorkerService = Depends(_worker_svc),
        s: Any = Depends(_session),
    ) -> dict[str, Any]:
        view = await svc.heartbeat(HeartbeatInput(**body.model_dump()))
        await s.commit()
        return _serialise_view(view)

    @app.post("/v1/orch/workers/checkpoint")
    async def ckpt(
        body: CheckpointRequest,
        svc: WorkerService = Depends(_worker_svc),
        s: Any = Depends(_session),
    ) -> dict[str, str]:
        cid = await svc.report_checkpoint(
            CheckpointReport(**body.model_dump()),
        )
        await s.commit()
        return {"checkpoint_id": cid}

    @app.post("/v1/orch/workers/result")
    async def result(
        body: ResultRequest,
        svc: WorkerService = Depends(_worker_svc),
        s: Any = Depends(_session),
    ) -> dict[str, Any]:
        view = await svc.report_result(ResultReport(**body.model_dump()))
        await s.commit()
        return _serialise_view(view)

    @app.post("/v1/orch/workers/error")
    async def err(
        body: ErrorRequest,
        svc: WorkerService = Depends(_worker_svc),
        s: Any = Depends(_session),
    ) -> dict[str, Any]:
        view = await svc.report_error(ErrorReport(**body.model_dump()))
        await s.commit()
        return _serialise_view(view)

    # ---------------------- per-run diagnostics -----------------------

    @app.get("/v1/orch/runs/{run_id}/transitions")
    async def list_transitions(
        run_id: str,
        sm: StateMachine = Depends(_sm),
    ) -> dict[str, Any]:
        """Return the full audit trail of state transitions for a run."""
        rows = await sm.history(run_id)
        return {
            "items": [
                {
                    "id": r.id,
                    "from_state": r.from_state,
                    "to_state": r.to_state,
                    "actor": r.actor,
                    "reason": r.reason,
                    "payload": dict(r.payload or {}),
                    "created_at": r.created_at.isoformat(),
                }
                for r in rows
            ],
        }

    @app.get("/v1/orch/runs/{run_id}/heartbeats")
    async def list_heartbeats(
        run_id: str,
        limit: int = Query(default=50, ge=1, le=500),
        s: Any = Depends(_session),
    ) -> dict[str, Any]:
        """Return the most recent heartbeats for a run (newest first)."""
        from sqlalchemy import select

        from efe_orch.db.models import WorkerHeartbeat
        stmt = (
            select(WorkerHeartbeat)
            .where(WorkerHeartbeat.run_id == run_id)
            .order_by(WorkerHeartbeat.observed_at.desc())
            .limit(limit)
        )
        rows = list((await s.execute(stmt)).scalars())
        return {
            "items": [
                {
                    "id": hb.id,
                    "worker_id": hb.worker_id,
                    "beat_seq": hb.beat_seq,
                    "observed_at": hb.observed_at.isoformat(),
                    "state_hint": hb.state_hint,
                    "progress_ratio": hb.progress_ratio,
                    "step": hb.step,
                    "extra": dict(hb.extra or {}),
                }
                for hb in rows
            ],
        }

    @app.get("/v1/orch/runs/{run_id}/checkpoints")
    async def list_checkpoints(
        run_id: str,
        s: Any = Depends(_session),
    ) -> dict[str, Any]:
        """Return all known checkpoints for a run, ordered by step."""
        from sqlalchemy import select

        from efe_orch.db.models import RunCheckpoint
        stmt = (
            select(RunCheckpoint)
            .where(RunCheckpoint.run_id == run_id)
            .order_by(RunCheckpoint.step.asc())
        )
        rows = list((await s.execute(stmt)).scalars())
        return {
            "items": [
                {
                    "id": c.id,
                    "step": c.step,
                    "blob_uri": c.blob_uri,
                    "metrics": dict(c.metrics or {}),
                    "size_bytes": c.size_bytes,
                    "is_best": c.is_best,
                    "created_at": c.created_at.isoformat(),
                }
                for c in rows
            ],
        }

    # ------------------------ admin / ops -----------------------------

    @app.post("/v1/orch/admin/reap")
    async def run_reaper(
        limit: int = Query(default=100, ge=1, le=10_000),
        s: Any = Depends(_session),
        sm: StateMachine = Depends(_sm),
        sched: SchedulerService = Depends(_scheduler),
        ctx: OrchContext = Depends(get_context),
    ) -> dict[str, Any]:
        reaper = HeartbeatReaper(
            s, state_machine=sm,
            settings=ctx.settings.heartbeat,
            scheduler=sched,
            clock=ctx.clock,
        )
        report = await reaper.scan(limit=limit)
        await s.commit()
        return {
            "scanned": report.scanned,
            "reaped": list(report.reaped),
            "skipped": list(report.skipped),
            "at": report.at.isoformat() if report.at else None,
        }

    @app.get("/v1/orch/admin/tenants/{tenant}/quota")
    async def get_quota(
        tenant: str,
        sched: SchedulerService = Depends(_scheduler),
    ) -> dict[str, Any]:
        """Return the live quota counters for one tenant."""
        # Read-only: ``_get_or_create`` creates a defaults-row if absent.
        quota = await sched.quota._get_or_create(tenant)
        return {
            "tenant_id": quota.tenant_id,
            "max_concurrent_runs": quota.max_concurrent_runs,
            "max_monthly_runs": quota.max_monthly_runs,
            "max_monthly_gpu_seconds": quota.max_monthly_gpu_seconds,
            "allowed_pools": list(quota.allowed_pools or []),
            "current_concurrent_runs": quota.current_concurrent_runs,
            "current_monthly_runs": quota.current_monthly_runs,
            "current_monthly_gpu_seconds": quota.current_monthly_gpu_seconds,
            "updated_at": quota.updated_at.isoformat(),
        }

    @app.post("/v1/orch/admin/tenants/{tenant}/quota:reset")
    async def reset_quota(
        tenant: str,
        sched: SchedulerService = Depends(_scheduler),
        s: Any = Depends(_session),
    ) -> dict[str, Any]:
        """Zero the tenant's monthly counters.

        Use sparingly — intended for monthly rollovers or SRE remediation
        after a quota accounting incident. Does NOT reset the concurrency
        counter (which reflects in-flight runs; resetting it would
        double-book).
        """
        quota = await sched.quota._get_or_create(tenant)
        quota.current_monthly_runs = 0
        quota.current_monthly_gpu_seconds = 0
        await s.commit()
        return {"tenant_id": tenant, "reset": True}

    # ----------------------- error mapping ----------------------------

    @app.exception_handler(OrchError)
    async def _on_orch_error(_req: Any, exc: OrchError) -> Any:
        return JSONResponse(
            status_code=exc.http_status, content=exc.to_dict(),
        )

    return app


# ---------------------------------------------------------------------------
#  helpers
# ---------------------------------------------------------------------------


def _serialise_view(view: Any) -> dict[str, Any]:
    d = {
        "id": view.id,
        "tenant_id": view.tenant_id,
        "task_id": view.task_id,
        "submitted_by": view.submitted_by,
        "snapshot_id": view.snapshot_id,
        "state": view.state.value,
        "state_version": view.state_version,
        "priority": view.priority,
        "pool": view.pool,
        "worker_id": view.worker_id,
        "attempt": view.attempt,
        "retry_count": view.retry_count,
        "last_error_code": view.last_error_code,
        "last_error_message": view.last_error_message,
        "created_at": view.created_at.isoformat(),
        "updated_at": view.updated_at.isoformat(),
        "started_at": view.started_at.isoformat() if view.started_at else None,
        "finished_at": view.finished_at.isoformat() if view.finished_at else None,
        "result_artifact_url": view.result_artifact_url,
        "result_metrics": view.result_metrics,
        "hyperparameters": view.hyperparameters,
        "tags": view.tags,
    }
    return d


__all__ = ["build_app"]
