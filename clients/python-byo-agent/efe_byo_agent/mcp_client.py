"""Lightweight MCP client over HTTP+SSE for efe-byo-agent (M20).

The full MCP spec includes stdio + HTTP+SSE transports. This client
supports HTTP+SSE only (matches M19 production deployment). For stdio
clients, use Anthropic's official MCP Python SDK.

This implementation is intentionally minimal — just enough to call
mcp-server tools and respond to sampling requests with our local
LLMProvider.
"""
from __future__ import annotations

import json
import uuid
from typing import Any, Awaitable, Callable

import httpx

from .exceptions import (
    MCPRpcError,
    MCPTransportError,
)


class MCPClient:
    """Minimal HTTP+SSE MCP client.

    Args :
      endpoint     : mcp-server base URL (e.g. http://efe-mcp:8800).
      oidc_token   : Bearer token for `Authorization` header.
      timeout_s    : per-request timeout.

    Usage :
        async with MCPClient(endpoint="...", oidc_token="...") as client:
            result = await client.call_tool("efe_summarize_classification", args)
    """

    def __init__(
        self,
        *,
        endpoint: str,
        oidc_token: str,
        timeout_s: float = 60.0,
    ) -> None:
        self._endpoint = endpoint.rstrip("/")
        self._oidc_token = oidc_token
        self._timeout = timeout_s
        self._client = httpx.AsyncClient(
            base_url=self._endpoint,
            headers={
                "Authorization": f"Bearer {oidc_token}",
                "Content-Type": "application/json",
                "Accept": "application/json, text/event-stream",
            },
            timeout=timeout_s,
        )

    async def __aenter__(self) -> "MCPClient":
        return self

    async def __aexit__(self, *args) -> None:
        await self.aclose()

    async def aclose(self) -> None:
        await self._client.aclose()

    # ─── Tool invocation ──────────────────────────────────────────

    async def call_tool(
        self,
        name: str,
        arguments: dict[str, Any],
        *,
        sampling_callback: Callable[[dict], Awaitable[str]] | None = None,
    ) -> dict[str, Any]:
        """Invoke an mcp-server tool by name.

        If the tool requires sampling and `sampling_callback` is provided,
        sampling round-trips are routed through it. M20 simplified
        protocol : the call returns the final tool output ; sampling
        round-trips inside mcp-server are negotiated server-side
        (mcp-server uses the connection's sampling adapter).

        For full sampling-callback support, prefer the official
        Anthropic MCP Python SDK with stdio transport.
        """
        request_id = str(uuid.uuid4())
        payload = {
            "jsonrpc": "2.0",
            "id": request_id,
            "method": "tools/call",
            "params": {
                "name": name,
                "arguments": arguments,
            },
        }
        try:
            resp = await self._client.post(
                "/mcp/messages",
                json=payload,
            )
        except httpx.HTTPError as e:
            raise MCPTransportError(f"transport failure : {e}") from e

        if resp.status_code >= 400:
            raise MCPTransportError(
                f"HTTP {resp.status_code} : {resp.text[:200]}"
            )

        try:
            body = resp.json()
        except json.JSONDecodeError as e:
            raise MCPTransportError(f"invalid JSON response : {e}") from e

        if "error" in body:
            err = body["error"]
            raise MCPRpcError(
                err.get("message", "rpc error"),
                code=err.get("code"),
                data=err.get("data") or {},
            )

        result = body.get("result", {})
        return result

    # ─── List operations ──────────────────────────────────────────

    async def list_tools(self) -> list[dict[str, Any]]:
        return await self._simple_call("tools/list", {})

    async def list_resources(self) -> list[dict[str, Any]]:
        return await self._simple_call("resources/list", {})

    async def list_prompts(self) -> list[dict[str, Any]]:
        return await self._simple_call("prompts/list", {})

    async def read_resource(self, uri: str) -> dict[str, Any]:
        return await self._simple_call("resources/read", {"uri": uri})

    # ─── Internals ────────────────────────────────────────────────

    async def _simple_call(
        self, method: str, params: dict[str, Any],
    ) -> Any:
        payload = {
            "jsonrpc": "2.0",
            "id": str(uuid.uuid4()),
            "method": method,
            "params": params,
        }
        try:
            resp = await self._client.post("/mcp/messages", json=payload)
        except httpx.HTTPError as e:
            raise MCPTransportError(f"transport failure : {e}") from e
        if resp.status_code >= 400:
            raise MCPTransportError(
                f"HTTP {resp.status_code} : {resp.text[:200]}"
            )
        body = resp.json()
        if "error" in body:
            err = body["error"]
            raise MCPRpcError(
                err.get("message", "rpc error"),
                code=err.get("code"),
                data=err.get("data") or {},
            )
        return body.get("result")


__all__ = ["MCPClient"]
