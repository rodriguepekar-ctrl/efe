"""AnthropicProvider — Anthropic Messages API (M20).

Install : `pip install efe-agent[anthropic]`.

Auth : `ANTHROPIC_API_KEY` env-var or explicit `api_key=`.
"""
from __future__ import annotations

import asyncio
import os
import time
from typing import Any, AsyncIterator

from ..exceptions import (
    ProviderAuthError,
    ProviderError,
    ProviderRateLimitError,
    ProviderTimeoutError,
    ProviderUnavailableError,
)
from ..types import CostModel, LLMResponse, ProviderHealth
from .base import LLMProvider


# Pricing (USD per 1M tokens) — pinned to 2026 list prices ; bump in
# version 0.21.0 if Anthropic price-list changes.
_PRICING: dict[str, tuple[float, float]] = {
    "claude-opus-4-7": (15.00, 75.00),
    "claude-sonnet-4-5": (3.00, 15.00),
    "claude-haiku-4": (0.30, 1.50),
}

_MAX_CONTEXT: dict[str, int] = {
    "claude-opus-4-7": 1_000_000,
    "claude-sonnet-4-5": 200_000,
    "claude-haiku-4": 200_000,
}


class AnthropicProvider(LLMProvider):
    """Anthropic Messages API provider.

    Supports tools (Anthropic native tool_use blocks) + streaming.
    """

    def __init__(
        self,
        *,
        model: str = "claude-sonnet-4-5",
        api_key: str | None = None,
        base_url: str | None = None,
        client: Any | None = None,
    ) -> None:
        try:
            import anthropic  # type: ignore[import-not-found]
        except ImportError as e:
            raise ImportError(
                "AnthropicProvider requires the `anthropic` package. "
                "Install it via `pip install efe-agent[anthropic]`."
            ) from e
        self._anthropic_module = anthropic
        self._model = model
        if client is not None:
            self._client = client
        else:
            self._client = anthropic.AsyncAnthropic(
                api_key=api_key or os.environ.get("ANTHROPIC_API_KEY"),
                base_url=base_url,
            )

    # ─── Protocol surface ─────────────────────────────────────────

    @property
    def name(self) -> str:
        return f"anthropic:{self._model}"

    @property
    def supports_tools(self) -> bool:
        return True

    @property
    def supports_streaming(self) -> bool:
        return True

    @property
    def max_context_tokens(self) -> int:
        return _MAX_CONTEXT.get(self._model, 200_000)

    @property
    def cost_model(self) -> CostModel:
        i, o = _PRICING.get(self._model, (0.0, 0.0))
        return CostModel(
            input_per_1m_tokens_usd=i,
            output_per_1m_tokens_usd=o,
        )

    # ─── Behaviour ────────────────────────────────────────────────

    async def complete(
        self,
        *,
        system_prompt: str | None,
        messages: list[dict[str, Any]],
        tools: list[dict[str, Any]] | None = None,
        max_tokens: int = 1024,
        temperature: float = 0.0,
        seed: int | None = None,
        timeout_s: float = 30.0,
    ) -> LLMResponse:
        kwargs: dict[str, Any] = {
            "model": self._model,
            "messages": messages,
            "max_tokens": max_tokens,
            "temperature": temperature,
        }
        if system_prompt:
            kwargs["system"] = system_prompt
        if tools:
            kwargs["tools"] = tools

        try:
            resp = await asyncio.wait_for(
                self._client.messages.create(**kwargs),
                timeout=timeout_s,
            )
        except asyncio.TimeoutError as e:
            raise ProviderTimeoutError(
                f"timeout after {timeout_s}s",
                provider_name=self.name,
            ) from e
        except Exception as e:
            self._reraise_typed(e)

        text = ""
        tool_calls: list[dict] = []
        for block in resp.content:
            btype = getattr(block, "type", None)
            if btype == "text":
                text += block.text
            elif btype == "tool_use":
                tool_calls.append({
                    "id": block.id,
                    "name": block.name,
                    "input": block.input,
                })

        usage = getattr(resp, "usage", None)
        ti = int(getattr(usage, "input_tokens", 0) or 0)
        to = int(getattr(usage, "output_tokens", 0) or 0)

        return LLMResponse(
            text=text,
            tokens_in=ti,
            tokens_out=to,
            cost_usd=self._compute_cost(ti, to),
            finish_reason=self._map_stop_reason(getattr(resp, "stop_reason", None)),
            tool_calls=tool_calls or None,
            provider_name=self.name,
            model_name=self._model,
            raw_response=resp,
        )

    async def stream(
        self,
        *,
        system_prompt: str | None,
        messages: list[dict[str, Any]],
        tools: list[dict[str, Any]] | None = None,
        max_tokens: int = 1024,
        temperature: float = 0.0,
        timeout_s: float = 30.0,
    ) -> AsyncIterator[LLMResponse]:
        # M20 simplification : single yield with full response (no
        # token-by-token streaming exposed). Real streaming is a
        # M21+ feature.
        resp = await self.complete(
            system_prompt=system_prompt,
            messages=messages,
            tools=tools,
            max_tokens=max_tokens,
            temperature=temperature,
            timeout_s=timeout_s,
        )
        yield resp

    async def health(self) -> ProviderHealth:
        try:
            await asyncio.wait_for(
                self._client.messages.create(
                    model=self._model,
                    messages=[{"role": "user", "content": "ping"}],
                    max_tokens=10,
                ),
                timeout=10.0,
            )
            return ProviderHealth(
                available=True,
                last_check_ts=time.time(),
            )
        except Exception:  # noqa: BLE001
            return ProviderHealth(
                available=False,
                last_check_ts=time.time(),
            )

    # ─── Internals ────────────────────────────────────────────────

    def _compute_cost(self, tokens_in: int, tokens_out: int) -> float:
        cm = self.cost_model
        return (
            tokens_in * cm.input_per_1m_tokens_usd / 1_000_000
            + tokens_out * cm.output_per_1m_tokens_usd / 1_000_000
        )

    def _map_stop_reason(self, sr: str | None) -> str:
        return {
            "end_turn": "stop",
            "max_tokens": "length",
            "stop_sequence": "stop",
            "tool_use": "tool_use",
        }.get(sr or "", "stop")

    def _reraise_typed(self, e: Exception) -> None:
        """Translate Anthropic SDK exceptions to typed ProviderError subclasses."""
        a = self._anthropic_module
        if isinstance(e, getattr(a, "AuthenticationError", tuple())):
            raise ProviderAuthError(
                str(e), provider_name=self.name, status_code=401,
            ) from e
        if isinstance(e, getattr(a, "RateLimitError", tuple())):
            raise ProviderRateLimitError(
                str(e), provider_name=self.name, status_code=429,
            ) from e
        if isinstance(e, getattr(a, "APIStatusError", tuple())):
            sc = getattr(e, "status_code", None)
            if sc and 500 <= int(sc) < 600:
                raise ProviderUnavailableError(
                    str(e), provider_name=self.name, status_code=int(sc),
                ) from e
            raise ProviderError(
                str(e), provider_name=self.name, status_code=sc,
            ) from e
        raise ProviderError(str(e), provider_name=self.name) from e


__all__ = ["AnthropicProvider"]
