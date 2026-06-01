"""StreamMultiplexer — unify backend chunks into canonical ChatChunks.

Real backends each speak a different streaming dialect :

* OpenAI SSE     : ``data: {"choices":[{"delta":{"content":"..."}}]}``
* Anthropic SSE  : typed events ``content_block_delta``, ``message_stop``
* vLLM SSE       : OpenAI-compatible
* Gemini         : chunks of ``GenerateContentResponse``

The multiplexer takes an async iterator of **backend-specific chunks**
(already parsed from the wire) and yields :class:`ChatChunk` in
canonical form, optionally also returning an SSE string for pass-through
to an OpenAI-compatible client.
"""

from __future__ import annotations

import json
from collections.abc import AsyncIterator
from typing import Any

from efe_inference.messages.response import (
    ChatChunk,
    FinishReason,
    ToolCallDelta,
    Usage,
)


def sse_chunk_to_canonical_openai(raw: dict[str, Any]) -> ChatChunk:
    """Translate one OpenAI streaming JSON payload to ChatChunk."""
    choices = raw.get("choices") or []
    if not choices:
        return ChatChunk()
    first = choices[0]
    delta = first.get("delta") or {}
    tool_call_delta: ToolCallDelta | None = None
    tool_calls = delta.get("tool_calls") or []
    if tool_calls:
        tc = tool_calls[0]
        fn = tc.get("function") or {}
        tool_call_delta = ToolCallDelta(
            id=tc.get("id"),
            name=fn.get("name"),
            arguments_delta=fn.get("arguments") or "",
        )
    finish = first.get("finish_reason")
    usage = None
    usage_raw = raw.get("usage")
    if isinstance(usage_raw, dict):
        pt = int(usage_raw.get("prompt_tokens", 0))
        ct = int(usage_raw.get("completion_tokens", 0))
        usage = Usage(
            prompt_tokens=pt,
            completion_tokens=ct,
            total_tokens=pt + ct,
        )
    return ChatChunk(
        index=int(first.get("index", 0)),
        delta_text=delta.get("content"),
        delta_tool_call=tool_call_delta,
        finish_reason=FinishReason(finish) if finish else None,
        usage=usage,
    )


def sse_format_openai(chunk: ChatChunk, *, model: str) -> str:
    """Render a canonical chunk as an OpenAI-compat SSE line."""
    payload: dict[str, Any] = {
        "object": "chat.completion.chunk",
        "model": model,
        "choices": [
            {
                "index": chunk.index,
                "delta": {},
                "finish_reason": (
                    chunk.finish_reason.value if chunk.finish_reason else None
                ),
            },
        ],
    }
    if chunk.delta_text is not None:
        payload["choices"][0]["delta"]["content"] = chunk.delta_text
    if chunk.delta_tool_call is not None:
        tc = chunk.delta_tool_call
        payload["choices"][0]["delta"]["tool_calls"] = [
            {
                "id": tc.id,
                "type": "function",
                "function": {
                    "name": tc.name,
                    "arguments": tc.arguments_delta,
                },
            },
        ]
    if chunk.usage is not None:
        payload["usage"] = {
            "prompt_tokens": chunk.usage.prompt_tokens,
            "completion_tokens": chunk.usage.completion_tokens,
            "total_tokens": chunk.usage.total_tokens,
        }
    return "data: " + json.dumps(payload, ensure_ascii=False) + "\n\n"


class StreamMultiplexer:
    """Turn any per-backend chunk iterator into a ChatChunk iterator."""

    def __init__(self, backend_kind: str) -> None:
        self._kind = backend_kind

    async def from_openai(
        self, src: AsyncIterator[dict[str, Any]],
    ) -> AsyncIterator[ChatChunk]:
        async for raw in src:
            yield sse_chunk_to_canonical_openai(raw)

    async def from_anthropic(
        self, src: AsyncIterator[dict[str, Any]],
    ) -> AsyncIterator[ChatChunk]:
        """Minimal Anthropic event demuxer.

        Anthropic sends typed events : ``content_block_delta``,
        ``content_block_stop``, ``message_delta``, ``message_stop``.
        We only emit ChatChunks for deltas and the final stop with
        usage.
        """
        async for raw in src:
            t = raw.get("type")
            if t == "content_block_delta":
                delta = (raw.get("delta") or {})
                kind = delta.get("type")
                if kind == "text_delta":
                    yield ChatChunk(delta_text=str(delta.get("text", "")))
                elif kind == "input_json_delta":
                    yield ChatChunk(
                        delta_tool_call=ToolCallDelta(
                            id=None, name=None,
                            arguments_delta=str(delta.get("partial_json", "")),
                        ),
                    )
            elif t == "message_delta":
                stop = (raw.get("delta") or {}).get("stop_reason")
                usage_raw = raw.get("usage") or {}
                usage = None
                if usage_raw:
                    pt = int(usage_raw.get("input_tokens", 0))
                    ct = int(usage_raw.get("output_tokens", 0))
                    usage = Usage(
                        prompt_tokens=pt, completion_tokens=ct,
                        total_tokens=pt + ct,
                    )
                yield ChatChunk(
                    finish_reason=(
                        FinishReason.STOP if stop == "end_turn"
                        else FinishReason.TOOL_CALLS if stop == "tool_use"
                        else FinishReason.LENGTH if stop == "max_tokens"
                        else None
                    ),
                    usage=usage,
                )


__all__ = [
    "StreamMultiplexer",
    "sse_chunk_to_canonical_openai",
    "sse_format_openai",
]
