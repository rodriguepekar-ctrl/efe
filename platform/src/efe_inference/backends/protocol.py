"""BackendAdapter protocol + request/response DTOs.

Each concrete backend receives a :class:`BackendRequest` (already
rendered by :class:`PromptTemplater`) and returns a
:class:`BackendResponse` that the outer adapter will normalise to a
canonical :class:`ChatResponse`. Streaming follows the same pattern
but yields :class:`~efe_inference.messages.response.ChatChunk`.
"""

from __future__ import annotations

from collections.abc import AsyncIterator
from dataclasses import dataclass, field
from enum import Enum
from typing import Any, Protocol

from efe_inference.config import BackendKind
from efe_inference.messages.request import SamplingParams, ToolDef
from efe_inference.messages.response import ChatChunk, FinishReason, Usage


class HealthStatus(str, Enum):
    HEALTHY = "healthy"
    DEGRADED = "degraded"
    UNHEALTHY = "unhealthy"


@dataclass(frozen=True, slots=True)
class BackendRequest:
    """The input to any :class:`BackendAdapter`.

    ``textual`` is populated for text-in/text-out backends; ``messages``
    for structured APIs. Exactly one of the two is non-None.
    """

    request_id: str
    tenant_id: str
    task_id: str
    model: str
    textual: str | None
    messages: tuple[dict[str, Any], ...] | None
    sampling: SamplingParams
    tools: tuple[ToolDef, ...]
    tool_choice: str
    stream: bool
    timeout_s: float


@dataclass(frozen=True, slots=True)
class BackendResponse:
    """Backend-produced response, pre-safety-postflight."""

    text: str
    tool_calls: tuple[dict[str, Any], ...] = field(default_factory=tuple)
    finish_reason: FinishReason = FinishReason.STOP
    usage: Usage = field(
        default_factory=lambda: Usage(
            prompt_tokens=0, completion_tokens=0, total_tokens=0,
        ),
    )
    raw: dict[str, Any] = field(default_factory=dict)
    backend_latency_ms: float = 0.0


class BackendAdapter(Protocol):
    name: str
    kind: BackendKind

    async def generate(
        self, req: BackendRequest,
    ) -> BackendResponse:  # pragma: no cover
        ...

    def stream(
        self, req: BackendRequest,
    ) -> AsyncIterator[ChatChunk]:  # pragma: no cover
        """Return an async iterator of chat chunks.

        Declared as a plain ``def`` (not ``async def``) so concrete
        implementations can be async generators (``async def`` + ``yield``)
        without forcing the caller to ``await`` the call first.
        """
        ...

    async def health(self) -> HealthStatus:  # pragma: no cover
        ...

    async def close(self) -> None:  # pragma: no cover
        ...


__all__ = [
    "BackendAdapter",
    "BackendRequest",
    "BackendResponse",
    "HealthStatus",
]
