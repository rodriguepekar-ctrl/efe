"""OpenAIProvider — OpenAI Chat Completions (M20).

Install : `pip install efe-agent[openai]`.

Auth : `OPENAI_API_KEY` env-var or explicit `api_key=`.

Also re-used by VLLMProvider and LlamaCppProvider via custom `base_url`
since vLLM and llama.cpp expose OpenAI-compatible HTTP APIs.
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


_PRICING: dict[str, tuple[float, float]] = {
    "gpt-5": (10.00, 40.00),
    "gpt-5-mini": (0.30, 1.20),
    "gpt-4o": (2.50, 10.00),
    "gpt-4o-mini": (0.15, 0.60),
}

_MAX_CONTEXT: dict[str, int] = {
    "gpt-5": 256_000,
    "gpt-5-mini": 128_000,
    "gpt-4o": 128_000,
    "gpt-4o-mini": 128_000,
}


class OpenAIProvider(LLMProvider):
    """OpenAI Chat Completions provider (also used for vLLM / llama.cpp)."""

    def __init__(
        self,
        *,
        model: str = "gpt-5-mini",
        api_key: str | None = None,
        base_url: str | None = None,
        client: Any | None = None,
        cost_override: CostModel | None = None,
        max_context_override: int | None = None,
        name_override: str | None = None,
    ) -> None:
        try:
            import openai  # type: ignore[import-not-found]
        except ImportError as e:
            raise ImportError(
                "OpenAIProvider requires the `openai` package. "
                "Install it via `pip install efe-agent[openai]`."
            ) from e
        self._openai_module = openai
        self._model = model
        self._cost_override = cost_override
        self._max_context_override = max_context_override
        self._name_override = name_override
        if client is not None:
            self._client = client
        else:
            self._client = openai.AsyncOpenAI(
                api_key=api_key or os.environ.get("OPENAI_API_KEY", "EMPTY"),
                base_url=base_url,
            )

    @property
    def name(self) -> str:
        if self._name_override:
            return self._name_override
        return f"openai:{self._model}"

    @property
    def supports_tools(self) -> bool:
        return True

    @property
    def supports_streaming(self) -> bool:
        return True

    @property
    def max_context_tokens(self) -> int:
        if self._max_context_override:
            return self._max_context_override
        return _MAX_CONTEXT.get(self._model, 128_000)

    @property
    def cost_model(self) -> CostModel:
        if self._cost_override:
            return self._cost_override
        i, o = _PRICING.get(self._model, (0.0, 0.0))
        return CostModel(
            input_per_1m_tokens_usd=i,
            output_per_1m_tokens_usd=o,
        )

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
        msgs: list[dict] = []
        if system_prompt:
            msgs.append({"role": "system", "content": system_prompt})
        msgs.extend(messages)

        kwargs: dict[str, Any] = {
            "model": self._model,
            "messages": msgs,
            "max_completion_tokens": max_tokens,
            "temperature": temperature,
        }
        if tools:
            kwargs["tools"] = tools
        if seed is not None:
            kwargs["seed"] = seed

        try:
            resp = await asyncio.wait_for(
                self._client.chat.completions.create(**kwargs),
                timeout=timeout_s,
            )
        except asyncio.TimeoutError as e:
            raise ProviderTimeoutError(
                f"timeout after {timeout_s}s",
                provider_name=self.name,
            ) from e
        except Exception as e:
            self._reraise_typed(e)

        if not resp.choices:
            raise ProviderError(
                "no choices in OpenAI response",
                provider_name=self.name,
            )
        choice = resp.choices[0]

        usage = getattr(resp, "usage", None)
        ti = int(getattr(usage, "prompt_tokens", 0) or 0)
        to = int(getattr(usage, "completion_tokens", 0) or 0)

        tool_calls = []
        for tc in getattr(choice.message, "tool_calls", None) or []:
            tool_calls.append({
                "id": tc.id,
                "name": tc.function.name,
                "input": tc.function.arguments,
            })

        return LLMResponse(
            text=choice.message.content or "",
            tokens_in=ti,
            tokens_out=to,
            cost_usd=self._compute_cost(ti, to),
            finish_reason=self._map_finish_reason(choice.finish_reason),
            tool_calls=tool_calls or None,
            provider_name=self.name,
            model_name=self._model,
            raw_response=resp,
        )

    async def stream(self, **kwargs: Any) -> AsyncIterator[LLMResponse]:
        resp = await self.complete(**kwargs)
        yield resp

    async def health(self) -> ProviderHealth:
        try:
            await asyncio.wait_for(
                self._client.chat.completions.create(
                    model=self._model,
                    messages=[{"role": "user", "content": "ping"}],
                    max_completion_tokens=10,
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

    def _compute_cost(self, ti: int, to: int) -> float:
        cm = self.cost_model
        return (
            ti * cm.input_per_1m_tokens_usd / 1_000_000
            + to * cm.output_per_1m_tokens_usd / 1_000_000
        )

    def _map_finish_reason(self, fr: str | None) -> str:
        return {
            "stop": "stop",
            "length": "length",
            "tool_calls": "tool_use",
            "function_call": "tool_use",
            "content_filter": "safety",
        }.get(fr or "", "stop")

    def _reraise_typed(self, e: Exception) -> None:
        m = self._openai_module
        if isinstance(e, getattr(m, "AuthenticationError", tuple())):
            raise ProviderAuthError(
                str(e), provider_name=self.name, status_code=401,
            ) from e
        if isinstance(e, getattr(m, "RateLimitError", tuple())):
            raise ProviderRateLimitError(
                str(e), provider_name=self.name, status_code=429,
            ) from e
        if isinstance(e, getattr(m, "APIStatusError", tuple())):
            sc = getattr(e, "status_code", None)
            if sc and 500 <= int(sc) < 600:
                raise ProviderUnavailableError(
                    str(e), provider_name=self.name, status_code=int(sc),
                ) from e
            raise ProviderError(
                str(e), provider_name=self.name, status_code=sc,
            ) from e
        raise ProviderError(str(e), provider_name=self.name) from e


__all__ = ["OpenAIProvider"]
