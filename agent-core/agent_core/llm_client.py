"""LLM client — talks to the M21 LLMRouter pod via vsock-relay.

The relay exposes a unix-domain-style HTTP endpoint over vsock CID 2
port 9000 (set in the Helm chart's NetworkPolicy as the only allowed
egress for the LLMRouter service). All requests are authenticated by
a per-run JWT minted by the controller (passed in `EFE_LLM_JWT`).
"""
from __future__ import annotations

import os
from dataclasses import dataclass
from typing import Any

import httpx
import structlog
from tenacity import retry, stop_after_attempt, wait_exponential

log = structlog.get_logger()


@dataclass
class LlmResponse:
    text: str | None
    tool_call: dict[str, Any] | None
    cost_cents: int


class LlmClient:
    def __init__(self, cfg, cost, audit) -> None:
        self.cfg = cfg
        self.cost = cost
        self.audit = audit
        # vsock-relay endpoint — DNS-resolved inside the microVM via
        # /etc/hosts injected at boot.
        self.base_url = os.environ.get(
            "EFE_LLM_ROUTER_URL", "http://efe-llm-router.local:50051"
        )
        self.jwt = os.environ.get("EFE_LLM_JWT", "")

    @retry(stop=stop_after_attempt(3), wait=wait_exponential(min=0.5, max=4.0))
    async def complete(self, messages: list[dict[str, Any]]) -> LlmResponse:
        async with httpx.AsyncClient(timeout=30.0) as client:
            r = await client.post(
                f"{self.base_url}/v1/complete",
                headers={"Authorization": f"Bearer {self.jwt}"},
                json={
                    "tenant_id": self.cfg.tenant_id,
                    "run_id": self.cfg.run_id,
                    "provider": self.cfg.provider,
                    "model": self.cfg.model,
                    "messages": messages,
                    "tool_whitelist": self.cfg.tool_whitelist,
                },
            )
            r.raise_for_status()
            data = r.json()

        cost_cents = int(data.get("cost_cents", 0))
        self.cost.add(cost_cents)
        await self.audit.event(
            "llm_call_completed",
            {
                "provider": self.cfg.provider,
                "model": self.cfg.model,
                "cost_cents": cost_cents,
                "input_tokens": data.get("input_tokens", 0),
                "output_tokens": data.get("output_tokens", 0),
            },
        )
        return LlmResponse(
            text=data.get("text"),
            tool_call=data.get("tool_call"),
            cost_cents=cost_cents,
        )
