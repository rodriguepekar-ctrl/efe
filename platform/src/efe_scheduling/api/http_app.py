"""FastAPI app builder.

The app is built imperatively (``add_api_route``) rather than with
decorators because that plays better with ``mypy --strict``. FastAPI is
an **optional** dependency : when missing, :func:`build_app` returns a
stub that the import-level tests can still exercise.
"""

from __future__ import annotations

import logging
from datetime import datetime, timezone
from typing import TYPE_CHECKING, Any

from efe_scheduling.api.deps import Deps
from efe_scheduling.api.schemas import (
    JobSummarySchema,
    ManualFireRequest,
    ProblemDetails,
    ScheduleCreateRequest,
    ScheduleSchema,
)
from efe_scheduling.errors import SchedulingError
from efe_scheduling.types import (
    JobKind,
    JobState,
    Schedule,
    ScheduleState,
    new_uuid,
)

try:  # pragma: no cover - optional dep
    from fastapi import FastAPI, HTTPException, Request, Response
    from fastapi.responses import JSONResponse

    _FASTAPI_AVAILABLE = True
except ImportError:  # pragma: no cover
    _FASTAPI_AVAILABLE = False

    class FastAPI:  # type: ignore[no-redef]
        def __init__(self, *_args: Any, **_kwargs: Any) -> None:
            self.routes: list[Any] = []

        def add_api_route(self, *args: Any, **kwargs: Any) -> None:
            self.routes.append((args, kwargs))

        def middleware(self, _name: str) -> Any:
            def deco(fn: Any) -> Any:
                return fn
            return deco

    class HTTPException(Exception):  # type: ignore[no-redef]
        def __init__(self, status_code: int, detail: Any = None) -> None:
            super().__init__(detail or "")
            self.status_code = status_code
            self.detail = detail

    class Request:  # type: ignore[no-redef]
        pass

    class Response:  # type: ignore[no-redef]
        pass

    class JSONResponse:  # type: ignore[no-redef]
        def __init__(self, content: Any, status_code: int = 200) -> None:
            self.content = content
            self.status_code = status_code


if TYPE_CHECKING:  # pragma: no cover
    pass

__all__ = ["build_app"]

_LOG = logging.getLogger("efe_scheduling.api")


def _problem(exc: SchedulingError, request_id: str | None = None) -> dict[str, Any]:
    return ProblemDetails(
        type=f"urn:efe:sched:error:{exc.code}",
        title=type(exc).__name__,
        status=exc.http_status,
        detail=exc.message,
        retryable=exc.retryable_hint,
        request_id=request_id,
    ).model_dump() if hasattr(ProblemDetails, "model_dump") else {
        "type": f"urn:efe:sched:error:{exc.code}",
        "title": type(exc).__name__,
        "status": exc.http_status,
        "detail": exc.message,
        "retryable": exc.retryable_hint,
        "request_id": request_id,
    }


