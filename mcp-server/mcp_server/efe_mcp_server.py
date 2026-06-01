"""EFE MCP Server (M16) — main entry point.

Boots the MCP protocol server, registers the 6 EFE tools, and dispatches
calls through the validation / rate-limit / audit pipeline.

Two transports are supported :
  - **stdio** (default) — JSON-RPC over stdin/stdout, used by Claude
    Desktop, Cursor, Zed, Continue, Cline. The MCP server runs as a
    subprocess of the LLM client.
  - **http** — HTTP+SSE on :8800, for cloud deployments.

The optional `mcp` Python SDK is imported lazily so unit tests don't
need the full MCP runtime — they exercise the dispatcher directly.
"""
from __future__ import annotations

import asyncio
import json
import logging
import os
import sys
from typing import Any, Optional

import click

from .audit_emitter import AuditEmitter
from .auth import AuthError, load_auth_from_env
from .backend_client import BackendClient, detect_backend
from .dispatcher import Dispatcher
from .profile import Profile, load_profile_from_env
from .rate_limiter import RateLimiter
from .sampling import SamplingClient
from .state import EmbeddingCache
from .tools import build_tool_registry


logger = logging.getLogger("efe_mcp")


# ─────────────────────────────────────────────────────────────────────
# Server bootstrap
# ─────────────────────────────────────────────────────────────────────


