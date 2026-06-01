"""Pin tests for the Agent facade — direct provider mode."""
from __future__ import annotations

import json

import pytest

from efe_byo_agent import Agent
from efe_byo_agent.exceptions import EfeAgentError
from efe_byo_agent.llm.base import LLMProvider
from efe_byo_agent.types import CostModel, LLMResponse, ProviderHealth


class _StaticProvider(LLMProvider):
    """Provider that returns a pre-canned response."""

    def __init__(self, response_text: str, name: str = "static"):
        self._response = response_text
        self._name = name

    @property
    def name(self) -> str:
        return self._name

    @property
    def supports_tools(self) -> bool:
        return True

    @property
    def supports_streaming(self) -> bool:
        return True

    @property
    def max_context_tokens(self) -> int:
        return 100_000

    @property
    def cost_model(self) -> CostModel:
        return CostModel(input_per_1m_tokens_usd=0.5, output_per_1m_tokens_usd=2.0)

    async def complete(self, **_kwargs) -> LLMResponse:
        return LLMResponse(
            text=self._response,
            tokens_in=100, tokens_out=50,
            cost_usd=0.0001,
            finish_reason="stop",
            provider_name=self._name,
            model_name="static",
        )

    async def stream(self, **kwargs):
        yield await self.complete(**kwargs)

    async def health(self) -> ProviderHealth:
        return ProviderHealth(available=True, last_check_ts=0.0)


def _summarize_payload() -> str:
    return json.dumps({
        "explanation": "x" * 50,
        "key_factors": ["a", "b"],
        "confidence_assessment": "high",
        "suggested_alternatives": [],
    })


@pytest.mark.asyncio
async def test_agent_helper_direct_mode_summarize():
    provider = _StaticProvider(_summarize_payload())
    agent = Agent(llm_provider=provider)
    try:
        result = await agent.helper(
            "summarize_classification",
            catalog="x",
            sentence="hello",
            classification_result={"label": "x", "confidence": 0.5},
        )
        assert result["confidence_assessment"] == "high"
        assert agent.cost_tracker.total_calls == 1
        assert agent.cost_tracker.total_cost_usd == 0.0001
    finally:
        await agent.aclose()


@pytest.mark.asyncio
async def test_agent_helper_unknown_raises():
    provider = _StaticProvider(_summarize_payload())
    agent = Agent(llm_provider=provider)
    try:
        with pytest.raises(EfeAgentError):
            await agent.helper("nonexistent_helper")
    finally:
        await agent.aclose()


@pytest.mark.asyncio
async def test_agent_mcp_tool_requires_endpoint():
    provider = _StaticProvider(_summarize_payload())
    agent = Agent(llm_provider=provider)  # no mcp_endpoint
    try:
        with pytest.raises(EfeAgentError):
            await agent.mcp_tool("efe_classify", {})
    finally:
        await agent.aclose()


@pytest.mark.asyncio
async def test_agent_endpoint_requires_token():
    provider = _StaticProvider(_summarize_payload())
    with pytest.raises(EfeAgentError):
        Agent(
            llm_provider=provider,
            mcp_endpoint="http://localhost:8800",
            # oidc_token missing
        )


@pytest.mark.asyncio
async def test_agent_cost_tracker_per_provider():
    provider = _StaticProvider(_summarize_payload())
    agent = Agent(llm_provider=provider)
    try:
        await agent.helper(
            "summarize_classification",
            catalog="x", sentence="hi",
            classification_result={"label": "x", "confidence": 0.5},
        )
        assert "static" in agent.cost_tracker.by_provider
        assert agent.cost_tracker.by_provider["static"]["calls"] == 1
    finally:
        await agent.aclose()


@pytest.mark.asyncio
async def test_agent_health_returns_provider_status():
    provider = _StaticProvider(_summarize_payload())
    agent = Agent(llm_provider=provider)
    try:
        h = await agent.health()
        assert h["provider"]["available"] is True
        assert h["provider"]["name"] == "static"
    finally:
        await agent.aclose()
