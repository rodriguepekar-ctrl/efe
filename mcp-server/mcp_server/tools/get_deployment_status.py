"""efe_get_deployment_status (M18 D49).

Read-only snapshot of the active EFE deployment : currently-deployed
bundle, helm release, replicas, health, uptime. Backend-mediated
(EFE 1.2 endpoint) ; falls back to `/info`-derived metadata when the
backend is older.
"""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeGetDeploymentStatusInput",
    "description": (
        "Return the active EFE deployment metadata : bundle, helm chart, "
        "replicas, health. Read-only."
    ),
    "properties": {},
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    # 1) EFE 1.2 path
    try:
        result = await ctx.backend.get_deployment_current(auth=ctx.auth)
        result.setdefault("source", "backend")
        return result
    except NotImplementedError:
        pass

    # 2) Fallback : compose from /info + /admin/bundle-info (M15-A5)
    info = await ctx.backend.get_info(auth=ctx.auth)
    if not info:
        raise ToolError(
            "backend does not expose /info ; cannot derive deployment status",
            code="not_supported", status_code=501,
        )

    # Audit fix M18-D49-A6 (LOW) : use the public abstract method
    # instead of `ctx.backend._request(...)` directly. This keeps the
    # `BackendClient` abstraction intact for the gRPC backend planned
    # in M19 (which won't have a `_request` method at all).
    bundle_info: dict[str, Any] = {}
    try:
        bundle_info = await ctx.backend.get_admin_bundle_info(auth=ctx.auth)
    except NotImplementedError:
        bundle_info = {}
    except Exception:  # noqa: BLE001
        bundle_info = {}

    embedder = info.get("embedder") or {}
    return {
        "source": "fallback_info_compose",
        "deployed_at": None,
        "deployed_by": None,
        "bundle": {
            "slug": bundle_info.get("bundle_slug")
            or embedder.get("bundle_slug")
            or "unknown",
            "version": bundle_info.get("bundle_id") or "unknown",
            "manifest_sha256": bundle_info.get("manifestSha256"),
        },
        "helm": {
            "chart_version": info.get("helm_chart_version"),
            "release_name": info.get("release_name"),
            "namespace": info.get("namespace"),
        },
        "replicas": {
            "desired": None, "ready": None, "available": None,
        },
        "health": "unknown",
        "uptime_seconds": None,
        "warnings": [
            "this is a fallback view ; deployment status fields are partial. "
            "Upgrade backend to EFE 1.2 for full status."
        ],
    }


TOOL = Tool(
    name="efe_get_deployment_status",
    description=(
        "Read-only snapshot of the active EFE deployment : bundle, helm "
        "release, replicas, health, uptime. Falls back to /info-derived "
        "metadata on EFE 1.1 backends."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    profile_tier="agent-native",
)
