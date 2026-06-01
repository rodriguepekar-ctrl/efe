"""Checkpoint manager. Serializes message history + step counter when
the agent is parked (awaiting approval, paused, …) so a fresh
agent-core process can resume."""
from __future__ import annotations

import json
import os
from typing import Any

import httpx
import structlog

log = structlog.get_logger()


class Checkpoint:
    def __init__(self, run_id: str) -> None:
        self.run_id = run_id
        self.base_url = os.environ.get(
            "EFE_MCP_SERVER_URL", "http://efe-mcp-server.local:8080"
        )
        self.jwt = os.environ.get("EFE_LLM_JWT", "")

    async def save(self, messages: list[dict[str, Any]], step: int) -> None:
        blob = json.dumps({"messages": messages, "step": step}).encode()
        async with httpx.AsyncClient(timeout=15.0) as client:
            await client.post(
                f"{self.base_url}/v1/agent/checkpoint",
                headers={
                    "Authorization": f"Bearer {self.jwt}",
                    "X-EFE-Run-Id": self.run_id,
                },
                content=blob,
            )

    async def load(self) -> dict[str, Any] | None:
        async with httpx.AsyncClient(timeout=15.0) as client:
            r = await client.get(
                f"{self.base_url}/v1/agent/checkpoint",
                headers={
                    "Authorization": f"Bearer {self.jwt}",
                    "X-EFE-Run-Id": self.run_id,
                },
            )
            if r.status_code == 404:
                return None
            r.raise_for_status()
            return r.json()
