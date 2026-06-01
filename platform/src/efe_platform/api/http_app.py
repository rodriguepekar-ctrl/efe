"""FastAPI application wiring.

Exposed surface:

* ``POST /v1/answer`` — single-shot bot answer.
* ``POST /v1/answer/stream`` — SSE stream of trace events + final.
* ``GET  /v1/catalog`` — list catalog tasks.
* ``GET  /v1/swarm/healthz`` — upstream Rust server probe.
* ``GET  /healthz`` — platform liveness.
* ``GET  /metrics`` — Prometheus text format.

The app is constructed by :func:`build_app` which takes a fully
initialised :class:`PlatformContext`. The convenience factory
:func:`create_default_app` wires a mock-backed context suitable for
local development.
"""

from __future__ import annotations

import json
import time
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from typing import Any

from efe_platform.api.dependencies import (
    PlatformContext,
    clear_context,
    get_context,
    set_context,
)
from efe_platform.api.schemas import (
    AnswerRequest,
    AnswerResponse,
    CatalogResponse,
    CatalogTaskOut,
    HealthResponse,
    ProblemDetail,
)
from efe_platform.bot.agent import Bot, BotConfig
from efe_platform.bot.budget import Budget
from efe_platform.config import PlatformConfig
from efe_platform.errors import BudgetExceeded, EfePlatformError
from efe_platform.swarm.client import InMemorySwarmTransport, SwarmClient, SwarmEndpoint
from efe_platform.swarm.cone import ConeRegistryClient, InMemoryAdminTransport
from efe_platform.telemetry.audit import AuditLog
from efe_platform.telemetry.metrics import RequestMetrics
from efe_platform.telemetry.tracing import TracingService
from efe_platform.version import VERSION

# ---------------------------------------------------------------------------
#  App factory
# ---------------------------------------------------------------------------


