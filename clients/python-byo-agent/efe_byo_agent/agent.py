"""Agent — high-level facade for efe-byo-agent (M20).

Two operation modes :

  1. Direct provider mode (no MCP) — instantiate with `llm_provider=`
     only. Helpers are invoked locally. Suitable for CI batch / scripts
     where MCP is not available.

  2. MCP-mediated mode — instantiate with `mcp_endpoint=` AND
     `oidc_token=`. Helpers can fetch context (e.g. `efe_classify`,
     `efe_diagnose_catalog`) from the EFE backend via mcp-server
     before calling the LLM. Use `agent.mcp_tool(name, args)` to invoke
     any mcp-server tool whose result you want to feed into a helper.
"""
from __future__ import annotations

from typing import Any

import structlog

from .exceptions import EfeAgentError
from .helpers import HELPERS
from .llm.base import LLMProvider
from .mcp_client import MCPClient
from .types import CostTracker

log = structlog.get_logger("efe_byo_agent.agent")


class Agent:
    """High-level BYO-LLM facade for EFE M20 helpers."""

    def __init__(
        self,
        *,
        llm_provider: LLMProvider,
        mcp_endpoint: str | None = None,
        oidc_token: str | None = None,
        timeout_s: float = 60.0,
        max_retries: int = 2,
    ) -> None:
        self.llm = llm_provider
        self.timeout_s = timeout_s
        self.max_retries = max_retries
        self.cost_tracker = CostTracker()

        if mcp_endpoint:
            if not oidc_token:
                raise EfeAgentError(
                    "mcp_endpoint set but oidc_token missing"
                )
            self.mcp: MCPClient | None = MCPClient(
                endpoint=mcp_endpoint,
                oidc_token=oidc_token,
                timeout_s=timeout_s,
            )
        else:
            self.mcp = None

    async def __aenter__(self) -> "Agent":
        return self

    async def __aexit__(self, *args) -> None:
        await self.aclose()

    async def aclose(self) -> None:
        if self.mcp is not None:
            await self.mcp.aclose()

    # ─── Direct helper mode ────────────────────────────────────────

    async def helper(self, name: str, **kwargs: Any) -> dict:
        """Invoke a M20 helper using the configured LLM provider directly.

        Same prompt template + validation as the corresponding
        mcp-server tool. Useful for CI / batch scripts.
        """
        if name not in HELPERS:
            raise EfeAgentError(
                f"unknown helper {name!r}. Known : {list(HELPERS.keys())}"
            )
        helper_cls = HELPERS[name]
        helper = helper_cls(agent=self)
        return await helper.run(**kwargs)

    # ─── MCP tool mode ─────────────────────────────────────────────

    async def mcp_tool(
        self,
        name: str,
        arguments: dict | None = None,
    ) -> Any:
        """Invoke any mcp-server tool by name (read-only or write).

        Sampling round-trips inside mcp-server are NOT routed through
        this Agent's LLM in M20 (mcp-server sampling adapter is
        connection-local). For full sampling-callback support, use
        Anthropic's official MCP Python SDK.
        """
        if self.mcp is None:
            raise EfeAgentError(
                "mcp_tool requires mcp_endpoint at Agent construction"
            )
        return await self.mcp.call_tool(name, arguments or {})

    # ─── Diagnostics ───────────────────────────────────────────────

    async def health(self) -> dict:
        provider_health = await self.llm.health()
        info: dict = {
            "provider": {
                "name": self.llm.name,
                "available": provider_health.available,
            },
            "cost_tracker": {
                "total_calls": self.cost_tracker.total_calls,
                "total_cost_usd": self.cost_tracker.total_cost_usd,
            },
        }
        if self.mcp is not None:
            info["mcp"] = {"endpoint_set": True}
        return info


__all__ = ["Agent"]