class EFEMCPServer:
    """High-level façade. Holds the dispatcher + transport.

    Construction is fully sync ; `run()` is the entry-point that picks
    the transport and starts the asyncio loop.
    """

    def __init__(
        self,
        *,
        endpoint: Optional[str] = None,
        transport: Optional[str] = None,
        rate_limit_per_min: Optional[int] = None,
        audit_enabled: Optional[bool] = None,
    ) -> None:
        # Resolve config from env if not passed.
        endpoint = endpoint or os.environ.get("EFE_ENDPOINT")
        if not endpoint:
            raise RuntimeError(
                "EFE_ENDPOINT environment variable is required "
                "(e.g. https://efe.example.com or http://localhost:8765)"
            )
        transport = (transport or os.environ.get("EFE_MCP_TRANSPORT", "stdio")).strip().lower()
        if transport not in ("stdio", "http"):
            raise RuntimeError(
                f"unsupported transport {transport!r}. Use 'stdio' or 'http'."
            )
        if rate_limit_per_min is None:
            rate_limit_per_min = int(os.environ.get("EFE_MCP_RATE_LIMIT_PER_MIN", "100"))

        # Auth
        try:
            self.auth = load_auth_from_env()
        except AuthError as exc:
            raise RuntimeError(f"auth bootstrap failed : {exc}") from exc

        # Backend
        verify_tls = os.environ.get("EFE_MCP_INSECURE", "false").lower() != "true"
        self.backend: BackendClient = detect_backend(endpoint, verify_tls=verify_tls)

        # M17 : profile (defaults to core, identical to M16)
        self.profile: Profile = load_profile_from_env()

        # Tools (filtered by profile)
        self.registry = build_tool_registry(self.profile)

        # Audit + rate limit
        self.audit = AuditEmitter(self.backend, enabled=audit_enabled)
        self.rate_limiter = RateLimiter(per_min=rate_limit_per_min)

        # M17 : embedding cache + sampling client (sampling stays None
        # until the MCP session attaches)
        self.embedding_cache = EmbeddingCache()
        self.sampling: SamplingClient | None = None

        # Dispatcher
        self.dispatcher = Dispatcher(
            registry=self.registry,
            backend=self.backend,
            auth=self.auth,
            audit=self.audit,
            rate_limiter=self.rate_limiter,
            profile=self.profile,
            sampling=self.sampling,
            embedding_cache=self.embedding_cache,
        )

        self.transport = transport

    # ------------------------------------------------------------------
    # Run loop
    # ------------------------------------------------------------------

    def run(self) -> None:
        if self.transport == "stdio":
            asyncio.run(self._run_stdio())
        elif self.transport == "http":
            asyncio.run(self._run_http())

    async def _run_stdio(self) -> None:
        """Run the MCP server over stdio (subprocess transport)."""
        try:
            from mcp.server import Server  # type: ignore[import-not-found]
            from mcp.server.stdio import stdio_server  # type: ignore[import-not-found]
            import mcp.types as types  # type: ignore[import-not-found]
        except ImportError:
            # Fallback : run a minimal JSON-RPC reader on stdin/stdout
            # so the server is usable in environments without the MCP
            # SDK installed (e.g. tests + offline CI).
            logger.info(
                "mcp SDK not available ; falling back to minimal stdio loop"
            )
            await self._run_stdio_minimal()
            return

        server = Server("efe-mcp-server")

        @server.list_tools()  # type: ignore[misc]
        async def list_tools_impl() -> list[Any]:
            return [
                types.Tool(
                    name=t["name"],
                    description=t["description"],
                    inputSchema=t["inputSchema"],
                )
                for t in self.dispatcher.list_tools()
            ]

        # M17-A2 fix : wire MCP session for streaming notifications.
        progress_session_adapter = _MCPProgressSessionAdapter(server)

        @server.call_tool()  # type: ignore[misc]
        async def call_tool_impl(
            name: str, arguments: dict[str, Any]
        ) -> list[Any]:
            # Extract progressToken from request _meta if present (MCP
            # SDK exposes this via request_context). Defensively handle
            # SDK shape differences.
            progress_token = None
            try:
                ctx = server.request_context  # type: ignore[attr-defined]
                meta = getattr(ctx, "meta", None) or getattr(ctx, "request_meta", None)
                if meta is not None:
                    progress_token = (
                        getattr(meta, "progressToken", None)
                        or (meta.get("progressToken") if isinstance(meta, dict) else None)
                    )
            except (AttributeError, KeyError):
                progress_token = None

            result = await self.dispatcher.dispatch(
                tool_name=name,
                args=arguments or {},
                progress_token=progress_token,
                progress_session=progress_session_adapter if progress_token else None,
            )
            payload = json.dumps(result.content, ensure_ascii=False, indent=2)
            return [
                types.TextContent(type="text", text=payload),
            ]

        async with stdio_server() as (read, write):
            init_options = server.create_initialization_options()
            await server.run(read, write, init_options)

    async def _run_stdio_minimal(self) -> None:
        """Bare-bones JSON-RPC 2.0 read loop on stdin (for offline tests)."""
        loop = asyncio.get_event_loop()
        reader = asyncio.StreamReader()
        protocol = asyncio.StreamReaderProtocol(reader)
        await loop.connect_read_pipe(lambda: protocol, sys.stdin)
        while True:
            line = await reader.readline()
            if not line:
                return
            try:
                request = json.loads(line.decode("utf-8"))
            except json.JSONDecodeError:
                self._write_jsonrpc_error(None, -32700, "parse error")
                continue
            await self._handle_jsonrpc_request(request)

    async def _handle_jsonrpc_request(self, request: dict[str, Any]) -> None:
        """Minimal dispatch — tools, resources, prompts."""
        request_id = request.get("id")
        method = request.get("method")
        params = request.get("params") or {}

        if method == "tools/list":
            self._write_jsonrpc_result(request_id, {"tools": self.dispatcher.list_tools()})
            return
        if method == "tools/call":
            name = params.get("name") or ""
            arguments = params.get("arguments") or {}
            meta = params.get("_meta") or {}
            progress_token = meta.get("progressToken") if isinstance(meta, dict) else None
            result = await self.dispatcher.dispatch(
                tool_name=name, args=arguments, progress_token=progress_token,
            )
            self._write_jsonrpc_result(
                request_id,
                {
                    "content": [
                        {
                            "type": "text",
                            "text": json.dumps(result.content, ensure_ascii=False),
                        }
                    ],
                    "isError": result.is_error,
                },
            )
            return
        if method == "resources/list":
            self._write_jsonrpc_result(
                request_id, {"resources": self.dispatcher.list_resources()},
            )
            return
        if method == "resources/read":
            uri = params.get("uri") or ""
            result_dict = await self.dispatcher.read_resource(uri)
            if result_dict is None:
                self._write_jsonrpc_error(
                    request_id, -32602,
                    "resources unsupported by current profile",
                )
                return
            if "error" in result_dict:
                self._write_jsonrpc_result(
                    request_id, {"contents": [], "_error": result_dict},
                )
                return
            self._write_jsonrpc_result(
                request_id, {"contents": [result_dict]},
            )
            return
        if method == "prompts/list":
            self._write_jsonrpc_result(
                request_id, {"prompts": self.dispatcher.list_prompts()},
            )
            return
        if method == "prompts/get":
            name = params.get("name") or ""
            args = params.get("arguments") or {}
            result_dict = self.dispatcher.get_prompt(name, args)
            self._write_jsonrpc_result(request_id, result_dict)
            return
        self._write_jsonrpc_error(request_id, -32601, f"method not found : {method}")

    @staticmethod
    def _write_jsonrpc_result(req_id: Any, result: Any) -> None:
        sys.stdout.write(
            json.dumps(
                {"jsonrpc": "2.0", "id": req_id, "result": result},
                ensure_ascii=False,
            )
            + "\n"
        )
        sys.stdout.flush()

    @staticmethod
    def _write_jsonrpc_error(req_id: Any, code: int, message: str) -> None:
        sys.stdout.write(
            json.dumps(
                {
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "error": {"code": code, "message": message},
                },
                ensure_ascii=False,
            )
            + "\n"
        )
        sys.stdout.flush()

    async def _run_http(self) -> None:
        """HTTP+SSE transport — minimal sketch ; M16 ships stdio first."""
        raise NotImplementedError(
            "HTTP+SSE transport scheduled for M16 audit pass / M19 livraison ; "
            "use EFE_MCP_TRANSPORT=stdio for now."
        )

    async def aclose(self) -> None:
        await self.backend.aclose()


