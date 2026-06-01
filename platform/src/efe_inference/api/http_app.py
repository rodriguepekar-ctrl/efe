"""FastAPI app factory — exposes 3 façades.

* ``/v1/chat/completions`` (OpenAI-compat)
* ``/v1/messages``          (Anthropic-compat)
* ``/v1/inference/chat``    (Native)

FastAPI is imported lazily so the package stays usable in test
environments without it installed.
"""

from __future__ import annotations

from typing import Any

from efe_inference.api.context import ApiContext
from efe_inference.errors import InferenceError
from efe_inference.messages.normalizer import (
    AnthropicNormalizer,
    NativeNormalizer,
    OpenAINormalizer,
)


def build_app(ctx: ApiContext) -> Any:
    try:
        from fastapi import FastAPI, Request
        from fastapi.responses import JSONResponse
    except ImportError as exc:  # pragma: no cover
        raise RuntimeError(
            "FastAPI not installed; cannot build HTTP app",
        ) from exc

    app = FastAPI(
        title="efe-inference",
        version="0.1.0",
        openapi_url="/openapi.json",
    )

    @app.exception_handler(InferenceError)
    async def _err(request: Request, exc: InferenceError) -> Any:
        return JSONResponse(
            status_code=exc.http_status, content=exc.to_dict(),
        )

    @app.get("/healthz")
    async def healthz() -> dict[str, str]:
        return {"status": "ok"}

    if ctx.settings.api.enable_openai_shim:
        _register_openai_routes(app, ctx)
    if ctx.settings.api.enable_anthropic_shim:
        _register_anthropic_routes(app, ctx)
    if ctx.settings.api.enable_native_shim:
        _register_native_routes(app, ctx)

    return app


def _register_openai_routes(app: Any, ctx: ApiContext) -> None:
    from fastapi import Request

    @app.post("/v1/chat/completions")  # type: ignore[untyped-decorator]
    async def chat_completions(request: Request) -> dict[str, Any]:
        body = await request.json()
        tenant = _require_tenant(request)
        task = _require_task(request)
        req = OpenAINormalizer.normalize(
            body, tenant_id=tenant, task_id=task,
        )
        resp = await ctx.adapter.generate(req)
        return _chat_response_to_openai(resp)


def _register_anthropic_routes(app: Any, ctx: ApiContext) -> None:
    from fastapi import Request

    @app.post("/v1/messages")  # type: ignore[untyped-decorator]
    async def messages(request: Request) -> dict[str, Any]:
        body = await request.json()
        tenant = _require_tenant(request)
        task = _require_task(request)
        req = AnthropicNormalizer.normalize(
            body, tenant_id=tenant, task_id=task,
        )
        resp = await ctx.adapter.generate(req)
        return _chat_response_to_anthropic(resp)


def _register_native_routes(app: Any, ctx: ApiContext) -> None:
    from fastapi import Request

    @app.post("/v1/inference/chat")  # type: ignore[untyped-decorator]
    async def native_chat(request: Request) -> dict[str, Any]:
        body = await request.json()
        tenant = _require_tenant(request)
        # Fix M7 — task_id comes from the gateway-signed header, not
        # from the request body. A body-supplied task_id is refused if
        # it disagrees with the header (403).
        task = _require_task(request)
        body_task = body.get("task_id")
        if body_task and str(body_task).strip() != task:
            from fastapi import HTTPException

            raise HTTPException(
                status_code=403,
                detail=(
                    f"body.task_id={body_task!r} disagrees with "
                    f"X-EFE-Task-Id={task!r}"
                ),
            )
        req = NativeNormalizer.normalize(
            body, tenant_id=tenant, task_id=task,
        )
        resp = await ctx.adapter.generate(req)
        return _chat_response_to_native(resp)


# ---------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------


def _require_tenant(request: Any) -> str:
    from fastapi import HTTPException

    hdr = request.headers.get("x-efe-tenant-id")
    if not hdr or not str(hdr).strip():
        raise HTTPException(
            status_code=401,
            detail="X-EFE-Tenant-Id header is required",
        )
    return str(hdr).strip()


def _require_task(request: Any) -> str:
    from fastapi import HTTPException

    hdr = request.headers.get("x-efe-task-id")
    if not hdr or not str(hdr).strip():
        raise HTTPException(
            status_code=401,
            detail="X-EFE-Task-Id header is required",
        )
    return str(hdr).strip()


def _chat_response_to_openai(resp: Any) -> dict[str, Any]:
    return {
        "id": resp.request_id,
        "object": "chat.completion",
        "model": resp.model,
        "choices": [
            {
                "index": c.index,
                "message": {
                    "role": c.message.role.value,
                    "content": c.message.text(),
                },
                "finish_reason": c.finish_reason.value,
            }
            for c in resp.choices
        ],
        "usage": {
            "prompt_tokens": resp.usage.prompt_tokens,
            "completion_tokens": resp.usage.completion_tokens,
            "total_tokens": resp.usage.total_tokens,
        },
        "efe": {
            "backend_kind": resp.backend_kind,
            "adapter_overhead_ms": resp.adapter_overhead_ms,
            "cached": resp.cached,
            "safety_flags": resp.safety_flags,
        },
    }


def _chat_response_to_anthropic(resp: Any) -> dict[str, Any]:
    c = resp.choices[0] if resp.choices else None
    text = c.message.text() if c else ""
    return {
        "id": resp.request_id,
        "type": "message",
        "role": "assistant",
        "model": resp.model,
        "content": [{"type": "text", "text": text}],
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": resp.usage.prompt_tokens,
            "output_tokens": resp.usage.completion_tokens,
        },
    }


def _chat_response_to_native(resp: Any) -> dict[str, Any]:
    return {
        "request_id": resp.request_id,
        "model": resp.model,
        "choices": [
            {
                "index": c.index,
                "message": {
                    "role": c.message.role.value,
                    "content": [
                        {"kind": "text", "text": c.message.text()},
                    ],
                },
                "finish_reason": c.finish_reason.value,
            }
            for c in resp.choices
        ],
        "usage": {
            "prompt_tokens": resp.usage.prompt_tokens,
            "completion_tokens": resp.usage.completion_tokens,
            "total_tokens": resp.usage.total_tokens,
        },
        "backend_kind": resp.backend_kind,
        "backend_latency_ms": resp.backend_latency_ms,
        "adapter_overhead_ms": resp.adapter_overhead_ms,
        "safety_flags": resp.safety_flags,
        "cached": resp.cached,
    }


__all__ = ["build_app"]
