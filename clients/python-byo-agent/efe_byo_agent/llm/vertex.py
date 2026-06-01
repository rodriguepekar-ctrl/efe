"""VertexProvider — Google Vertex AI Gemini (M20).

Install : `pip install efe-agent[vertex]`.

Auth :
  1. Workload Identity (preferred GKE).
  2. `GOOGLE_APPLICATION_CREDENTIALS` env-var pointing to SA JSON.
  3. `gcloud auth application-default login` (dev).
"""
from __future__ import annotations

import asyncio
import os
import time
from typing import Any, AsyncIterator

from ..exceptions import (
    ProviderAuthError,
    ProviderError,
    ProviderTimeoutError,
    ProviderUnavailableError,
)
from ..types import CostModel, LLMResponse, ProviderHealth
from .base import LLMProvider


_PRICING: dict[str, tuple[float, float]] = {
    "gemini-2.5-pro": (1.25, 10.00),
    "gemini-2.5-flash": (0.075, 0.30),
}

_MAX_CONTEXT = {
    "gemini-2.5-pro": 2_000_000,
    "gemini-2.5-flash": 1_000_000,
}


class VertexProvider(LLMProvider):
    """Vertex AI Gemini provider."""

    def __init__(
        self,
        *,
        model: str = "gemini-2.5-flash",
        project: str | None = None,
        location: str = "us-central1",
        cost_override: CostModel | None = None,
    ) -> None:
        try:
            import vertexai  # type: ignore[import-not-found]
            from vertexai.generative_models import GenerativeModel  # type: ignore[import-not-found]
        except ImportError as e:
            raise ImportError(
                "VertexProvider requires `google-cloud-aiplatform`. "
                "Install via `pip install efe-agent[vertex]`."
            ) from e

        proj = project or os.environ.get("GOOGLE_CLOUD_PROJECT")
        if not proj:
            raise ValueError(
                "VertexProvider requires `project` arg or "
                "GOOGLE_CLOUD_PROJECT env-var."
            )
        vertexai.init(project=proj, location=location)
        self._model_name = model
        self._model = GenerativeModel(model)
        self._cost = cost_override or self._default_cost(model)

    @property
    def name(self) -> str:
        return f"vertex:{self._model_name}"

    @property
    def supports_tools(self) -> bool:
        return True

    @property
    def supports_streaming(self) -> bool:
        return True

    @property
    def max_context_tokens(self) -> int:
        return _MAX_CONTEXT.get(self._model_name, 1_000_000)

    @property
    def cost_model(self) -> CostModel:
        return self._cost

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
        # Vertex SDK is sync ; wrap.
        loop = asyncio.get_running_loop()
        prompt_parts = self._convert_messages(messages, system_prompt)

        try:
            resp = await asyncio.wait_for(
                loop.run_in_executor(
                    None,
                    lambda: self._model.generate_content(
                        prompt_parts,
                        generation_config={
                            "max_output_tokens": max_tokens,
                            "temperature": temperature,
                        },
                    ),
                ),
                timeout=timeout_s,
            )
        except asyncio.TimeoutError as e:
            raise ProviderTimeoutError(
                f"timeout after {timeout_s}s", provider_name=self.name,
            ) from e
        except Exception as e:
            self._reraise_typed(e)

        text = resp.text if hasattr(resp, "text") else ""
        usage = getattr(resp, "usage_metadata", None)
        ti = int(getattr(usage, "prompt_token_count", 0) or 0)
        to = int(getattr(usage, "candidates_token_count", 0) or 0)

        return LLMResponse(
            text=text,
            tokens_in=ti,
            tokens_out=to,
            cost_usd=(
                ti * self._cost.input_per_1m_tokens_usd / 1_000_000
                + to * self._cost.output_per_1m_tokens_usd / 1_000_000
            ),
            finish_reason="stop",
            provider_name=self.name,
            model_name=self._model_name,
            raw_response=resp,
        )

    async def stream(self, **kwargs: Any) -> AsyncIterator[LLMResponse]:
        resp = await self.complete(**kwargs)
        yield resp

    async def health(self) -> ProviderHealth:
        try:
            await self.complete(
                system_prompt=None,
                messages=[{"role": "user", "content": "ping"}],
                max_tokens=10,
                timeout_s=10.0,
            )
            return ProviderHealth(available=True, last_check_ts=time.time())
        except Exception:  # noqa: BLE001
            return ProviderHealth(available=False, last_check_ts=time.time())

    def _convert_messages(
        self,
        messages: list[dict[str, Any]],
        system_prompt: str | None,
    ) -> list[str]:
        parts: list[str] = []
        if system_prompt:
            parts.append(f"System: {system_prompt}")
        for m in messages:
            role = m.get("role", "user")
            content = m.get("content", "")
            if isinstance(content, list):
                content = "".join(
                    b.get("text", "") if isinstance(b, dict) else str(b)
                    for b in content
                )
            parts.append(f"{role.capitalize()}: {content}")
        return parts

    def _default_cost(self, model: str) -> CostModel:
        i, o = _PRICING.get(model, (0.0, 0.0))
        return CostModel(
            input_per_1m_tokens_usd=i,
            output_per_1m_tokens_usd=o,
        )

    def _reraise_typed(self, e: Exception) -> None:
        # Vertex AI raises various exception types ; we map by message
        # heuristics since the SDK doesn't expose a uniform hierarchy.
        msg = str(e).lower()
        if "permission" in msg or "unauthorized" in msg or "401" in msg:
            raise ProviderAuthError(str(e), provider_name=self.name) from e
        if "503" in msg or "unavailable" in msg:
            raise ProviderUnavailableError(
                str(e), provider_name=self.name, status_code=503,
            ) from e
        raise ProviderError(str(e), provider_name=self.name) from e


__all__ = ["VertexProvider"]
