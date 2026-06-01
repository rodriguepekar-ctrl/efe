"""BedrockProvider — AWS Bedrock Runtime (M20).

Install : `pip install efe-agent[bedrock]`.

Auth chain (M19 BYOK carry-over) :
  1. IRSA (EKS) — preferred prod.
  2. EC2 instance role.
  3. AWS_PROFILE env.
  4. Static AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY.

Uses the Converse API (`bedrock-runtime.converse`) which works
uniformly for Claude / Llama / Titan models hosted on Bedrock.
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


# Bedrock pricing varies per model + region. Caller can override via
# cost_override. Defaults are best-effort 2026 list prices for
# Anthropic Claude Sonnet 4.5 us-east-1.
_DEFAULT_COST = CostModel(
    input_per_1m_tokens_usd=3.00,
    output_per_1m_tokens_usd=15.00,
)


class BedrockProvider(LLMProvider):
    """AWS Bedrock provider via the Converse API."""

    def __init__(
        self,
        *,
        model_id: str = "anthropic.claude-sonnet-4-5-v1:0",
        region: str | None = None,
        cost_override: CostModel | None = None,
        max_context_tokens: int = 200_000,
        client: Any | None = None,
    ) -> None:
        try:
            import boto3  # type: ignore[import-not-found]
        except ImportError as e:
            raise ImportError(
                "BedrockProvider requires the `boto3` package. "
                "Install it via `pip install efe-agent[bedrock]`."
            ) from e
        self._boto3_module = boto3
        self._model_id = model_id
        self._cost = cost_override or _DEFAULT_COST
        self._max_context = max_context_tokens
        self._region = (
            region or os.environ.get("AWS_REGION", "us-east-1")
        )
        self._client = client or boto3.client(
            "bedrock-runtime",
            region_name=self._region,
        )

    @property
    def name(self) -> str:
        return f"bedrock:{self._model_id}"

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
        # boto3 is sync ; wrap in run_in_executor.
        bedrock_messages = self._convert_messages(messages)
        kwargs = {
            "modelId": self._model_id,
            "messages": bedrock_messages,
            "inferenceConfig": {
                "maxTokens": max_tokens,
                "temperature": temperature,
            },
        }
        if system_prompt:
            kwargs["system"] = [{"text": system_prompt}]
        if tools:
            kwargs["toolConfig"] = {
                "tools": [{"toolSpec": t} for t in tools],
            }

        loop = asyncio.get_running_loop()
        try:
            resp = await asyncio.wait_for(
                loop.run_in_executor(None, lambda: self._client.converse(**kwargs)),
                timeout=timeout_s,
            )
        except asyncio.TimeoutError as e:
            raise ProviderTimeoutError(
                f"timeout after {timeout_s}s", provider_name=self.name,
            ) from e
        except Exception as e:
            self._reraise_typed(e)

        text = ""
        tool_calls: list[dict] = []
        for block in resp["output"]["message"]["content"]:
            if "text" in block:
                text += block["text"]
            elif "toolUse" in block:
                tu = block["toolUse"]
                tool_calls.append({
                    "id": tu.get("toolUseId"),
                    "name": tu["name"],
                    "input": tu.get("input"),
                })

        usage = resp.get("usage") or {}
        ti = int(usage.get("inputTokens", 0))
        to = int(usage.get("outputTokens", 0))

        return LLMResponse(
            text=text,
            tokens_in=ti,
            tokens_out=to,
            cost_usd=(
                ti * self._cost.input_per_1m_tokens_usd / 1_000_000
                + to * self._cost.output_per_1m_tokens_usd / 1_000_000
            ),
            finish_reason=self._map_stop_reason(resp.get("stopReason")),
            tool_calls=tool_calls or None,
            provider_name=self.name,
            model_name=self._model_id,
            raw_response=resp,
        )

    async def stream(self, **kwargs: Any) -> AsyncIterator[LLMResponse]:
        # M20 simplification : single-yield wrapper.
        resp = await self.complete(**kwargs)
        yield resp

    async def health(self) -> ProviderHealth:
        try:
            loop = asyncio.get_running_loop()
            await asyncio.wait_for(
                loop.run_in_executor(
                    None,
                    lambda: self._client.converse(
                        modelId=self._model_id,
                        messages=[{
                            "role": "user",
                            "content": [{"text": "ping"}],
                        }],
                        inferenceConfig={"maxTokens": 10},
                    ),
                ),
                timeout=10.0,
            )
            return ProviderHealth(available=True, last_check_ts=time.time())
        except Exception:  # noqa: BLE001
            return ProviderHealth(available=False, last_check_ts=time.time())

    def _convert_messages(
        self, messages: list[dict[str, Any]],
    ) -> list[dict[str, Any]]:
        """Translate Anthropic-style messages → Bedrock Converse format."""
        out = []
        for m in messages:
            content = m.get("content")
            if isinstance(content, str):
                blocks = [{"text": content}]
            elif isinstance(content, list):
                blocks = []
                for b in content:
                    if isinstance(b, str):
                        blocks.append({"text": b})
                    elif isinstance(b, dict) and b.get("type") == "text":
                        blocks.append({"text": b["text"]})
                    elif isinstance(b, dict):
                        blocks.append(b)
            else:
                blocks = [{"text": str(content)}]
            out.append({"role": m.get("role", "user"), "content": blocks})
        return out

    def _map_stop_reason(self, sr: str | None) -> str:
        return {
            "end_turn": "stop",
            "max_tokens": "length",
            "tool_use": "tool_use",
            "stop_sequence": "stop",
            "content_filtered": "safety",
            "guardrail_intervened": "safety",
        }.get(sr or "", "stop")

    def _reraise_typed(self, e: Exception) -> None:
        boto3 = self._boto3_module
        ClientError = getattr(boto3.exceptions, "Boto3Error", Exception)  # noqa: N806
        # botocore raises ClientError, which has .response['Error']['Code']
        try:
            from botocore.exceptions import ClientError as BotoClientError
        except ImportError:  # pragma: no cover
            BotoClientError = None
        if BotoClientError is not None and isinstance(e, BotoClientError):
            err = e.response.get("Error", {}) if hasattr(e, "response") else {}
            code = err.get("Code", "")
            sc = e.response.get("ResponseMetadata", {}).get("HTTPStatusCode")
            if code in ("AccessDeniedException", "UnauthorizedException"):
                raise ProviderAuthError(
                    str(e), provider_name=self.name, status_code=sc,
                ) from e
            if code == "ThrottlingException":
                raise ProviderRateLimitError(
                    str(e), provider_name=self.name, status_code=429,
                ) from e
            if sc and 500 <= int(sc) < 600:
                raise ProviderUnavailableError(
                    str(e), provider_name=self.name, status_code=int(sc),
                ) from e
        raise ProviderError(str(e), provider_name=self.name) from e


__all__ = ["BedrockProvider"]
