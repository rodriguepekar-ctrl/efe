"""MCP client — talks to the embedded mcp-server pod via vsock-relay.

The mcp-server enforces **layer 3** of the tool whitelist + per-call
approval. If a tool call needs operator approval and we don't have a
valid token, the server replies with HTTP 403 +
`X-EFE-NeedsApproval: <approval_id>` and we raise `ToolCallDenied`.
"""
from __future__ import annotations

import os
from typing import Any

import httpx
import structlog

log = structlog.get_logger()


class ToolCallDenied(Exception):
    """Raised when the mcp-server denies a tool call (whitelist or approval)."""


class McpClient:
    def __init__(self, cfg, audit) -> None:
        self.cfg = cfg
        self.audit = audit
        self.base_url = os.environ.get(
            "EFE_MCP_SERVER_URL", "http://efe-mcp-server.local:8080"
        )
        self.jwt = os.environ.get("EFE_LLM_JWT", "")

    async def call_tool(
        self,
        tool_name: str,
        arguments: dict[str, Any],
        step: int,
    ) -> str:
        await self.audit.event(
            "tool_call_attempted",
            {"tool": tool_name, "step": step},
        )
        async with httpx.AsyncClient(timeout=60.0) as client:
            r = await client.post(
                f"{self.base_url}/v1/tools/{tool_name}",
                headers={
                    "Authorization": f"Bearer {self.jwt}",
                    "X-EFE-Run-Id": self.cfg.run_id,
                    "X-EFE-Tenant": self.cfg.tenant_id,
                    "X-EFE-Step": str(step),
                },
                json=arguments,
            )
            if r.status_code == 403:
                approval_id = r.headers.get("X-EFE-NeedsApproval", "")
                raise ToolCallDenied(
                    f"approval required (id={approval_id})"
                )
            r.raise_for_status()
            data = r.json()

        await self.audit.event(
            "tool_call_completed",
            {"tool": tool_name, "step": step},
        )
        return str(data.get("result", ""))

    async def report_final(self, final_output: dict[str, Any]) -> None:
        async with httpx.AsyncClient(timeout=15.0) as client:
            r = await client.post(
                f"{self.base_url}/v1/agent/final",
                headers={
                    "Authorization": f"Bearer {self.jwt}",
                    "X-EFE-Run-Id": self.cfg.run_id,
                    "X-EFE-Tenant": self.cfg.tenant_id,
                },
                json=final_output,
            )
            r.raise_for_status()
