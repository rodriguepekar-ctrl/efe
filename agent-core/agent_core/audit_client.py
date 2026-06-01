"""Audit emission client — fire-and-forget event posting to controller.

Events go through the same vsock-relay as the mcp-server traffic, but
on a separate `/v1/audit/append` endpoint. The controller persists
each event in `agent_audit` (HMAC-chained per tenant).
"""
from __future__ import annotations

import asyncio
import os
from typing import Any

import httpx
import structlog

log = structlog.get_logger()


class AuditClient:
    def __init__(self, cfg) -> None:
        self.cfg = cfg
        self.base_url = os.environ.get(
            "EFE_MCP_SERVER_URL", "http://efe-mcp-server.local:8080"
        )
        self.jwt = os.environ.get("EFE_LLM_JWT", "")

    async def event(self, event_type: str, payload: dict[str, Any]) -> None:
        # Fire-and-forget : never block the agent loop on audit failures
        # (the controller-side audit is the authoritative source via the
        # mcp-server side-band events ; this is just defense-in-depth).
        try:
            async with httpx.AsyncClient(timeout=5.0) as client:
                await client.post(
                    f"{self.base_url}/v1/audit/append",
                    headers={
                        "Authorization": f"Bearer {self.jwt}",
                        "X-EFE-Run-Id": self.cfg.run_id,
                        "X-EFE-Tenant": self.cfg.tenant_id,
                    },
                    json={"event_type": event_type, "payload": payload},
                )
        except Exception as e:  # noqa: BLE001
            log.warning("audit emit failed", event=event_type, err=str(e))

    async def heartbeat(self, info: dict[str, Any]) -> None:
        await self.event("agent_core_heartbeat", info)

    async def fire_and_forget(self, event_type: str, payload: dict[str, Any]) -> None:
        # Useful for non-critical-path events.
        asyncio.create_task(self.event(event_type, payload))