def build_app(context: PlatformContext) -> Any:
    """Instantiate the FastAPI application with the given context."""
    try:
        from fastapi import Depends, FastAPI, HTTPException
        from fastapi.responses import PlainTextResponse, StreamingResponse
    except ImportError as exc:  # pragma: no cover — optional dep
        raise RuntimeError(
            "FastAPI is not installed — install with `pip install efe-platform[api]`."
        ) from exc

    @asynccontextmanager
    async def lifespan(app: Any) -> AsyncIterator[None]:
        set_context(context)
        try:
            yield
        finally:
            # Graceful shutdown: close swarm, close audit.
            await context.swarm.close()
            if context.audit is not None:
                context.audit.close()
            clear_context()

    app = FastAPI(
        title="EFE Platform",
        version=VERSION,
        lifespan=lifespan,
        docs_url="/v1/docs",
        openapi_url="/v1/openapi.json",
    )

    # ---------------------------------------------------------- Routes

    @app.get("/healthz", response_model=HealthResponse)
    async def healthz(ctx: PlatformContext = Depends(get_context)) -> HealthResponse:
        upstream = await _probe_swarm(ctx)
        return HealthResponse(
            status="ok",
            version=VERSION,
            upstream_swarm_reachable=upstream,
        )

    @app.get("/v1/swarm/healthz", response_model=HealthResponse)
    async def swarm_healthz(
        ctx: PlatformContext = Depends(get_context),
    ) -> HealthResponse:
        upstream = await _probe_swarm(ctx)
        return HealthResponse(
            status="ok" if upstream else "degraded",
            version=VERSION,
            upstream_swarm_reachable=upstream,
        )

    @app.get("/metrics")
    async def metrics(ctx: PlatformContext = Depends(get_context)) -> Any:
        return PlainTextResponse(
            content=ctx.metrics.render(),
            media_type="text/plain; version=0.0.4; charset=utf-8",
        )

    @app.get("/v1/catalog/cones")
    async def list_cones(
        tenant: str,
        task: str,
        ctx: PlatformContext = Depends(get_context),
    ) -> dict[str, Any]:
        """List the registered alignment cones for a ``tenant/task`` pair."""
        if ctx.cones is None:
            raise HTTPException(
                status_code=501,
                detail="cone registry not configured on this deployment",
            )
        keys = await ctx.cones.list_versions(tenant, task)
        return {"tenant": tenant, "task": task, "versions": list(keys)}

    @app.get("/v1/catalog", response_model=CatalogResponse)
    async def catalog(
        ctx: PlatformContext = Depends(get_context),
    ) -> CatalogResponse:
        if ctx.catalog is None:
            return CatalogResponse(version=0, tasks=[])
        return CatalogResponse(
            version=ctx.catalog.version,
            tasks=[
                CatalogTaskOut(
                    id=t.id,
                    description=t.description,
                    tenant=t.tenant,
                    cone_key=t.cone_key,
                    dim_latent=t.dim_latent,
                    tags=list(t.tags),
                )
                for t in ctx.catalog.tasks
            ],
        )

    @app.post("/v1/answer", response_model=AnswerResponse)
    async def answer(
        body: AnswerRequest,
        ctx: PlatformContext = Depends(get_context),
    ) -> AnswerResponse:
        tenant = body.tenant or ctx.config.tenant_default
        budget = _maybe_budget(body, ctx.config.budget.max_wall_clock_ms)
        t0 = time.monotonic()
        try:
            reply = await ctx.bot.answer(
                body.prompt,
                tenant=tenant,
                request_id=body.request_id,
                budget=budget,
            )
        except EfePlatformError as exc:
            # Bot catches most — any that escape are bugs. Map to 500.
            raise HTTPException(
                status_code=500,
                detail=ProblemDetail(
                    title="Unhandled platform error",
                    status=500,
                    code=exc.code,
                    detail=exc.message,
                ).model_dump(),
            ) from exc
        duration_s = time.monotonic() - t0
        ctx.metrics.record_request(
            RequestMetrics(
                tenant=tenant,
                outcome="ok" if reply.ok else "error",
                duration_s=duration_s,
                tool_calls=int(reply.ledger.get("tool_calls_made", 0)),
                llm_tokens=int(reply.ledger.get("tokens_consumed", 0)),
                llm_usd=float(reply.ledger.get("usd_consumed", 0.0)),
            )
        )
        if reply.error == "budget_exceeded":
            # Extract the offending dimension from the structured recap
            # so the metric is actually actionable.
            detail = reply.recap.get("error_detail") or {}
            dim = str(detail.get("dimension", "unknown"))
            ctx.metrics.record_budget_exceeded(dimension=dim)
        if ctx.audit is not None:
            ctx.audit.record_bot_reply(
                request_id=reply.request_id,
                tenant=tenant,
                reply_payload=reply.to_dict(),
            )
        return AnswerResponse(**reply.to_dict())

    @app.post("/v1/answer/stream")
    async def answer_stream(
        body: AnswerRequest,
        ctx: PlatformContext = Depends(get_context),
    ) -> Any:
        """SSE stream — emits one event per trace step plus the final reply."""
        tenant = body.tenant or ctx.config.tenant_default
        budget = _maybe_budget(body, ctx.config.budget.max_wall_clock_ms)

        async def gen() -> AsyncIterator[bytes]:
            # Simplified: run the full bot call, then emit accumulated
            # trace as individual events. An incremental streaming
            # version would need a per-step queue — that's a V1.1.
            t0 = time.monotonic()
            reply = await ctx.bot.answer(
                body.prompt,
                tenant=tenant,
                request_id=body.request_id,
                budget=budget,
            )
            for ev in reply.trace:
                yield _sse(ev.kind, ev.to_dict())
            yield _sse("final", reply.to_dict())
            duration_s = time.monotonic() - t0
            ctx.metrics.record_request(
                RequestMetrics(
                    tenant=tenant,
                    outcome="ok" if reply.ok else "error",
                    duration_s=duration_s,
                    tool_calls=int(reply.ledger.get("tool_calls_made", 0)),
                    llm_tokens=int(reply.ledger.get("tokens_consumed", 0)),
                    llm_usd=float(reply.ledger.get("usd_consumed", 0.0)),
                )
            )

        return StreamingResponse(gen(), media_type="text/event-stream")

    # ---------------------------------------------------- Exception handlers

    @app.exception_handler(BudgetExceeded)
    async def _on_budget(_req: Any, exc: BudgetExceeded) -> Any:
        return _problem_response(402, "Payment Required", exc)

    @app.exception_handler(EfePlatformError)
    async def _on_platform(_req: Any, exc: EfePlatformError) -> Any:
        status = 400 if not exc.retryable else 503
        return _problem_response(status, "Platform error", exc)

    return app


