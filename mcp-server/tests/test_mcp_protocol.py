"""JSON-RPC 2.0 minimal protocol pin tests (M16).

The MCP server falls back to a tiny JSON-RPC reader when the official
`mcp` SDK is not available. These tests verify that the fallback path
honours the protocol shape the SDK expects.
"""
from __future__ import annotations

import json

import pytest

from mcp_server.dispatcher import Dispatcher
from mcp_server.efe_mcp_server import EFEMCPServer


@pytest.mark.asyncio
async def test_tools_list_returns_six_tools(dispatcher):
    tools = dispatcher.list_tools()
    names = {t["name"] for t in tools}
    assert names == {
        "efe_classify",
        "efe_list_catalogs",
        "efe_get_audit_log",
        "efe_get_admin_quotas",
        "efe_register_catalog",
        "efe_evolve_catalog",
    }


@pytest.mark.asyncio
async def test_tools_list_each_has_inputSchema_and_description(dispatcher):
    for t in dispatcher.list_tools():
        assert "name" in t
        assert "description" in t
        assert isinstance(t["description"], str) and len(t["description"]) > 10
        assert "inputSchema" in t
        assert t["inputSchema"]["type"] == "object"


@pytest.mark.asyncio
async def test_jsonrpc_tools_list_dispatches(dispatcher, capsys):
    """`tools/list` JSON-RPC method shape."""
    server = _make_server_with_dispatcher(dispatcher)
    request = {"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}}
    await server._handle_jsonrpc_request(request)
    captured = capsys.readouterr().out.strip()
    response = json.loads(captured)
    assert response["jsonrpc"] == "2.0"
    assert response["id"] == 1
    assert "tools" in response["result"]
    assert len(response["result"]["tools"]) == 6


@pytest.mark.asyncio
async def test_jsonrpc_tools_call_returns_text_content(dispatcher, capsys):
    server = _make_server_with_dispatcher(dispatcher)
    # Pre-create a catalog
    await dispatcher.dispatch(
        tool_name="efe_register_catalog",
        args={
            "name": "ggg",
            "classes": [{"name": "x", "examples": ["a"]}],
            "confirm": True,
        },
    )
    capsys.readouterr()  # discard
    request = {
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {"name": "efe_classify", "arguments": {"catalog": "ggg", "text": "hi"}},
    }
    await server._handle_jsonrpc_request(request)
    captured = capsys.readouterr().out.strip()
    response = json.loads(captured)
    assert response["id"] == 2
    result = response["result"]
    assert result["isError"] is False
    assert isinstance(result["content"], list)
    assert result["content"][0]["type"] == "text"
    inner = json.loads(result["content"][0]["text"])
    assert inner["catalog_name"] == "ggg"


@pytest.mark.asyncio
async def test_jsonrpc_tools_call_isError_on_unknown_tool(dispatcher, capsys):
    server = _make_server_with_dispatcher(dispatcher)
    request = {
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {"name": "nonexistent", "arguments": {}},
    }
    await server._handle_jsonrpc_request(request)
    captured = capsys.readouterr().out.strip()
    response = json.loads(captured)
    assert response["result"]["isError"] is True
    inner = json.loads(response["result"]["content"][0]["text"])
    assert inner["code"] == "unknown_tool"


@pytest.mark.asyncio
async def test_jsonrpc_method_not_found(dispatcher, capsys):
    server = _make_server_with_dispatcher(dispatcher)
    request = {"jsonrpc": "2.0", "id": 4, "method": "bogus/op", "params": {}}
    await server._handle_jsonrpc_request(request)
    captured = capsys.readouterr().out.strip()
    response = json.loads(captured)
    assert response["error"]["code"] == -32601


# ─────────────────────────────────────────────────────────────────────
# helpers
# ─────────────────────────────────────────────────────────────────────


def _make_server_with_dispatcher(dispatcher: Dispatcher) -> EFEMCPServer:
    """Build an EFEMCPServer instance bypassing __init__ (avoids env dep)."""
    server = EFEMCPServer.__new__(EFEMCPServer)
    server.dispatcher = dispatcher
    server.transport = "stdio"
    server.registry = dispatcher.registry
    server.backend = dispatcher.backend
    server.auth = dispatcher.auth
    server.audit = dispatcher.audit
    server.rate_limiter = dispatcher.rate_limiter
    return server
