"""AzureOpenAIProvider — Azure OpenAI deployment (M20).

Install : `pip install efe-agent[azure-openai]`.

Auth options :
  1. Managed Identity / Workload Identity (preferred AKS) — via
     `azure.identity.DefaultAzureCredential`.
  2. API key (`AZURE_OPENAI_API_KEY`).
  3. Service Principal (`AZURE_CLIENT_ID` / `AZURE_TENANT_ID` /
     `AZURE_CLIENT_SECRET`).
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


_DEFAULT_COST = CostModel(
    input_per_1m_tokens_usd=10.00,  # GPT-5 list price
    output_per_1m_tokens_usd=40.00,
)


class AzureOpenAIProvider(LLMProvider):
    """Azure OpenAI provider."""

    def __init__(
        self,
        *,
        deployment_name: str,
        endpoint: str,
        api_version: str = "2024-08-01-preview",
        api_key: str | None = None,
        cost_override: CostModel | None = None,
        max_context_tokens: int = 128_000,
        client: Any | None = None,
    ) -> None:
        try:
            import openai  # type: ignore[import-not-found]
        except ImportError as e:
            raise ImportError(
                "AzureOpenAIProvider requires the `openai` package. "
                "Install via `pip install efe-agent[azure-openai]`."
            ) from e
        self._openai_module = openai
        self._deployment = deployment_name
        self._cost = cost_override or _DEFAULT_COST
        self._max_context = max_context_tokens

        if client is not None:
            self._client = client
        elif api_key or os.environ.get("AZURE_OPENAI_API_KEY"):
            self._client = openai.AsyncAzureOpenAI(
                api_key=api_key or os.environ["AZURE_OPENAI_API_KEY"],
                api_version=api_version,
                azure_endpoint=endpoint,
            )
        else:
            try:
                from azure.identity import DefaultAzureCredential
                from azure.identity.aio import (
                    DefaultAzureCredential as AsyncDAC,
                )
            except ImportError as e:
                raise ImportError(
                    "AzureOpenAIProvider with Managed Identity requires "
                    "azure-identity. Install via "
                    "`pip install efe-agent[azure-openai]`."
                ) from e
            credential = AsyncDAC()

            async def _token_provider() -> str:
                token = await credential.get_token(
                    "https://cognitiveservices.azure.com/.default"
                )
                return token.token

            self._client = openai.AsyncAzureOpenAI(
                azure_ad_token_provider=_token_provider,
                api_version=api_version,
                azure_endpoint=endpoint,
            )

    @property
    def name(self) -> str:
        return f"azure-openai:{self._deployment}"

    @property
    def supports_tools(self) -> bool:
        return True

    @property
    def supports_streaming(self) -> bool:
        return True

    @property
    def max_context_tokens(self) -> int:
        return self._max_context

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
        msgs: list[dict] = []
        if system_prompt:
            msgs.append({"role": "system", "content": system_prompt})
        msgs.extend(messages)

        kwargs: dict[str, Any] = {
            "model": self._deployment,
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
                f"timeout after {timeout_s}s", provider_name=self.name,
            ) from e
        except Exception as e:
            self._reraise_typed(e)

        if not resp.choices:
            raise ProviderError(
                "no choices in response", provider_name=self.name,
            )
        choice = resp.choices[0]
        usage = getattr(resp, "usage", None)
        ti = int(getattr(usage, "prompt_tokens", 0) or 0)
        to = int(getattr(usage, "completion_tokens", 0) or 0)

        return LLMResponse(
            text=choice.message.content or "",
            tokens_in=ti,
            tokens_out=to,
            cost_usd=(
                ti * self._cost.input_per_1m_tokens_usd / 1_000_000
                + to * self._cost.output_per_1m_tokens_usd / 1_000_000
            ),
            finish_reason={
                "stop": "stop", "length": "length",
                "tool_calls": "tool_use", "content_filter": "safety",
            }.get(choice.finish_reason or "", "stop"),
            tool_calls=[
                {"id": tc.id, "name": tc.function.name,
                 "input": tc.function.arguments}
                for tc in (choice.message.tool_calls or [])
            ] or None,
            provider_name=self.name,
            model_name=self._deployment,
            raw_response=resp,
        )

    async def stream(self, **kwargs: Any) -> AsyncIterator[LLMResponse]:
        resp = await self.complete(**kwargs)
        yield resp

    async def health(self) -> ProviderHealth:
        try:
            await asyncio.wait_for(
                self._client.chat.completions.create(
                    model=self._deployment,
                    messages=[{"role": "user", "content": "ping"}],
                    max_completion_tokens=10,
                ),
                timeout=10.0,
            )
            return ProviderHealth(available=True, last_check_ts=time.time())
        except Exception:  # noqa: BLE001
            return ProviderHealth(available=False, last_check_ts=time.time())

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


__all__ = ["AzureOpenAIProvider"]
