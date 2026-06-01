"""ClaudeBackend — Anthropic /v1/messages over HTTP.

Subtleties handled :

* ``system`` lives in its own top-level field, not in messages.
* Roles alternate strictly user/assistant in the messages array.
* Tool use is a content block, not a top-level field.
* Usage is reported as ``input_tokens`` / ``output_tokens``.
"""

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
class ClaudeBackend:
    name: str
    api_key: str
    kind: BackendKind = BackendKind.ANTHROPIC
    base_url: str = "https://api.anthropic.com"
    anthropic_version: str = "2023-06-01"
    http_client: Any = None

    async def generate(self, req: BackendRequest) -> BackendResponse:
        if self.http_client is None:
            raise BackendUnavailable(
                "ClaudeBackend needs http_client injected",
            )
        payload = self._payload(req)
        try:
            r = await self.http_client.post(
                f"{self.base_url}/v1/messages",
                json=payload, timeout=req.timeout_s,
                headers=self._headers(),
            )
        except Exception as exc:
            raise BackendUnavailable(f"claude http error: {exc}") from exc
        _raise_for_status(r)
        data = r.json()
        text, tool_calls = _parse_content(data.get("content") or [])
        usage = _parse_usage(data.get("usage") or {})
        return BackendResponse(
            text=text,
            tool_calls=tool_calls,
            finish_reason=_finish(data.get("stop_reason")),
            usage=usage,
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
        # Anthropic has no public health endpoint; approximate via a
        # tiny listing call. In practice we rely on request-time errors
        # + circuit breaker to trip when upstream is unhappy.
        if self.http_client is None:
            return HealthStatus.DEGRADED
        return HealthStatus.HEALTHY

    async def close(self) -> None:
        if self.http_client is not None:
            close = getattr(self.http_client, "aclose", None)
            if close is not None:
                await close()

    # ------------------------------------------------------- private

    def _headers(self) -> dict[str, str]:
        return {
            "x-api-key": self.api_key,
            "anthropic-version": self.anthropic_version,
            "content-type": "application/json",
        }

    def _payload(self, req: BackendRequest) -> dict[str, Any]:
        """Build the Anthropic payload.

        Fix H4 — Anthropic requires strict ``user``/``assistant``
        alternation. Two consecutive same-role messages are merged
        (content lists concatenated); ``tool`` role is rewritten into
        an ``assistant`` tool_result block per the Anthropic spec.
        """
        messages_in = list(req.messages or ())
        system = ""
        chat_messages: list[dict[str, Any]] = []
        for m in messages_in:
            role = m.get("role")
            if role == "system":
                content = m.get("content")
                if isinstance(content, list):
                    system = "".join(
                        b.get("text", "") for b in content
                        if b.get("type") == "text"
                    )
                else:
                    system = str(content or "")
                continue
            # Normalise non-user/assistant roles: ``tool`` → ``user``
            # content tool_result (Anthropic convention).
            if role not in ("user", "assistant"):
                m = _coerce_non_chat_role(m)
            msg = _canonical_to_anthropic_message(m)
            if chat_messages and chat_messages[-1]["role"] == msg["role"]:
                # Merge consecutive same-role messages.
                merged_content = list(chat_messages[-1]["content"])
                merged_content.extend(msg["content"])
                chat_messages[-1]["content"] = merged_content
            else:
                chat_messages.append(msg)

        payload: dict[str, Any] = {
            "model": req.model,
            "messages": chat_messages,
            "max_tokens": req.sampling.max_tokens,
            "temperature": req.sampling.temperature,
            "top_p": req.sampling.top_p,
        }
        if system:
            payload["system"] = system
        if req.sampling.top_k is not None:
            payload["top_k"] = req.sampling.top_k
        if req.sampling.stop:
            payload["stop_sequences"] = list(req.sampling.stop)
        if req.tools:
            payload["tools"] = [
                {
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters_json_schema,
                }
                for t in req.tools
            ]
        return payload


def _coerce_non_chat_role(m: dict[str, Any]) -> dict[str, Any]:
    """Rewrite roles like ``tool`` into an Anthropic-compatible shape.

    Anthropic only accepts ``user``/``assistant``. A ``tool`` message in
    our canonical form is the client returning a tool_result block; we
    wrap it under a ``user`` role.
    """
    if m.get("role") == "tool":
        content = m.get("content")
        tool_call_id = m.get("tool_call_id", "")
        return {
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": tool_call_id,
                "content": content if isinstance(content, str) else "",
                "is_error": False,
            }],
        }
    # Fallback — coerce anything else to user.
    return {"role": "user", "content": m.get("content", [])}


