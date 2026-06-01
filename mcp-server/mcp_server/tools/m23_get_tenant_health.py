"""efe_get_tenant_health (D8) — read-only tenant health snapshot.

Aggregates :
    - DB liveness (SELECT 1)
    - Redis liveness (PING)
    - LLM provider breaker state (M21 + M22 if enabled)
    - Recent classify metrics (RPS, p99, error rate — last 5 min)
    - Compliance status (AI Act + GDPR attestation, expiration)
    - Mode-3 review queue depth (pending count)
    - Cost today (USD spend vs daily quota)
    - Last classify timestamp

READ-only, no `confirm` needed. Profile : agent-native (read tier).

Pin tests : tests/test_m23_get_tenant_health.py.
"""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeGetTenantHealthInput",
    "description": (
        "Read-only health snapshot for a tenant. Aggregates DB/Redis "
        "liveness, breaker states, recent metrics, compliance status, "
        "review queue depth, cost. Used by efe-admin smoke-test."
    ),
    "properties": {
        "tenant_id": {"type": "string", "pattern": "^[a-z0-9][a-z0-9-]{1,30}[a-z0-9]$"},
    },
    "required": ["tenant_id"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not hasattr(ctx.backend, "get_tenant_health"):
        raise ToolError(
            "Backend does not support get_tenant_health (EFE 1.6 required)",
            code="not_implemented",
        )

    tenant = args["tenant_id"]

    try:
        h = await ctx.backend.get_tenant_health(tenant_id=tenant, auth=ctx.auth)
    except getattr(ctx.backend, "NotFoundError", Exception) as e:
        raise ToolError(
            f"tenant '{tenant}' not found",
            code="not_found",
            status_code=404,
        ) from e
    except Exception as e:
        raise ToolError(
            f"get_tenant_health failed: {e}",
            code="backend_error",
        ) from e

    # Defensive structure : every field has a sensible default if the
    # backend trims it. Caller (CLI) chooses what to display.
    return {
        "tenant_id": tenant,
        "status": h.get("status", "unknown"),  # healthy | degraded | down | suspended
        "checks": {
            "db": h.get("checks", {}).get("db", {"status": "unknown"}),
            "redis": h.get("checks", {}).get("redis", {"status": "unknown"}),
            "llm_breaker": h.get("checks", {}).get("llm_breaker", {"status": "unknown"}),
            "audit_chain": h.get("checks", {}).get("audit_chain", {"status": "unknown"}),
        },
        "metrics": {
            "rps_last_5m": h.get("metrics", {}).get("rps_last_5m"),
            "p99_ms_last_5m": h.get("metrics", {}).get("p99_ms_last_5m"),
            "error_rate_last_5m": h.get("metrics", {}).get("error_rate_last_5m"),
        },
        "compliance": {
            "ai_act_art14_acknowledged": h.get("compliance", {}).get(
                "ai_act_art14_acknowledged", False
            ),
            "gdpr_art22_acknowledged": h.get("compliance", {}).get(
                "gdpr_art22_acknowledged", False
            ),
            "expires_in_days": h.get("compliance", {}).get("expires_in_days"),
        },
        "pending_reviews": h.get("pending_reviews", 0),
        "cost_today_usd": h.get("cost_today_usd"),
        "daily_cost_cap_usd": h.get("daily_cost_cap_usd"),
        "last_classify_at": h.get("last_classify_at"),
    }


TOOL = Tool(
    name="efe_get_tenant_health",
    description=(
        "Read-only health snapshot for a tenant. Aggregates DB/Redis "
        "liveness, breaker states, recent metrics, compliance, review "
        "queue, cost. Used by efe-admin smoke-test."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=False,
    profile_tier="agent-native",
    feature_flag="D8_ADMIN_ENABLED",
)
