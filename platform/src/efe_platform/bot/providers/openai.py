"""OpenAI provider — wraps ``openai.chat.completions``.

Optional dependency: ``pip install efe-platform[openai]``. The import
of ``openai`` lives inside the constructor, same reason as the Claude
provider.
"""

from __future__ import annotations

import json
import os
import uuid
from collections.abc import Mapping, Sequence
from typing import Any

from efe_platform.bot.planner import LlmTurn, PlannedCall
from efe_platform.bot.tool_registry import ToolSchema
from efe_platform.errors import LlmProviderUnavailable


class OpenAIProvider:
    """OpenAI Chat Completions wrapper using function calling."""

    def __init__(
        self,
        *,
        model: str = "gpt-4o-mini",
        temperature: float = 0.1,
        api_key: str | None = None,
        price_per_input_token_usd: float = 0.15 / 1_000_000,
        price_per_output_token_usd: float = 0.60 / 1_000_000,
    ) -> None:
        try:
            import openai
        except ImportError as exc:  # pragma: no cover
            raise LlmProviderUnavailable(
                "openai SDK not installed. Install with `pip install "
                "efe-platform[openai]`."
            ) from exc
        key = api_key or os.environ.get("OPENAI_API_KEY")
        if not key:
            raise LlmProviderUnavailable("OPENAI_API_KEY is not set")
        self._client = openai.AsyncOpenAI(api_key=key)
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
        payload_messages = _translate_messages_openai(system, messages)
        kwargs: dict[str, Any] = {
            "model": self._model,
            "messages": payload_messages,
            "max_tokens": max_tokens,
            "temperature": self._temperature,
        }
        if tool_schemas:
            kwargs["tools"] = _convert_schemas_openai(tool_schemas)
        resp = await self._client.chat.completions.create(**kwargs)

        choice = resp.choices[0]
        message = choice.message
        text = (message.content or "") if hasattr(message, "content") else ""
        tool_uses: list[PlannedCall] = []
        for call in (message.tool_calls or []) if hasattr(message, "tool_calls") else []:
            fn = getattr(call, "function", None)
            args_raw = getattr(fn, "arguments", "{}") if fn else "{}"
            try:
                args = json.loads(args_raw) if args_raw else {}
            except json.JSONDecodeError:
                args = {}
            tool_uses.append(
                PlannedCall(
                    tool_use_id=str(getattr(call, "id", uuid.uuid4())),
                    tool_name=str(getattr(fn, "name", "")),
                    arguments=args,
                )
            )
        usage = getattr(resp, "usage", None)
        tokens_in = getattr(usage, "prompt_tokens", 0) if usage else 0
        tokens_out = getattr(usage, "completion_tokens", 0) if usage else 0
        total = tokens_in + tokens_out
        usd = tokens_in * self._price_in + tokens_out * self._price_out
        return LlmTurn(
            text=text,
            tool_uses=tuple(tool_uses),
            tokens_used=total,
            usd_used=usd,
            finish_reason=str(getattr(choice, "finish_reason", "stop")),
        )


# ---------------------------------------------------------------------------
#  Schema translation
# ---------------------------------------------------------------------------


def _convert_schemas_openai(
    schemas: Sequence[ToolSchema],
) -> list[dict[str, Any]]:
    """OpenAI wants ``{type: "function", function: {name, description, parameters}}``."""
    return [
        {
            "type": "function",
            "function": {
                "name": s.name,
                "description": s.description,
                "parameters": dict(s.input_schema),
            },
        }
        for s in schemas
    ]


# ---------------------------------------------------------------------------
#  Message translation
# ---------------------------------------------------------------------------


def _translate_messages_openai(
    system: str, messages: Sequence[Mapping[str, Any]]
) -> list[dict[str, Any]]:
    out: list[dict[str, Any]] = [{"role": "system", "content": system}]
    for m in messages:
        role = m["role"]
        if role == "user":
            out.append({"role": "user", "content": m["content"]})
        elif role == "assistant":
            out.append({"role": "assistant", "content": m["content"]})
        elif role == "tool":
            out.append(
                {
                    "role": "tool",
                    "tool_call_id": m["tool_use_id"],
                    "name": m["tool_name"],
                    "content": m["content"],
                }
            )
    return out


__all__ = ["OpenAIProvider"]
