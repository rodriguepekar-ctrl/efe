"""Canonical :class:`ChatResponse` + companions."""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
from typing import Any

from efe_inference.messages.request import ChatMessage


class FinishReason(str, Enum):
    STOP = "stop"
    LENGTH = "length"
    CONTENT_FILTER = "content_filter"
    TOOL_CALLS = "tool_calls"
    TIMEOUT = "timeout"
    BACKEND_ERROR = "backend_error"


@dataclass(frozen=True, slots=True)
class Usage:
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    prompt_cache_hits: int = 0

    def __post_init__(self) -> None:
        if self.prompt_tokens < 0 or self.completion_tokens < 0:
            raise ValueError("token counts must be non-negative")
        expected = self.prompt_tokens + self.completion_tokens
        if self.total_tokens != expected:
            raise ValueError(
                f"total_tokens={self.total_tokens} does not equal "
                f"prompt ({self.prompt_tokens}) + "
                f"completion ({self.completion_tokens})",
            )


@dataclass(frozen=True, slots=True)
class ToolCall:
    """Parsed, validated tool call emitted by the assistant."""

    id: str
    name: str
    arguments: dict[str, Any]


@dataclass(frozen=True, slots=True)
class ChatChoice:
    index: int
    message: ChatMessage
    finish_reason: FinishReason


@dataclass(frozen=True, slots=True)
class ChatResponse:
    request_id: str
    model: str
    choices: tuple[ChatChoice, ...]
    usage: Usage
    backend_kind: str
    backend_latency_ms: float
    adapter_overhead_ms: float
    safety_flags: dict[str, list[str]] = field(default_factory=dict)
    cached: str = "no"     # "no" | "verbatim" | "semantic"


@dataclass(frozen=True, slots=True)
class ToolCallDelta:
    """Incremental tool call delta over a stream."""

    id: str | None
    name: str | None
    arguments_delta: str


@dataclass(frozen=True, slots=True)
class ChatChunk:
    """One streaming chunk in canonical form."""

    index: int = 0
    delta_text: str | None = None
    delta_tool_call: ToolCallDelta | None = None
    finish_reason: FinishReason | None = None
    usage: Usage | None = None


__all__ = [
    "ChatChoice",
    "ChatChunk",
    "ChatResponse",
    "FinishReason",
    "ToolCall",
    "ToolCallDelta",
    "Usage",
]