# ---------------------------------------------------------------------------
#  Default app (for `efe-platform serve --dev`)
# ---------------------------------------------------------------------------


def create_default_app(config: PlatformConfig | None = None) -> Any:
    """Wire a mock-backed platform for local development / smoke tests."""
    cfg = config or PlatformConfig.from_env()
    swarm = SwarmClient(
        SwarmEndpoint(cfg.swarm.endpoint),
        transport=InMemorySwarmTransport(),
        default_deadline_ms=cfg.swarm.default_deadline_ms,
        max_retries=cfg.swarm.max_retries,
        jwt=cfg.swarm.jwt,
    )
    cones = ConeRegistryClient(InMemoryAdminTransport())
    bot = Bot(
        BotConfig(lang=(cfg.tenant_default and "fr") or "fr"),
        swarm=swarm,
        cones=cones,
    )
    audit: AuditLog | None = (
        AuditLog(cfg.audit.path, redact_prompts=cfg.audit.redact_prompts)
        if cfg.audit.enabled
        else None
    )
    tracing = (
        TracingService(
            service_name=cfg.telemetry.otel_service_name,
            otel_endpoint=cfg.telemetry.otel_endpoint,
        )
        if cfg.telemetry.otel_endpoint
        else None
    )
    ctx = PlatformContext(
        config=cfg,
        bot=bot,
        swarm=swarm,
        cones=cones,
        audit=audit,
        tracing=tracing,
    )
    return build_app(ctx)


# ---------------------------------------------------------------------------
#  Helpers
# ---------------------------------------------------------------------------


def _maybe_budget(body: AnswerRequest, default_ms: int) -> Budget | None:
    if all(
        v is None
        for v in (
            body.budget_ms,
            body.budget_tokens,
            body.budget_usd,
            body.budget_tool_calls,
        )
    ):
        return None
    return Budget(
        max_wall_clock_ms=body.budget_ms or default_ms,
        max_llm_tokens=body.budget_tokens or 16_000,
        max_usd=body.budget_usd if body.budget_usd is not None else 0.25,
        max_tool_calls=body.budget_tool_calls or 8,
    )


async def _probe_swarm(ctx: PlatformContext) -> bool:
    try:
        # Minimal touch: describe a trivial spec. Absorbs any error.
        from efe_platform.swarm.protocol import ConeRef
        from efe_platform.swarm.spec_builder import SpecBuilder

        spec = (
            SpecBuilder()
            .with_problem_embedding([0.0] * 16)
            .with_cone_ref(ConeRef("probe", "probe", 1))
            .build()
        )
        await ctx.swarm.describe(spec, deadline_ms=500)
        return True
    except Exception:
        return False


def _sse(event: str, payload: Any) -> bytes:
    body = json.dumps(payload, ensure_ascii=False)
    return f"event: {event}\ndata: {body}\n\n".encode()


def _problem_response(status: int, title: str, exc: EfePlatformError) -> Any:
    from fastapi.responses import JSONResponse

    problem = ProblemDetail(
        title=title, status=status, detail=exc.message, code=exc.code
    )
    return JSONResponse(status_code=status, content=problem.model_dump())


__all__ = ["build_app", "create_default_app"]
