"""FastAPI integration for Letter D.

Exports:

* :func:`efe_lifespan`                     — pass to ``FastAPI(lifespan=...)``
* :func:`get_efe`                          — dependency for routes
* :func:`register_efe_exception_handlers`  — HTTP error mapping

Usage::

    from fastapi import FastAPI, Depends
    from efe_tool.integrations.fastapi import (
        efe_lifespan, get_efe, register_efe_exception_handlers,
    )

    app = FastAPI(lifespan=efe_lifespan)
    register_efe_exception_handlers(app)

    @app.post("/tickets")
    async def classify(body: TicketIn, efe = Depends(get_efe)):
        v = await efe.classify_async(body.message, catalog="support_triage")
        return v.as_dict()

The module **does not** import ``fastapi`` at the top level. Hosts that
don't use FastAPI can still ``import efe_tool.integrations`` without
installing it.
"""

from __future__ import annotations

import logging
from contextlib import asynccontextmanager
from typing import Any, AsyncIterator, Callable, Optional

from . import EfeFacade
from ._errors import EfeFacadeError


_log = logging.getLogger("efe_tool.integrations.fastapi")


# ---------------------------------------------------------------------------
#  Lifespan
# ---------------------------------------------------------------------------


@asynccontextmanager
async def efe_lifespan(app: Any) -> AsyncIterator[None]:
    """FastAPI lifespan context manager.

    On startup: constructs :class:`EfeFacade.from_env()` and blocks on
    :meth:`preload`. On shutdown: calls :meth:`EfeFacade.close`.

    The instance is stored on ``app.state.efe`` for dependency access.
    If a custom factory is needed, set ``app.state.efe_factory`` to a
    zero-arg callable before startup; the lifespan will invoke it.
    """
    factory: Callable[[], EfeFacade] = getattr(
        app.state, "efe_factory", EfeFacade.from_env,
    )
    facade = factory()
    try:
        facade.preload()
    except Exception:
        _log.exception("efe_facade_preload_failed_on_lifespan")
        raise
    app.state.efe = facade
    _log.info("efe_facade_attached_to_app_state",
              extra={"endpoint": facade.config.endpoint})
    try:
        yield
    finally:
        try:
            facade.close(timeout_s=30.0)
        except Exception:  # pragma: no cover
            _log.exception("efe_facade_close_error")


# ---------------------------------------------------------------------------
#  Dependency
# ---------------------------------------------------------------------------


def get_efe(request: Any = None) -> EfeFacade:
    """FastAPI dependency that returns the app-scoped facade.

    Usage::

        @app.get("/")
        async def root(efe: EfeFacade = Depends(get_efe)):
            ...

    In tests, override via::

        app.dependency_overrides[get_efe] = lambda: FakeEfeFacade()
    """
    if request is None:  # pragma: no cover — used only via Depends
        raise RuntimeError("get_efe must be wrapped with Depends(...)")
    facade = getattr(request.app.state, "efe", None)
    if facade is None:
        raise EfeFacadeError(
            "no EfeFacade attached to app.state — "
            "did you configure lifespan=efe_lifespan?",
            hint="add `lifespan=efe_lifespan` to your FastAPI() constructor",
        )
    return facade


# ---------------------------------------------------------------------------
#  Exception handlers
# ---------------------------------------------------------------------------


def register_efe_exception_handlers(app: Any) -> None:
    """Register a handler that turns every :class:`EfeFacadeError` into
    a JSON response with the appropriate status code."""
    try:
        from fastapi import Request
        from fastapi.responses import JSONResponse
    except ImportError as e:  # pragma: no cover
        raise RuntimeError(
            "register_efe_exception_handlers requires FastAPI; install it first",
        ) from e

    @app.exception_handler(EfeFacadeError)
    async def _handler(request: "Request", exc: EfeFacadeError) -> "JSONResponse":
        _log.warning(
            "efe_facade_error_on_request",
            extra={
                "path": str(request.url.path),
                "error_code": exc.error_code,
                "retryable": exc.retryable,
            },
        )
        return JSONResponse(
            status_code=exc.suggested_http_status,
            content=exc.to_payload(),
        )


# ---------------------------------------------------------------------------
#  Health endpoints helper
# ---------------------------------------------------------------------------


def register_health_routes(
    app: Any,
    *,
    path_live: str = "/livez",
    path_ready: str = "/readyz",
) -> None:
    """Install ``/livez`` and ``/readyz`` endpoints wired to the facade.

    Conforms to Kubernetes conventions: ``/livez`` returns 200 as long
    as the process is alive; ``/readyz`` returns 200 only when the
    facade is ready and the gRPC server is serving.
    """
    try:
        from fastapi import Response  # noqa: F401
    except ImportError as e:  # pragma: no cover
        raise RuntimeError(
            "register_health_routes requires FastAPI",
        ) from e

    @app.get(path_live)
    async def _live() -> dict[str, str]:
        return {"status": "ok"}

    @app.get(path_ready)
    async def _ready(request: Any):
        facade = getattr(request.app.state, "efe", None)
        if facade is None:
            from fastapi.responses import JSONResponse
            return JSONResponse(
                status_code=503,
                content={"status": "not_ready",
                         "reason": "facade not attached"},
            )
        hs = facade.healthcheck()
        if hs.serving:
            return hs.as_dict()
        from fastapi.responses import JSONResponse
        return JSONResponse(status_code=503, content=hs.as_dict())


__all__ = [
    "efe_lifespan",
    "get_efe",
    "register_efe_exception_handlers",
    "register_health_routes",
]
