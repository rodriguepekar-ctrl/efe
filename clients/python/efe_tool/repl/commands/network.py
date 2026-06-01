"""Network commands: ``:endpoint`` — change the gRPC endpoint live."""

from __future__ import annotations

from typing import Any

from ...cli.config import validate_config
from ...cli.errors import UsageError, from_bridge_error
from . import Command, register


def _cmd_endpoint(state: Any, args: list[str], log: Any) -> str:
    if not args:
        return f"endpoint: {state.endpoint} ({'TLS' if state.tls else 'insecure'})"
    new = args[0]
    if new == "reset":
        new = state.boot_cfg.endpoint

    # Cheap validation to reject URL-schemes and missing ports early.
    from dataclasses import replace
    tentative = replace(state.boot_cfg, endpoint=new)
    try:
        validate_config(tentative)
    except UsageError as e:
        raise UsageError(f"invalid endpoint: {e}", hint="expected host:port") from e

    # Close existing client, build a fresh one.
    from efe_tool.errors import BridgeError
    if state.client is not None:
        try:
            state.client.close()
        except Exception as e:  # pragma: no cover
            log.warn("repl_client_close_failed", error=str(e))
        state.client = None

    try:
        from efe_tool.client import EfeClient
        state.client = EfeClient(
            endpoint=new,
            embedder=state.embedder,
            jwt=state.jwt,
            tenant=state.tenant,
            timeout_s=state.timeout_s,
            insecure=not state.tls,
        )
    except BridgeError as e:
        raise from_bridge_error(e) from e

    state.endpoint = new
    log.info("repl_endpoint_switched", endpoint=new)
    return f"endpoint switched to {new}"


register(Command(
    name="endpoint",
    run=_cmd_endpoint,
    doc="Without arg: show endpoint. With HOST:PORT: switch. 'reset' → boot value.",
    group="network",
))
