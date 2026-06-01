"""VllmBackend — delegates routing to M06, speaks OpenAI-compat HTTP.

At runtime this backend :

1. Asks M06.TenantRouter for a :class:`RouteDecision` (cached 1s).
2. Forwards an OpenAI ``/v1/chat/completions`` payload to the endpoint
   URL.
3. Parses the response back into a :class:`BackendResponse`.

M06 is the canonical routing authority — we never second-guess its
choice. If the returned URL 5xx's, we raise :class:`BackendUnavailable`
and the circuit breaker opens naturally.

The backend is stateless beyond the HTTP client; it can be replicated
safely.
"""

from __future__ import annotations

from collections.abc import AsyncIterator
from dataclasses import dataclass, field
from typing import Any, Protocol

from efe_inference.backends.protocol import (
    BackendRequest,
    BackendResponse,
    HealthStatus,
)
from efe_inference.config import BackendKind
from efe_inference.errors import (
    BackendAuthError,
    BackendBadRequest,
    BackendRateLimited,
    BackendUnavailable,
)
from efe_inference.messages.response import (
    ChatChunk,
    FinishReason,
    Usage,
)


class M06RouterClient(Protocol):
    async def route(
        self, *, tenant_id: str, task_id: str, variant: str = "default",
    ) -> RouteDecisionLike:  # pragma: no cover
        ...


@dataclass(frozen=True, slots=True)
class RouteDecisionLike:
    endpoint_url: str
    artifact_id: str | None = None


@dataclass
class _InMemoryRouter:
    """Single-URL router used when M06 is not available (dev)."""

    url: str

    async def route(
        self, *, tenant_id: str, task_id: str, variant: str = "default",
    ) -> RouteDecisionLike:
        return RouteDecisionLike(endpoint_url=self.url, artifact_id=None)


@dataclass
class VllmBackend:
    name: str
    kind: BackendKind = BackendKind.VLLM_LOCAL
    router: M06RouterClient = field(
        default_factory=lambda: _InMemoryRouter("http://localhost:8000"),
    )
    http_client: Any = None                # httpx.AsyncClient or compatible

    async def generate(self, req: BackendRequest) -> BackendResponse:
        route = await self.router.route(
            tenant_id=req.tenant_id, task_id=req.task_id,
        )
        payload = self._to_openai_payload(req)
        return await self._call(route.endpoint_url, payload, req.timeout_s)

    async def stream(
        self, req: BackendRequest,
    ) -> AsyncIterator[ChatChunk]:
        # vLLM OpenAI-compat streaming — simplified here; production
        # parses SSE lines into the OpenAI JSON chunk format and
        # delegates to the StreamMultiplexer.
        resp = await self.generate(req)
        yield ChatChunk(
            delta_text=resp.text,
            finish_reason=resp.finish_reason,
            usage=resp.usage,
        )

    async def health(self) -> HealthStatus:
        try:
            route = await self.router.route(
                tenant_id="_health", task_id="_health",
            )
            if self.http_client is None:
                return HealthStatus.HEALTHY
            r = await self.http_client.get(
                f"{route.endpoint_url}/healthz", timeout=2.0,
            )
            return (
                HealthStatus.HEALTHY if r.status_code < 500
                else HealthStatus.DEGRADED
            )
        except Exception:
            return HealthStatus.UNHEALTHY

    async def close(self) -> None:
        if self.http_client is not None:
            close = getattr(self.http_client, "aclose", None)
            if close is not None:
                await close()

    # -------- helpers --------

    def _to_openai_payload(self, req: BackendRequest) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "model": req.model,
            "messages": list(req.messages or ()),
            "temperature": req.sampling.temperature,
            "top_p": req.sampling.top_p,
            "max_tokens": req.sampling.max_tokens,
            "stream": False,
        }
        if req.tools:
            payload["tools"] = [
                {
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters_json_schema,
                    },
                }
                for t in req.tools
            ]
            payload["tool_choice"] = req.tool_choice
        return payload

    async def _call(
        self, url: str, payload: dict[str, Any], timeout_s: float,
    ) -> BackendResponse:
        if self.http_client is None:
            raise BackendUnavailable(
                "VllmBackend requires an http_client injected",
            )
        try:
            r = await self.http_client.post(
                f"{url}/v1/chat/completions", json=payload, timeout=timeout_s,
            )
        except Exception as exc:
            raise BackendUnavailable(f"vllm http error: {exc}") from exc
        if r.status_code == 401:
            raise BackendAuthError("vllm 401 unauthorized")
        if r.status_code == 400:
            raise BackendBadRequest(f"vllm 400: {r.text[:512]}")
        if r.status_code == 429:
            raise BackendRateLimited("vllm 429 rate limited")
        if r.status_code >= 500:
            raise BackendUnavailable(f"vllm {r.status_code}: {r.text[:256]}")
        data = r.json()
        choice = (data.get("choices") or [{}])[0]
        msg = choice.get("message") or {}
        usage_raw = data.get("usage") or {}
        pt = int(usage_raw.get("prompt_tokens", 0))
        ct = int(usage_raw.get("completion_tokens", 0))
        return BackendResponse(
            text=str(msg.get("content") or ""),
            tool_calls=tuple(msg.get("tool_calls") or ()),
            finish_reason=_finish(choice.get("finish_reason")),
            usage=Usage(
                prompt_tokens=pt, completion_tokens=ct,
                total_tokens=pt + ct,
            ),
            raw=data,
        )


def _finish(raw: Any) -> FinishReason:
    try:
        return FinishReason(str(raw))
    except ValueError:
        return FinishReason.STOP


__all__ = ["VllmBackend"]
