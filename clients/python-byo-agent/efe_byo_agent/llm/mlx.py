"""MLXProvider — Apple MLX (M20).

Local inference on Apple Silicon (M-series). Useful for dev /
operator workstation without API access.

Install : `pip install efe-agent[mlx]` (Mac only).
"""
from __future__ import annotations

import asyncio
import time
from typing import Any, AsyncIterator

from ..exceptions import ProviderError
from ..types import CostModel, LLMResponse, ProviderHealth
from .base import LLMProvider


_FREE_COST = CostModel(
    input_per_1m_tokens_usd=0.0,
    output_per_1m_tokens_usd=0.0,
)


class MLXProvider(LLMProvider):
    """Apple MLX local inference provider.

    M20 limitations :
      - No native tool support (`supports_tools=False`).
      - Token counting is approximate (chars ÷ 4).
      - Streaming yields a single full response (M21+ for token streams).
    """

    def __init__(
        self,
        *,
        model_path: str,
        max_context_tokens: int = 8192,
    ) -> None:
        try:
            from mlx_lm import generate, load  # type: ignore[import-not-found]
        except ImportError as e:
            raise ImportError(
                "MLXProvider requires `mlx-lm`. "
                "Install via `pip install efe-agent[mlx]` (Mac M-series only)."
            ) from e
        self._load = load
        self._generate = generate
        self._model_path = model_path
        self._model, self._tokenizer = load(model_path)
        self._max_context = max_context_tokens

    @property
    def name(self) -> str:
        return f"mlx:{self._model_path}"

    @property
    def supports_tools(self) -> bool:
        return False

    @property
    def supports_streaming(self) -> bool:
        return False

    @property
    def max_context_tokens(self) -> int:
        return self._max_context

    @property
    def cost_model(self) -> CostModel:
        return _FREE_COST

    async def complete(
        self,
        *,
        system_prompt: str | None,
        messages: list[dict[str, Any]],
        tools: list[dict[str, Any]] | None = None,
        max_tokens: int = 1024,
        temperature: float = 0.0,
        seed: int | None = None,
        timeout_s: float = 60.0,
    ) -> LLMResponse:
        if tools:
            raise ProviderError(
                "MLXProvider does not support tools in M20",
                provider_name=self.name,
            )

        # Build single concat prompt (MLX is text-completion style).
        prompt_parts: list[str] = []
        if system_prompt:
            prompt_parts.append(f"System: {system_prompt}")
        for m in messages:
            role = m.get("role", "user")
            content = m.get("content", "")
            if isinstance(content, list):
                content = "".join(
                    b.get("text", "") if isinstance(b, dict) else str(b)
                    for b in content
                )
            prompt_parts.append(f"{role.capitalize()}: {content}")
        prompt_parts.append("Assistant:")
        prompt = "\n".join(prompt_parts)

        try:
            text = await asyncio.wait_for(
                asyncio.to_thread(
                    self._generate,
                    self._model,
                    self._tokenizer,
                    prompt,
                    max_tokens=max_tokens,
                ),
                timeout=timeout_s,
            )
        except asyncio.TimeoutError:
            raise
        except Exception as e:
            raise ProviderError(
                f"MLX generation failed : {e}",
                provider_name=self.name,
            ) from e

        # Strip the prompt prefix if echoed
        if text.startswith(prompt):
            text = text[len(prompt):]

        # Approximate token counting.
        ti_approx = len(prompt) // 4
        to_approx = len(text) // 4

        return LLMResponse(
            text=text.strip(),
            tokens_in=ti_approx,
            tokens_out=to_approx,
            cost_usd=0.0,
            finish_reason="stop",
            provider_name=self.name,
            model_name=self._model_path,
        )

    async def stream(self, **kwargs: Any) -> AsyncIterator[LLMResponse]:
        resp = await self.complete(**kwargs)
        yield resp

    async def health(self) -> ProviderHealth:
        # MLX is local : if the model loaded, we're healthy.
        return ProviderHealth(available=True, last_check_ts=time.time())


__all__ = ["MLXProvider"]
