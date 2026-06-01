"""Common helpers for the M24 active learning MCP tools."""
from __future__ import annotations

import os
from typing import Any

from ..types import ToolContext, ToolError


def _server_m24_enabled() -> bool:
    return os.environ.get("EFE_FEATURE_M24_ACTIVE_LEARNING_ENABLED", "false").lower() in ("1", "true", "yes")


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


def assert_m24_enabled(ctx: ToolContext) -> None:
    if not _server_m24_enabled():
        raise ToolError(
            "M24 active learning is disabled at the server level",
            code="failed_precondition", status_code=412,
        )


async def fetch_tenant_flags(ctx: ToolContext, tenant_id: str) -> dict[str, Any]:
    try:
        return await ctx.backend.get_tenant_features(tenant_id)
    except Exception:
        return {
            "m24_active_learning_enabled": False,
            "m24_auto_promotion_enabled": False,
            "m24_human_review_required": True,
            "m24_dpo_consent_active": False,
        }


async def call_grpc(
    ctx: ToolContext, method: str, *args: Any, **kwargs: Any,
) -> Any:
    """Bridge to the backend ActiveLearningService client."""
    fn = getattr(ctx.backend, f"al_{method}", None)
    if fn is None:
        raise ToolError(
            f"backend does not expose al_{method}",
            code="not_implemented", status_code=501,
        )
    return await fn(*args, **kwargs)
