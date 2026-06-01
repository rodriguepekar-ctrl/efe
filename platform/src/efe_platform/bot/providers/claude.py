"""Anthropic Claude provider — wraps ``anthropic.Messages`` API.

Optional dependency: ``pip install efe-platform[claude]``. The import
of ``anthropic`` itself is inside the constructor so importing this
module does not fail when the SDK is absent.
"""

from __future__ import annotations

import os
import uuid
from collections.abc import Mapping, Sequence
from typing import Any

from efe_platform.bot.planner import LlmTurn, PlannedCall
from efe_platform.bot.tool_registry import ToolSchema
from efe_platform.errors import LlmProviderUnavailable


class ClaudeProvider:
    """Anthropic Claude wrapper."""

    def __init__(
        self,
        *,
        model: str = "claude-3-5-sonnet-latest",
        temperature: float = 0.1,
        api_key: str | None = None,
        price_per_input_token_usd: float = 3.0 / 1_000_000,
        price_per_output_token_usd: float = 15.0 / 1_000_000,
    ) -> None:
        try:
            import anthropic
        except ImportError as exc:  # pragma: no cover — optional dep
            raise LlmProviderUnavailable(
                "anthropic SDK not installed. Install with `pip install "
                "efe-platform[claude]`."
            ) from exc
        key = api_key or os.environ.get("ANTHROPIC_API_KEY")
        if not key:
            raise LlmProviderUnavailable("ANTHROPIC_API_KEY is not set")
        self._client = anthropic.AsyncAnthropic(api_key=key)
        self._model = model
        self._temperature = temperature
        self._price_in = price_per_input_token_usd
        self._price_out = price_per_output_token_usd

    async def run(
        self,
        *,
        phase: str,
        system: str,
        messages: Sequence[Mapping[str, Any]],
        tool_schemas: Sequence[ToolSchema],
        max_tokens: int,
    ) -> LlmTurn:
        payload_messages = _translate_messages_anthropic(messages)
        kwargs: dict[str, Any] = {
            "model": self._model,
            "system": system,
            "messages": payload_messages,
            "max_tokens": max_tokens,
            "temperature": self._temperature,
        }
        if tool_schemas:
            kwargs["tools"] = _convert_schemas_anthropic(tool_schemas)
        resp = await self._client.messages.create(**kwargs)

        text_parts: list[str] = []
        tool_uses: list[PlannedCall] = []
        for block in resp.content:
            btype = getattr(block, "type", None)
            if btype == "text":
                text_parts.append(getattr(block, "text", ""))
            elif btype == "tool_use":
                tool_uses.append(
                    PlannedCall(
                        tool_use_id=getattr(block, "id", str(uuid.uuid4())),
                        tool_name=getattr(block, "name", ""),
                        arguments=dict(getattr(block, "input", {})),
                    )
                )
        usage = getattr(resp, "usage", None)
        tokens_in = getattr(usage, "input_tokens", 0) if usage else 0
        tokens_out = getattr(usage, "output_tokens", 0) if usage else 0
        total_tokens = tokens_in + tokens_out
        usd = tokens_in * self._price_in + tokens_out * self._price_out
        return LlmTurn(
            text="".join(text_parts),
            tool_uses=tuple(tool_uses),
            tokens_used=total_tokens,
            usd_used=usd,
            finish_reason=str(getattr(resp, "stop_reason", "stop")),
        )


# ---------------------------------------------------------------------------
#  Schema translation
# ---------------------------------------------------------------------------


def _convert_schemas_anthropic(
    schemas: Sequence[ToolSchema],
) -> list[dict[str, Any]]:
    """Anthropic wants ``{name, description, input_schema}``."""
    return [
        {
            "name": s.name,
            "description": s.description,
            "input_schema": dict(s.input_schema),
        }
        for s in schemas
    ]


# ---------------------------------------------------------------------------
#  Message translation
# ---------------------------------------------------------------------------


def _translate_messages_anthropic(
    messages: Sequence[Mapping[str, Any]],
) -> list[dict[str, Any]]:
    """Translate our neutral message shape to Anthropic's content-block schema."""
    out: list[dict[str, Any]] = []
    # We batch consecutive tool messages into a single user turn with
    # tool_result blocks, as required by Anthropic's API.
    i = 0
    while i < len(messages):
        m = messages[i]
        role = m["role"]
        if role == "user":
            out.append({"role": "user", "content": [{"type": "text", "text": m["content"]}]})
            i += 1
        elif role == "assistant":
            out.append(
                {
                    "role": "assistant",
                    "content": [{"type": "text", "text": m["content"]}],
                }
            )
            i += 1
        elif role == "tool":
            blocks: list[dict[str, Any]] = []
            while i < len(messages) and messages[i]["role"] == "tool":
                t = messages[i]
                blocks.append(
                    {
                        "type": "tool_result",
                        "tool_use_id": t["tool_use_id"],
                        "content": t["content"],
                    }
                )
                i += 1
            out.append({"role": "user", "content": blocks})
        else:  # pragma: no cover — defensive
            raise ValueError(f"unknown role {role}")
    return out


__all__ = ["ClaudeProvider"]
