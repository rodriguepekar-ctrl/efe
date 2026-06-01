"""LlamaCppProvider — local llama.cpp HTTP server (M20).

Run :
  llama-server -m model.gguf --port 8080

Then :
  LlamaCppProvider(server_url="http://localhost:8080/v1", model="local")

OpenAI-compatibility delegated to OpenAIProvider with a custom base_url.
"""
from __future__ import annotations

from typing import Any, AsyncIterator

from ..types import CostModel, LLMResponse, ProviderHealth
from .base import LLMProvider
from .openai import OpenAIProvider


_FREE_COST = CostModel(
    input_per_1m_tokens_usd=0.0,
    output_per_1m_tokens_usd=0.0,
)


class LlamaCppProvider(LLMProvider):
    """llama.cpp local HTTP server (OpenAI-compatible)."""

    def __init__(
        self,
        *,
        server_url: str = "http://localhost:8080/v1",
        model: str = "local-model",
        api_key: str = "sk-no-key-required",
        max_context_tokens: int = 8192,
    ) -> None:
        self._inner = OpenAIProvider(
            model=model,
            api_key=api_key,
            base_url=server_url,
            cost_override=_FREE_COST,
            max_context_override=max_context_tokens,
            name_override=f"llama-cpp:{model}",
        )

    @property
    def name(self) -> str:
        return self._inner.name

    @property
    def supports_tools(self) -> bool:
        # llama.cpp tool support varies by build ; we expose conservative
        # default of False for M20. Override after testing your build.
        return False

    @property
    def supports_streaming(self) -> bool:
        return self._inner.supports_streaming

    @property
    def max_context_tokens(self) -> int:
        return self._inner.max_context_tokens

    @property
    def cost_model(self) -> CostModel:
        return self._inner.cost_model

    async def complete(self, **kwargs: Any) -> LLMResponse:
        return await self._inner.complete(**kwargs)

    async def stream(self, **kwargs: Any) -> AsyncIterator[LLMResponse]:
        async for r in self._inner.stream(**kwargs):
            yield r

    async def health(self) -> ProviderHealth:
        return await self._inner.health()


__all__ = ["LlamaCppProvider"]
