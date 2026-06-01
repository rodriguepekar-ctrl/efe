"""Bounded conversation memory for the bot.

The bot does NOT rely on the LLM provider's hidden server-side memory.
Every turn is reconstructed from a :class:`ConversationMemory` so the
platform owns the full context window and can truncate it
deterministically.

Design notes
------------

* **FIFO with a cap** — by default 24 messages or 12 000 approx
  tokens, whichever comes first. Older messages are dropped oldest-first.

* **Role tagging** — ``"user" | "assistant" | "tool"`` only.
  Tool results are included as ``role="tool"`` with a JSON-encoded
  content — they're part of the conversation replay, not a side-channel.

* **Freezable snapshot** — :meth:`snapshot` returns an immutable tuple
  safe to hand off to another coroutine.
"""

from __future__ import annotations

import json
from collections import deque
from dataclasses import dataclass, field
from typing import Any, Literal

Role = Literal["user", "assistant", "tool"]


# ---------------------------------------------------------------------------
#  Message
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class MemoryMessage:
    """Immutable conversational message."""

    role: Role
    content: str
    # For tool messages, the tool-use id + tool name.
    tool_use_id: str | None = None
    tool_name: str | None = None
    # Pre-computed approximate token count for budgeting.
    approx_tokens: int = 0

    def __post_init__(self) -> None:
        if self.role == "tool":
            if not self.tool_use_id:
                raise ValueError("tool message requires tool_use_id")
            if not self.tool_name:
                raise ValueError("tool message requires tool_name")
        if self.approx_tokens < 0:
            raise ValueError("approx_tokens must be >= 0")

    def to_dict(self) -> dict[str, Any]:
        out: dict[str, Any] = {
            "role": self.role,
            "content": self.content,
            "approx_tokens": self.approx_tokens,
        }
        if self.tool_use_id:
            out["tool_use_id"] = self.tool_use_id
        if self.tool_name:
            out["tool_name"] = self.tool_name
        return out


# ---------------------------------------------------------------------------
#  Memory
# ---------------------------------------------------------------------------


def _approx_tokens(text: str) -> int:
    """Cheap deterministic token estimate — 1 token ≈ 4 characters.

    Used only for budget accounting, never for provider-visible counts.
    """
    return max(1, len(text) // 4)


@dataclass
class ConversationMemory:
    """Bounded conversation history."""

    max_messages: int = 24
    max_approx_tokens: int = 12_000
    _messages: deque[MemoryMessage] = field(default_factory=deque)
    _approx_tokens: int = 0

    def __post_init__(self) -> None:
        if self.max_messages <= 0:
            raise ValueError("max_messages must be > 0")
        if self.max_approx_tokens <= 0:
            raise ValueError("max_approx_tokens must be > 0")

    # --------------------------------------------------------- Mutators

    def add_user(self, text: str) -> MemoryMessage:
        msg = MemoryMessage(
            role="user", content=text, approx_tokens=_approx_tokens(text)
        )
        self._append(msg)
        return msg

    def add_assistant(self, text: str) -> MemoryMessage:
        msg = MemoryMessage(
            role="assistant", content=text, approx_tokens=_approx_tokens(text)
        )
        self._append(msg)
        return msg

    def add_tool_result(
        self, *, tool_use_id: str, tool_name: str, result: Any
    ) -> MemoryMessage:
        # Serialize the result for replay fidelity.
        content = json.dumps(result, ensure_ascii=False, default=_default_json)
        msg = MemoryMessage(
            role="tool",
            content=content,
            tool_use_id=tool_use_id,
            tool_name=tool_name,
            approx_tokens=_approx_tokens(content),
        )
        self._append(msg)
        return msg

    def clear(self) -> None:
        self._messages.clear()
        self._approx_tokens = 0

    # ---------------------------------------------------------- Accessors

    def snapshot(self) -> tuple[MemoryMessage, ...]:
        return tuple(self._messages)

    def __len__(self) -> int:
        return len(self._messages)

    @property
    def approx_token_count(self) -> int:
        return self._approx_tokens

    # ---------------------------------------------------------- Internal

    def _append(self, msg: MemoryMessage) -> None:
        self._messages.append(msg)
        self._approx_tokens += msg.approx_tokens
        self._compact()

    def _compact(self) -> None:
        """Drop oldest messages until we fit the caps.

        Rule: when we drop a non-tool message, any immediately-following
        tool messages become *orphans* (their prompting assistant is
        gone) and are dropped too. Providers reject orphan
        ``tool_result`` blocks, so keeping them around would poison the
        next request.
        """
        while (
            len(self._messages) > self.max_messages
            or self._approx_tokens > self.max_approx_tokens
        ) and self._messages:
            old = self._messages.popleft()
            self._approx_tokens -= old.approx_tokens
            if old.role != "tool":
                # Sweep trailing orphan tool messages.
                while self._messages and self._messages[0].role == "tool":
                    removed = self._messages.popleft()
                    self._approx_tokens -= removed.approx_tokens


# ---------------------------------------------------------------------------
#  Helpers
# ---------------------------------------------------------------------------


def _default_json(obj: Any) -> Any:
    # Dataclasses serialize through their ``__dict__`` if not already JSON-able.
    try:
        return obj.to_dict()
    except AttributeError:
        pass
    if hasattr(obj, "__dataclass_fields__"):
        from dataclasses import asdict

        return asdict(obj)
    return str(obj)


__all__ = ["ConversationMemory", "MemoryMessage", "Role"]
