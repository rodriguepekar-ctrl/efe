"""Labeler UI — FastAPI + Jinja2 + HTMX. Server-rendered, zero SPA.

The UI is driven entirely by HTMX ``hx-get``/``hx-post``; all
templates render server-side. Keyboard shortcuts are implemented in a
tiny inline script (< 50 lines).

Routes:
    GET  /                                    — tenant/task picker
    GET  /labeler/{tenant}/{task}             — main labeling page (full HTML)
    GET  /labeler/{tenant}/{task}/next        — htmx: returns next sample card
    POST /labeler/{tenant}/{task}/label       — htmx: submit & return next
    POST /labeler/{tenant}/{task}/flag        — htmx: flag & return next
    GET  /labeler/{tenant}/{task}/stats       — htmx: stats partial
    GET  /labeler/expert/{tenant}/{task}      — expert adjudication queue
    POST /labeler/expert/resolve              — expert decision
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

from efe_ft.api.dependencies import FtContext, get_context
from efe_ft.api.labeling_service import LabelingFacade
from efe_ft.version import VERSION

_TEMPLATES_DIR = Path(__file__).parent / "templates"


def build_ui_app(context: FtContext, *, class_set: list[str]) -> Any:
    try:
        from fastapi import Depends, FastAPI, Form, Query, Request
        from fastapi.responses import HTMLResponse
        from fastapi.templating import Jinja2Templates
    except ImportError as exc:  # pragma: no cover
        raise RuntimeError("FastAPI is required for labeler UI") from exc

    tpl = Jinja2Templates(directory=str(_TEMPLATES_DIR))

    app = FastAPI(title="EFE Labeler", version=VERSION)

    def _lbl(ctx: FtContext = Depends(get_context)) -> LabelingFacade:
        return LabelingFacade(
            ctx.sessionmaker, session_store=ctx.session_store,
            class_set=ctx.class_set,
            session_ttl_s=ctx.settings.labeling.session_ttl_s,
        )

    @app.get("/", response_class=HTMLResponse)
    async def index(request: Request) -> Any:
        return tpl.TemplateResponse(request, "index.html", {})

    @app.get("/labeler/{tenant}/{task}", response_class=HTMLResponse)
    async def labeler_page(
        request: Request, tenant: str, task: str,
        annotator_id: str = Query(default="anon"),
        lbl: LabelingFacade = Depends(_lbl),
    ) -> Any:
        session = await lbl.start_session(
            annotator_id=annotator_id, tenant_id=tenant, task_id=task,
        )
        return tpl.TemplateResponse(request, "labeler.html", {
            "session_id": session.session_id,
            "annotator_id": annotator_id,
            "tenant": tenant, "task": task,
            "class_set": class_set,
            "current_sample": None,
            "next_sample_id": None,
        })

    @app.get("/labeler/{tenant}/{task}/stats", response_class=HTMLResponse)
    async def stats(
        request: Request, tenant: str, task: str,
        annotator_id: str = Query(..., min_length=1),
        lbl: LabelingFacade = Depends(_lbl),
    ) -> Any:
        s = await lbl.annotator_stats(
            tenant_id=tenant, annotator_id=annotator_id,
        )
        return tpl.TemplateResponse(request, "_stats.html", {"stats": s})

    @app.post("/labeler/{tenant}/{task}/label", response_class=HTMLResponse)
    async def submit_label(
        request: Request, tenant: str, task: str,
        session_id: str = Form(...),
        sample_id: str = Form(...),
        label: str = Form(...),
        time_ms: int = Form(default=0),
        lbl: LabelingFacade = Depends(_lbl),
    ) -> Any:
        result = await lbl.submit_label(
            tenant_id=tenant, session_id=session_id,
            sample_id=sample_id, label=label, time_ms=time_ms,
        )
        return tpl.TemplateResponse(request, "_sample_card.html", {
            "session_id": session_id, "tenant": tenant, "task": task,
            "next_sample_id": result.next_sample_id,
            "class_set": class_set,
            "escalated": result.triggered_escalation,
            "escalation_ref": result.escalation_ref,
        })

    @app.post("/labeler/{tenant}/{task}/flag", response_class=HTMLResponse)
    async def flag(
        request: Request, tenant: str, task: str,
        session_id: str = Form(...),
        sample_id: str = Form(...),
        reason: str = Form(...),
        lbl: LabelingFacade = Depends(_lbl),
    ) -> Any:
        result = await lbl.flag_sample(
            tenant_id=tenant, session_id=session_id,
            sample_id=sample_id, reason=reason,  # type: ignore[arg-type]
        )
        return tpl.TemplateResponse(request, "_sample_card.html", {
            "session_id": session_id, "tenant": tenant, "task": task,
            "next_sample_id": result.next_sample_id,
            "class_set": class_set,
        })

    @app.get("/labeler/expert/{tenant}/{task}", response_class=HTMLResponse)
    async def expert_queue(
        request: Request, tenant: str, task: str,
        lbl: LabelingFacade = Depends(_lbl),
    ) -> Any:
        queue = await lbl.escalation_queue(
            tenant_id=tenant, task_id=task, limit=50,
        )
        return tpl.TemplateResponse(request, "expert_queue.html", {
            "tenant": tenant, "task": task,
            "items": queue, "class_set": class_set,
        })

    @app.post("/labeler/expert/resolve", response_class=HTMLResponse)
    async def expert_resolve(
        request: Request,
        tenant: str = Form(...),
        sample_id: str = Form(...),
        expert_id: str = Form(...),
        label: str = Form(...),
        lbl: LabelingFacade = Depends(_lbl),
    ) -> Any:
        ref = await lbl.resolve_escalation(
            tenant_id=tenant, sample_id=sample_id,
            expert_id=expert_id, label=label,
        )
        return HTMLResponse(
            f"<li class='resolved'>resolved ✓ (history_id={ref[:12]}…)</li>",
        )

    return app


__all__ = ["build_ui_app"]