# ─────────────────────────────────────────────────────────────────────
# CLI entry-point
# ─────────────────────────────────────────────────────────────────────


@click.command(name="efe-mcp-server")
@click.option(
    "--endpoint",
    envvar="EFE_ENDPOINT",
    help="EFE backend URL (https://efe.example.com or http://localhost:8765).",
)
@click.option(
    "--transport",
    envvar="EFE_MCP_TRANSPORT",
    type=click.Choice(["stdio", "http"], case_sensitive=False),
    default="stdio",
    show_default=True,
    help="MCP transport.",
)
@click.option(
    "--rate-limit-per-min",
    envvar="EFE_MCP_RATE_LIMIT_PER_MIN",
    type=int,
    default=100,
    show_default=True,
    help="Per-(token, tool) requests per minute.",
)
@click.option(
    "--log-level",
    envvar="EFE_MCP_LOG_LEVEL",
    type=click.Choice(["DEBUG", "INFO", "WARNING", "ERROR"]),
    default="INFO",
    show_default=True,
)
def main(
    endpoint: Optional[str],
    transport: str,
    rate_limit_per_min: int,
    log_level: str,
) -> None:
    """Start the EFE MCP Server.

    Reads `EFE_OIDC_TOKEN` from the environment for authentication.
    """
    # Send logs to stderr — stdout is reserved for the JSON-RPC channel
    # in stdio transport mode.
    logging.basicConfig(
        level=log_level,
        stream=sys.stderr,
        format="%(asctime)s %(levelname)s %(name)s : %(message)s",
    )
    try:
        server = EFEMCPServer(
            endpoint=endpoint,
            transport=transport,
            rate_limit_per_min=rate_limit_per_min,
        )
    except RuntimeError as exc:
        logger.error("startup failed : %s", exc)
        sys.exit(2)
    logger.info(
        "efe-mcp-server boot : endpoint=%s transport=%s tools=%d "
        "rate_limit=%d/min token=%s",
        server.backend.info.endpoint,
        server.transport,
        len(server.registry),
        server.rate_limiter.per_min,
        server.auth.redacted_token(),
    )
    try:
        server.run()
    except KeyboardInterrupt:
        logger.info("shutting down (SIGINT)")
    finally:
        try:
            asyncio.run(server.aclose())
        except RuntimeError:
            # Event loop already closed by `server.run()`.
            pass


# ─────────────────────────────────────────────────────────────────────
# MCP SDK session adapter (M17-A2 fix)
# ─────────────────────────────────────────────────────────────────────


class _MCPProgressSessionAdapter:
    """Adapts the MCP SDK server session to our ProgressSession protocol.

    Defensive : silently no-op if the SDK doesn't expose
    `request_context.session.send_progress_notification` (e.g. older
    SDK or shape mismatch). The dispatcher still gets a valid object,
    streaming just degrades gracefully to a final-only response.
    """

    def __init__(self, mcp_server: Any) -> None:
        self._server = mcp_server

    async def send_progress_notification(
        self,
        progress_token: str,
        *,
        progress: int,
        total: int = 100,
        message: str = "",  # noqa: ARG002 — kept for API symmetry
    ) -> None:
        try:
            ctx = self._server.request_context
            session = ctx.session
        except AttributeError:
            return
        send = getattr(session, "send_progress_notification", None)
        if send is None:
            return
        try:
            # MCP SDK signature evolved over versions ; try the most
            # common shapes in order.
            try:
                await send(progress_token=progress_token, progress=progress, total=total)
                return
            except TypeError:
                pass
            try:
                await send(progress_token, progress, total)
                return
            except TypeError:
                pass
            await send(progress_token, progress)
        except Exception:  # noqa: BLE001
            return  # never propagate — streaming is best-effort


if __name__ == "__main__":
    main()