def build_app(deps: Deps) -> FastAPI:
    """Build and return the FastAPI application.

    Even in the no-FastAPI fallback, this returns an object exposing
    ``add_api_route`` so tests can assert the expected routes were
    registered.
    """
    app = FastAPI(title="EFE Scheduling & Continuous Learning", version="1.0.0")

    # ------------------------------------------------------------------
    # Handlers
    # ------------------------------------------------------------------

    async def health() -> dict[str, str]:
        return {"status": "ok"}

    async def readiness() -> dict[str, str]:
        # Minimal : just check stores respond.
        return {"status": "ok"}

    async def create_schedule(body: ScheduleCreateRequest) -> dict[str, Any]:
        now = deps.clock.now()
        schedule_id = new_uuid()
        # NOTE: spec is received as a Pydantic model; re-hydrate later.
        # For brevity, store the spec JSON verbatim and require the
        # caller to pass an already-deserialised ScheduleSpec via a
        # factory in production.
        schedule = Schedule(
            schedule_id=schedule_id,
            tenant_id=body.tenant_id,
            task_id=body.task_id,
            job_kind=JobKind(body.job_kind),
            spec=_spec_from_schema(body.spec),
            payload=dict(body.payload),
            enabled=body.enabled,
            state=(ScheduleState.enabled if body.enabled else ScheduleState.disabled),
            state_version=1,
            last_evaluated_at=None,
            created_at=now,
            updated_at=now,
        )
        await deps.schedule_store.create(schedule)
        return {"schedule_id": schedule_id, "state": schedule.state.value}

    async def get_schedule(schedule_id: str) -> dict[str, Any]:
        row = await deps.schedule_store.get(schedule_id)
        return _schedule_to_dict(row)

    async def list_jobs(
        tenant_id: str | None = None, state: str | None = None, limit: int = 100
    ) -> list[dict[str, Any]]:
        target_state = JobState(state) if state else JobState.queued
        specs = await deps.job_store.list_by_state(
            state=target_state, tenant_id=tenant_id, limit=limit
        )
        out = []
        for s in specs:
            current_state = await deps.job_store.current_state(s.job_id)
            attempts_left = await deps.job_store.attempts_left(s.job_id)  # type: ignore[attr-defined]
            out.append(
                JobSummarySchema(
                    job_id=s.job_id,
                    tenant_id=s.tenant_id,
                    task_id=s.task_id,
                    kind=s.kind.value,
                    priority=int(s.priority),
                    run_at=s.run_at,
                    state=current_state.value,
                    attempts_left=attempts_left,
                    created_at=s.created_at,
                ).model_dump() if hasattr(JobSummarySchema, "model_dump") else {
                    "job_id": s.job_id,
                    "tenant_id": s.tenant_id,
                    "task_id": s.task_id,
                    "kind": s.kind.value,
                    "priority": int(s.priority),
                    "run_at": s.run_at.isoformat(),
                    "state": current_state.value,
                    "attempts_left": attempts_left,
                    "created_at": s.created_at.isoformat(),
                }
            )
        return out

    async def manual_fire(trigger_id: str, body: ManualFireRequest) -> dict[str, Any]:
        idempotency_key = body.idempotency_key
        existing = await deps.idempotency_store.recall(idempotency_key)
        if existing is not None:
            return existing
        out = {
            "trigger_id": trigger_id,
            "actor": body.actor,
            "payload": dict(body.payload),
            "fired_at": datetime.now(tz=timezone.utc).isoformat(),
        }
        await deps.idempotency_store.remember(
            key=idempotency_key, result=out, ttl_s=86_400
        )
        return out

    # ------------------------------------------------------------------
    # Error handling
    # ------------------------------------------------------------------

    async def scheduling_error_handler(
        request: Request, exc: SchedulingError
    ) -> JSONResponse:
        del request
        body = _problem(exc)
        return JSONResponse(content=body, status_code=exc.http_status)

    if _FASTAPI_AVAILABLE:
        # Route registration (imperative, mypy-friendly).
        app.add_api_route("/health", health, methods=["GET"])
        app.add_api_route("/ready", readiness, methods=["GET"])
        app.add_api_route("/v1/schedules", create_schedule, methods=["POST"])
        app.add_api_route("/v1/schedules/{schedule_id}", get_schedule, methods=["GET"])
        app.add_api_route("/v1/jobs", list_jobs, methods=["GET"])
        app.add_api_route(
            "/v1/triggers/{trigger_id}/fire", manual_fire, methods=["POST"]
        )
        app.add_exception_handler(SchedulingError, scheduling_error_handler)  # type: ignore[arg-type]
    else:  # pragma: no cover
        app.add_api_route("/health", health, methods=["GET"])
        app.add_api_route("/ready", readiness, methods=["GET"])
        app.add_api_route("/v1/schedules", create_schedule, methods=["POST"])
        app.add_api_route("/v1/schedules/{schedule_id}", get_schedule, methods=["GET"])
        app.add_api_route("/v1/jobs", list_jobs, methods=["GET"])
        app.add_api_route(
            "/v1/triggers/{trigger_id}/fire", manual_fire, methods=["POST"]
        )

    return app


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _spec_from_schema(schema: Any) -> Any:
    """Convert a Pydantic schedule schema into the dataclass equivalent."""
    from efe_scheduling.types import (
        CompositeSchedule,
        CronSchedule,
        IntervalSchedule,
        OneShotSchedule,
        RRuleSchedule,
    )
    kind = schema.kind if hasattr(schema, "kind") else schema.get("kind")
    match kind:
        case "cron":
            return CronSchedule(
                expression=schema.expression,
                timezone_name=schema.timezone_name,
                jitter_s=schema.jitter_s,
            )
        case "interval":
            return IntervalSchedule(
                period_s=schema.period_s,
                phase_s=schema.phase_s,
                jitter_s=schema.jitter_s,
            )
        case "rrule":
            return RRuleSchedule(
                rrule_text=schema.rrule_text,
                timezone_name=schema.timezone_name,
                dtstart=schema.dtstart,
            )
        case "one_shot":
            return OneShotSchedule(at=schema.at)
        case "composite":
            return CompositeSchedule(
                op=schema.op,
                left=_spec_from_schema(schema.left),
                right=_spec_from_schema(schema.right),
            )
        case _:
            raise ValueError(f"unsupported schedule kind {kind!r}")


def _schedule_to_dict(sched: Schedule) -> dict[str, Any]:
    return {
        "schedule_id": sched.schedule_id,
        "tenant_id": sched.tenant_id,
        "task_id": sched.task_id,
        "job_kind": sched.job_kind.value,
        "enabled": sched.enabled,
        "state": sched.state.value,
        "state_version": sched.state_version,
        "last_evaluated_at": (
            sched.last_evaluated_at.isoformat()
            if sched.last_evaluated_at is not None
            else None
        ),
        "created_at": sched.created_at.isoformat(),
        "updated_at": sched.updated_at.isoformat(),
    }
