"""OpenAiBackend — OpenAI-compatible chat completions over HTTP."""

from __future__ import annotations

from collections.abc import AsyncIterator
from dataclasses import dataclass
from typing import Any

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


@dataclass
class OpenAiBackend:
    name: str
    api_key: str
    kind: BackendKind = BackendKind.OPENAI
    base_url: str = "https://api.openai.com/v1"
    org_id: str | None = None
    http_client: Any = None

    async def generate(self, req: BackendRequest) -> BackendResponse:
        if self.http_client is None:
            raise BackendUnavailable("OpenAiBackend needs http_client injected")
        payload = self._payload(req)
        try:
            r = await self.http_client.post(
                f"{self.base_url}/chat/completions",
                json=payload, timeout=req.timeout_s,
                headers=self._headers(),
            )
        except Exception as exc:
            raise BackendUnavailable(f"openai http error: {exc}") from exc
        _raise_for_status(r)
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

    async def stream(
        self, req: BackendRequest,
    ) -> AsyncIterator[ChatChunk]:
        resp = await self.generate(req)
        yield ChatChunk(
            delta_text=resp.text,
            finish_reason=resp.finish_reason,
            usage=resp.usage,
        )

    async def health(self) -> HealthStatus:
        if self.http_client is None:
            return HealthStatus.DEGRADED
        try:
            r = await self.http_client.get(
                f"{self.base_url}/models",
                headers=self._headers(),
                timeout=3.0,
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

    # ------------------------------------------------------- private

    def _headers(self) -> dict[str, str]:
        hdrs = {
            "Authorization": f"Bearer {self.api_key}",
            "Content-Type": "application/json",
        }
        if self.org_id:
            hdrs["OpenAI-Organization"] = self.org_id
        return hdrs

    def _payload(self, req: BackendRequest) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "model": req.model,
            "messages": list(req.messages or ()),
            "temperature": req.sampling.temperature,
            "top_p": req.sampling.top_p,
            "max_tokens": req.sampling.max_tokens,
            "stream": False,
        }
        if req.sampling.seed is not None:
            payload["seed"] = req.sampling.seed
        if req.sampling.stop:
            payload["stop"] = list(req.sampling.stop)
        # Fix H6 — forward presence / frequency penalty when non-default.
        if req.sampling.presence_penalty != 0.0:
            payload["presence_penalty"] = req.sampling.presence_penalty
        if req.sampling.frequency_penalty != 0.0:
            payload["frequency_penalty"] = req.sampling.frequency_penalty
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


def _raise_for_status(r: Any) -> None:
    if r.status_code == 401:
        raise BackendAuthError("openai 401 unauthorized")
    if r.status_code == 400:
        raise BackendBadRequest(f"openai 400: {r.text[:512]}")
    if r.status_code == 429:
        raise BackendRateLimited("openai 429 rate limited")
    if r.status_code >= 500:
        raise BackendUnavailable(f"openai {r.status_code}: {r.text[:256]}")


def _finish(raw: Any) -> FinishReason:
    try:
        return FinishReason(str(raw))
    except ValueError:
        return FinishReason.STOP


__all__ = ["OpenAiBackend"]
