"""Canonical :class:`ChatRequest` + companions.

All backends receive a rendered form of this structure; all inbound
requests (OpenAI / Anthropic / native) are normalised into it. The
types are frozen dataclasses with ``slots=True`` for speed and hash
stability — any accidental mutation raises :class:`FrozenInstanceError`
in tests.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
from typing import Any, Literal


class Role(str, Enum):
    SYSTEM = "system"
    USER = "user"
    ASSISTANT = "assistant"
    TOOL = "tool"


class BlockKind(str, Enum):
    TEXT = "text"
    IMAGE = "image"
    TOOL_USE = "tool_use"
    TOOL_RESULT = "tool_result"


@dataclass(frozen=True, slots=True)
class TextBlock:
    text: str = ""
    kind: BlockKind = BlockKind.TEXT

    def __post_init__(self) -> None:
        if not isinstance(self.text, str):
            object.__setattr__(self, "text", str(self.text))


@dataclass(frozen=True, slots=True)
class ImageBlock:
    image_url: str = ""        # data: URL or https:
    mime: str = "image/png"
    kind: BlockKind = BlockKind.IMAGE

    def __post_init__(self) -> None:
        if not self.image_url:
            raise ValueError("ImageBlock requires image_url")


@dataclass(frozen=True, slots=True)
class ToolUseBlock:
    """Assistant-emitted tool call in canonical form."""

    tool_call_id: str = ""
    tool_name: str = ""
    tool_args: dict[str, Any] = field(default_factory=dict)
    kind: BlockKind = BlockKind.TOOL_USE


@dataclass(frozen=True, slots=True)
class ToolResultBlock:
    """Tool result fed back to the model by the client."""

    tool_call_id: str = ""
    content: str = ""
    is_error: bool = False
    kind: BlockKind = BlockKind.TOOL_RESULT


ContentBlock = TextBlock | ImageBlock | ToolUseBlock | ToolResultBlock


@dataclass(frozen=True, slots=True)
class ChatMessage:
    role: Role
    content: tuple[ContentBlock, ...]
    name: str | None = None
    tool_call_id: str | None = None

    def __post_init__(self) -> None:
        if not isinstance(self.content, tuple):
            object.__setattr__(self, "content", tuple(self.content))
        if self.role == Role.TOOL and self.tool_call_id is None:
            raise ValueError("tool role requires tool_call_id")

    def text(self) -> str:
        """Concatenate every TextBlock; stable for hashing."""
        return "\n".join(
            b.text for b in self.content if isinstance(b, TextBlock)
        )


@dataclass(frozen=True, slots=True)
class SamplingParams:
    temperature: float = 1.0
    top_p: float = 1.0
    top_k: int | None = None
    max_tokens: int = 1024
    stop: tuple[str, ...] = ()
    presence_penalty: float = 0.0
    frequency_penalty: float = 0.0
    seed: int | None = None

    def __post_init__(self) -> None:
        if not 0.0 <= self.temperature <= 2.0:
            raise ValueError(
                f"temperature must be in [0, 2], got {self.temperature}",
            )
        if not 0.0 <= self.top_p <= 1.0:
            raise ValueError(
                f"top_p must be in [0, 1], got {self.top_p}",
            )
        if self.max_tokens < 1:
            raise ValueError(
                f"max_tokens must be >= 1, got {self.max_tokens}",
            )


@dataclass(frozen=True, slots=True)
class ToolDef:
    name: str
    description: str
    parameters_json_schema: dict[str, Any] = field(default_factory=dict)

    def __post_init__(self) -> None:
        if not self.name or not self.name.isidentifier():
            raise ValueError(
                f"tool name must be a valid identifier, got {self.name!r}",
            )


@dataclass(frozen=True, slots=True)
class ChatRequest:
    tenant_id: str
    task_id: str
    request_id: str
    messages: tuple[ChatMessage, ...]
    sampling: SamplingParams = field(default_factory=SamplingParams)
    tools: tuple[ToolDef, ...] = ()
    tool_choice: Literal["auto", "required", "none"] = "auto"
    stream: bool = False
    model_hint: str | None = None
    user_id: str | None = None
    timeout_budget_s: float = 30.0
    metadata: dict[str, str] = field(default_factory=dict)

    def __post_init__(self) -> None:
        if not self.tenant_id or not self.tenant_id.strip():
            raise ValueError("tenant_id is required")
        if not self.task_id or not self.task_id.strip():
            raise ValueError("task_id is required")
        if not self.request_id or not self.request_id.strip():
            raise ValueError("request_id is required")
        if not self.messages:
            raise ValueError("messages must be non-empty")
        if not isinstance(self.messages, tuple):
            object.__setattr__(self, "messages", tuple(self.messages))
        if self.timeout_budget_s <= 0:
            raise ValueError("timeout_budget_s must be positive")

    def total_text(self) -> str:
        return "\n\n".join(m.text() for m in self.messages)


__all__ = [
    "BlockKind",
    "ChatMessage",
    "ChatRequest",
    "ContentBlock",
    "ImageBlock",
    "Role",
    "SamplingParams",
    "TextBlock",
    "ToolDef",
    "ToolResultBlock",
    "ToolUseBlock",
]
