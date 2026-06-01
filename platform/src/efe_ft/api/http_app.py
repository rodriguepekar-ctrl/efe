"""FastAPI app — HTTP surface for M01 + M02.

Endpoints:
    POST /v1/ft/ingest/:tenant/:task/sample      — single sample ingest
    POST /v1/ft/ingest/:tenant/:task/batch       — batch of samples
    GET  /v1/ft/samples/:tenant/:task            — list samples (paginated)
    POST /v1/ft/snapshots/:tenant/:task          — materialise snapshot
    GET  /v1/ft/snapshots/:snapshot_id           — snapshot details
    DELETE /v1/ft/samples/:tenant/:sample_id     — delete sample (GDPR)
    POST /v1/ft/labeling/sessions                — start labeling session
    POST /v1/ft/labeling/labels                  — submit a label
    POST /v1/ft/labeling/flags                   — flag a sample
    GET  /v1/ft/labeling/escalations/:tenant     — expert queue
    POST /v1/ft/labeling/escalations/resolve     — expert decision
    GET  /healthz                                — liveness
    GET  /metrics                                — Prometheus text
"""

from __future__ import annotations

from typing import Any

from efe_ft.api.dependencies import FtContext, get_context
from efe_ft.api.feature_store import FeatureStoreFacade
from efe_ft.api.http_schemas import (
    FlagRequest,
    IngestBatchRequest,
    ResolveRequest,
    SnapshotRequest,
    StartSessionRequest,
    SubmitLabelRequest,
)
from efe_ft.api.labeling_service import LabelingFacade
from efe_ft.errors import FtError
from efe_ft.ingest.schemas import SampleIngestPayload
from efe_ft.snapshots import Partitioning
from efe_ft.version import VERSION


