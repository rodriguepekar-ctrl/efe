"""Consumer helper for the ``ExecuteStreaming`` RPC."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, AsyncIterator, Literal

from efe_swarm_integration.contract.adapter_result import parse_stream_chunk
from efe_swarm_integration.errors import StreamAborted, UnexpectedChunk
from efe_swarm_integration.types import (
    ExecuteResponse,
    ServerEvent,
    StreamChunk,
    TextDelta,
)

__all__ = ["StreamEvent", "consume_stream"]


@dataclass(frozen=True, slots=True)
class StreamEvent:
    """Normalised event yielded by :func:`consume_stream`."""

    kind: Literal["text_delta", "server_event", "final"]
    text_delta: TextDelta | None = None
    server_event: ServerEvent | None = None
    final: ExecuteResponse | None = None


async def consume_stream(
    chunks: AsyncIterator[dict[str, Any]],
) -> AsyncIterator[StreamEvent]:
    """Parse raw wire chunks and yield typed :class:`StreamEvent` instances.

    Guarantees :

    * A ``final`` event is always produced last (or a
      :class:`StreamAborted` is raised).
    * :class:`UnexpectedChunk` is raised on malformed payloads.
    """
    seen_final = False
    async for raw in chunks:
        try:
            chunk: StreamChunk = parse_stream_chunk(raw)
        except Exception as exc:
            raise UnexpectedChunk(f"bad stream chunk: {exc}") from exc
        if chunk.payload_kind == "text_delta":
            yield StreamEvent(kind="text_delta", text_delta=chunk.text_delta)
        elif chunk.payload_kind == "server_event":
            yield StreamEvent(
                kind="server_event", server_event=chunk.server_event
            )
        elif chunk.payload_kind == "final":
            seen_final = True
            yield StreamEvent(kind="final", final=chunk.final)
            return
    if not seen_final:
        raise StreamAborted("stream ended before final chunk")