def _canonical_to_anthropic_message(m: dict[str, Any]) -> dict[str, Any]:
    role = m.get("role")
    content = m.get("content")
    if isinstance(content, str):
        return {
            "role": role,
            "content": [{"type": "text", "text": content}],
        }
    out: list[dict[str, Any]] = []
    for b in content or []:
        t = b.get("type")
        if t == "text":
            out.append({"type": "text", "text": b.get("text", "")})
        elif t == "image":
            url = b.get("image_url", "")
            src = (
                {"type": "base64", "data": url.split(",", 1)[-1],
                 "media_type": b.get("mime", "image/png")}
                if url.startswith("data:") else
                {"type": "url", "url": url,
                 "media_type": b.get("mime", "image/png")}
            )
            out.append({"type": "image", "source": src})
        elif t == "tool_use":
            out.append({
                "type": "tool_use",
                "id": b.get("id", ""),
                "name": b.get("name", ""),
                "input": b.get("input", {}),
            })
        elif t == "tool_result":
            out.append({
                "type": "tool_result",
                "tool_use_id": b.get("tool_use_id", ""),
                "content": b.get("content", ""),
                "is_error": b.get("is_error", False),
            })
    return {"role": role, "content": out}


def _parse_content(
    blocks: list[dict[str, Any]],
) -> tuple[str, tuple[dict[str, Any], ...]]:
    text_parts: list[str] = []
    tool_calls: list[dict[str, Any]] = []
    for b in blocks:
        t = b.get("type")
        if t == "text":
            text_parts.append(str(b.get("text", "")))
        elif t == "tool_use":
            tool_calls.append({
                "id": b.get("id", ""),
                "type": "function",
                "function": {
                    "name": b.get("name", ""),
                    "arguments": _json_dumps_safe(b.get("input") or {}),
                },
            })
    return "".join(text_parts), tuple(tool_calls)


def _parse_usage(raw: dict[str, Any]) -> Usage:
    pt = int(raw.get("input_tokens", 0))
    ct = int(raw.get("output_tokens", 0))
    return Usage(
        prompt_tokens=pt, completion_tokens=ct, total_tokens=pt + ct,
    )


def _json_dumps_safe(v: Any) -> str:
    import json

    return json.dumps(v, ensure_ascii=False)


def _raise_for_status(r: Any) -> None:
    if r.status_code == 401:
        raise BackendAuthError("claude 401 unauthorized")
    if r.status_code == 400:
        raise BackendBadRequest(f"claude 400: {r.text[:512]}")
    if r.status_code == 429:
        raise BackendRateLimited("claude 429 rate limited")
    if r.status_code >= 500:
        raise BackendUnavailable(f"claude {r.status_code}: {r.text[:256]}")


_ANTHROPIC_STOP: dict[str, FinishReason] = {
    "end_turn": FinishReason.STOP,
    "max_tokens": FinishReason.LENGTH,
    "stop_sequence": FinishReason.STOP,
    "tool_use": FinishReason.TOOL_CALLS,
}


def _finish(raw: Any) -> FinishReason:
    return _ANTHROPIC_STOP.get(str(raw), FinishReason.STOP)


__all__ = ["ClaudeBackend"]