def build_app(context: FtContext) -> Any:
    """Build the FastAPI app bound to ``context``."""
    try:
        from fastapi import Depends, FastAPI, HTTPException, Query
        from fastapi.responses import JSONResponse
    except ImportError as exc:  # pragma: no cover — optional dep
        raise RuntimeError("FastAPI is required for efe_ft.api.http_app") from exc

    app = FastAPI(
        title="EFE Fine-Tuning — M01+M02",
        version=VERSION,
        docs_url="/v1/docs",
        openapi_url="/v1/openapi.json",
    )

    def _fs(ctx: FtContext = Depends(get_context)) -> FeatureStoreFacade:
        return FeatureStoreFacade(
            ctx.sessionmaker,
            deduper=ctx.deduper,
            pii_policy=ctx.settings.ingest.pii_policy_default,
            balance_skew_max=ctx.settings.ingest.balance_skew_max,
        )

    def _lbl(ctx: FtContext = Depends(get_context)) -> LabelingFacade:
        return LabelingFacade(
            ctx.sessionmaker,
            session_store=ctx.session_store,
            class_set=ctx.class_set,
            session_ttl_s=ctx.settings.labeling.session_ttl_s,
        )

    # --------------------------------------------------- health
    @app.get("/healthz")
    async def healthz() -> dict[str, str]:
        return {"status": "ok", "version": VERSION}

    # --------------------------------------------------- ingest
    @app.post("/v1/ft/ingest/{tenant}/{task}/sample")
    async def ingest_sample(
        tenant: str, task: str, payload: SampleIngestPayload,
        fs: FeatureStoreFacade = Depends(_fs),
    ) -> dict[str, Any]:
        if payload.tenant_id != tenant:
            raise HTTPException(400, "payload.tenant_id mismatch")
        if payload.task_id != task:
            raise HTTPException(400, "payload.task_id mismatch")
        outcome = await fs.ingest_sample(payload)
        return outcome.to_dict()

    @app.post("/v1/ft/ingest/{tenant}/{task}/batch")
    async def ingest_batch(
        tenant: str, task: str, body: IngestBatchRequest,
        fs: FeatureStoreFacade = Depends(_fs),
    ) -> dict[str, Any]:
        for p in body.items:
            if p.tenant_id != tenant or p.task_id != task:
                raise HTTPException(400, "tenant/task mismatch in one item")
        outcome = await fs.ingest_batch(body.items)
        return outcome.to_dict()

    # --------------------------------------------------- listing
    @app.get("/v1/ft/samples/{tenant}/{task}")
    async def list_samples(
        tenant: str, task: str,
        status: str | None = Query(default=None),
        page_size: int = Query(default=100, ge=1, le=1000),
        page_token: str | None = Query(default=None),
        fs: FeatureStoreFacade = Depends(_fs),
    ) -> dict[str, Any]:
        rows, next_token = await fs.list_samples(
            tenant_id=tenant, task_id=task,
            status=status, page_size=page_size, page_token=page_token,
        )
        return {
            "items": [
                {
                    "id": r.id, "text": r.text, "label": r.label,
                    "status": r.status, "tenant_id": r.tenant_id,
                    "task_id": r.task_id,
                }
                for r in rows
            ],
            "next_token": next_token,
        }

    # --------------------------------------------------- snapshots
    @app.post("/v1/ft/snapshots/{tenant}/{task}")
    async def materialise_snapshot(
        tenant: str, task: str, body: SnapshotRequest,
        fs: FeatureStoreFacade = Depends(_fs),
    ) -> dict[str, Any]:
        partitioning = None
        if body.train or body.val or body.holdout:
            partitioning = Partitioning(
                train=body.train or 0,
                val=body.val or 0,
                holdout=body.holdout or 0,
                seed=body.seed,
            )
        snap = await fs.materialise_snapshot(
            tenant_id=tenant, task_id=task,
            actor_id=body.actor_id,
            status_filter=body.status_filter,
            partitioning=partitioning,
            min_samples=body.min_samples,
        )
        return {
            "id": snap.id,
            "tenant_id": snap.tenant_id,
            "task_id": snap.task_id,
            "n_samples": snap.n_samples,
            "content_hash": snap.content_hash.hex(),
            "label_balance": dict(snap.label_balance),
        }

    @app.get("/v1/ft/snapshots/{snapshot_id}")
    async def get_snapshot(
        snapshot_id: str, fs: FeatureStoreFacade = Depends(_fs),
    ) -> dict[str, Any]:
        snap = await fs.get_snapshot(snapshot_id=snapshot_id)
        if snap is None:
            raise HTTPException(404, "snapshot not found")
        return {
            "id": snap.id,
            "tenant_id": snap.tenant_id,
            "task_id": snap.task_id,
            "n_samples": snap.n_samples,
            "content_hash": snap.content_hash.hex(),
            "label_balance": dict(snap.label_balance),
            "created_at": snap.created_at.isoformat(),
        }

    # --------------------------------------------------- delete
    @app.delete("/v1/ft/samples/{tenant}/{sample_id}")
    async def delete_sample(
        tenant: str, sample_id: str,
        reason: str = Query(default="gdpr_user_request"),
        actor_id: str = Query(default="system"),
        fs: FeatureStoreFacade = Depends(_fs),
    ) -> dict[str, bool]:
        ok = await fs.delete_sample(
            tenant_id=tenant, sample_id=sample_id,
            actor_id=actor_id, reason=reason,
        )
        return {"deleted": ok}

    # --------------------------------------------------- labeling
    @app.post("/v1/ft/labeling/sessions")
    async def start_session(
        body: StartSessionRequest,
        lbl: LabelingFacade = Depends(_lbl),
    ) -> dict[str, str]:
        session = await lbl.start_session(
            annotator_id=body.annotator_id,
            tenant_id=body.tenant_id, task_id=body.task_id,
        )
        return {
            "session_id": session.session_id,
            "annotator_id": session.annotator_id,
            "tenant_id": session.tenant_id,
            "task_id": session.task_id,
        }

    @app.post("/v1/ft/labeling/labels")
    async def submit_label(
        body: SubmitLabelRequest,
        lbl: LabelingFacade = Depends(_lbl),
    ) -> dict[str, Any]:
        result = await lbl.submit_label(
            tenant_id=body.tenant_id, session_id=body.session_id,
            sample_id=body.sample_id, label=body.label,
            time_ms=body.time_ms,
        )
        return {
            "accepted": result.accepted,
            "triggered_escalation": result.triggered_escalation,
            "escalation_ref": result.escalation_ref,
            "labels_submitted": result.labels_submitted,
            "next_sample_id": result.next_sample_id,
        }

    @app.post("/v1/ft/labeling/flags")
    async def flag_sample(
        body: FlagRequest, lbl: LabelingFacade = Depends(_lbl),
    ) -> dict[str, Any]:
        result = await lbl.flag_sample(
            tenant_id=body.tenant_id, session_id=body.session_id,
            sample_id=body.sample_id, reason=body.reason,
        )
        return {
            "accepted": result.accepted,
            "next_sample_id": result.next_sample_id,
        }

    @app.get("/v1/ft/labeling/escalations/{tenant}")
    async def escalation_queue(
        tenant: str, task: str, limit: int = Query(default=50, ge=1, le=500),
        lbl: LabelingFacade = Depends(_lbl),
    ) -> dict[str, Any]:
        queue = await lbl.escalation_queue(
            tenant_id=tenant, task_id=task, limit=limit,
        )
        return {"items": [e.to_dict() for e in queue]}

    @app.post("/v1/ft/labeling/escalations/resolve")
    async def resolve_escalation(
        body: ResolveRequest, lbl: LabelingFacade = Depends(_lbl),
    ) -> dict[str, str]:
        ref = await lbl.resolve_escalation(
            tenant_id=body.tenant_id, sample_id=body.sample_id,
            expert_id=body.expert_id, label=body.label,
        )
        return {"history_id": ref}

    # --------------------------------------------------- error mapper
    @app.exception_handler(FtError)
    async def _on_ft_error(_req: Any, exc: FtError) -> Any:
        return JSONResponse(
            status_code=exc.http_status, content=exc.to_dict(),
        )

    return app


__all__ = ["build_app"]
