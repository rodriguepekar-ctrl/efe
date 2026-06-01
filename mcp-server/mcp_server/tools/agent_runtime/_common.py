"""Common helpers for the M25 agent runtime MCP tools."""
from __future__ import annotations

import os
from typing import Any

from ..types import ToolContext, ToolError


def _server_m25_enabled() -> bool:
    return os.environ.get("EFE_FEATURE_AGENT_RUNTIME_ENABLED", "false").lower() in ("1", "true", "yes")


def assert_m25_enabled(ctx: ToolContext) -> None:
    if not _server_m25_enabled():
        raise ToolError(
            "M25 agent runtime is disabled at the server level",
            code="failed_precondition", status_code=412,
        )


def assert_tenant_match(ctx: ToolContext, requested_tenant_id: str) -> None:
    caller = ctx.tenant_id
    if not caller:
        raise ToolError("no tenant claim in caller context",
                        code="tenant_required", status_code=401)
    if caller != requested_tenant_id:
        raise ToolError(
            f"caller tenant '{caller}' != requested '{requested_tenant_id}'",
            code="tenant_mismatch", status_code=403,
        )


async def fetch_tenant_flags(ctx: ToolContext, tenant_id: str) -> dict[str, Any]:
    try:
        return await ctx.backend.get_tenant_features(tenant_id)
    except Exception:
        return {
            "m25_agent_runtime_enabled": False,
            "m25_dpia_signed_at": None,
            "m25_dpia_expires_at": None,
        }


def assert_dpia_active(flags: dict[str, Any]) -> None:
    if not flags.get("m25_agent_runtime_enabled", False):
        raise ToolError(
            "M25 not enabled for tenant",
            code="failed_precondition", status_code=412,
        )
    signed = flags.get("m25_dpia_signed_at")
    expires = flags.get("m25_dpia_expires_at")
    if not signed or not expires:
        raise ToolError(
            "DPIA not signed",
            code="failed_precondition", status_code=412,
        )


async def call_grpc(
    ctx: ToolContext, method: str, *args: Any, **kwargs: Any,
) -> Any:
    """Bridge to the backend AgentRuntimeService client."""
    fn = getattr(ctx.backend, f"agent_{method}", None)
    if fn is None:
        raise ToolError(
            f"backend does not expose agent_{method}",
            code="not_implemented", status_code=501,
        )
    return await fn(*args, **kwargs)
