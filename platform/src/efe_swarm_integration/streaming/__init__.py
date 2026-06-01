"""Streaming utilities — consumer of server-streaming RPCs."""

from __future__ import annotations

from efe_swarm_integration.streaming.bidi import BidiStreamHandle
from efe_swarm_integration.streaming.server_stream import (
    StreamEvent,
    consume_stream,
)

__all__ = [
    "BidiStreamHandle",
    "StreamEvent",
    "consume_stream",
]
