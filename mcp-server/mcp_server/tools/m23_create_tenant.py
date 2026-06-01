"""efe_create_tenant (D8) — create a new tenant.

WRITE tool, requires `confirm:true`. Idempotent on tenant_id : returns
HTTP 409 (`code=conflict`) if the tenant already exists.

Side effects (single transaction) :
    1. INSERT tenants (name, plan, max_rps, ...)
    2. INSERT initial admin api_key (returned ONCE in plaintext)
    3. INSERT ui_audit_log entry (HMAC-chained)

Default OFF : gated by `EFE_FEATURE_D8_ADMIN_ENABLED`.

Pin tests : tests/test_m23_create_tenant.py.
"""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


_TENANT_PLANS: tuple[str, ...] = ("trial", "growth", "enterprise")
_RESIDENCIES: tuple[str, ...] = ("EU_ONLY", "US_ONLY", "any")


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeCreateTenantInput",
    "description": (
        "Create a new tenant. Idempotent on tenant_id (409 if exists). "
        "Returns the initial admin api_key in plaintext ONCE. WRITE — "
        "requires `confirm:true`. Default OFF via "
        "`EFE_FEATURE_D8_ADMIN_ENABLED`."
    ),
    "properties": {
        "name": {
            "type": "string",
            "pattern": "^[a-z0-9][a-z0-9-]{1,30}[a-z0-9]$",
            "description": "Tenant slug, 3-32 chars, [a-z0-9-]+.",
        },
        "plan": {"type": "string", "enum": list(_TENANT_PLANS), "default": "trial"},
        "max_rps": {"type": "integer", "minimum": 1, "maximum": 100000, "default": 100},
        "max_tokens_per_day": {
            "type": "integer",
            "minimum": 0,
            "maximum": 1_000_000_000,
            "default": 100_000,
        },
        "data_residency": {
            "type": "string",
            "enum": list(_RESIDENCIES),
            "default": "any",
        },
        "contact_email": {"type": ["string", "null"], "format": "email"},
        "confirm": {"type": "boolean", "const": True},
    },
    "required": ["name", "confirm"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not args.get("confirm"):
        raise ToolError(
            "create_tenant requires confirm:true",
            code="confirm_required",
        )
    if not hasattr(ctx.backend, "create_tenant"):
        raise ToolError(
            "Backend does not support create_tenant (EFE 1.6 required)",
            code="not_implemented",
        )

    name = args["name"]
    plan = args.get("plan", "trial")
    max_rps = int(args.get("max_rps", 100))
    max_tokens_per_day = int(args.get("max_tokens_per_day", 100_000))
    data_residency = args.get("data_residency", "any")
    contact_email = args.get("contact_email")

    if plan not in _TENANT_PLANS:
        raise ToolError(f"invalid plan '{plan}'", code="invalid_input")
    if data_residency not in _RESIDENCIES:
        raise ToolError(
            f"invalid data_residency '{data_residency}'", code="invalid_input"
        )

    try:
        result = await ctx.backend.create_tenant(
            name=name,
            plan=plan,
            max_rps=max_rps,
            max_tokens_per_day=max_tokens_per_day,
            data_residency=data_residency,
            contact_email=contact_email,
            auth=ctx.auth,
        )
    except getattr(ctx.backend, "ConflictError", Exception) as e:
        # Idempotency : already exists.
        raise ToolError(
            f"tenant '{name}' already exists",
            code="conflict",
            status_code=409,
        ) from e
    except Exception as e:
        raise ToolError(
            f"create_tenant failed: {e}",
            code="backend_error",
        ) from e

    return {
        "tenant_id": result.get("tenant_id", name),
        "plan": result.get("plan", plan),
        "max_rps": result.get("max_rps", max_rps),
        "data_residency": result.get("data_residency", data_residency),
        "created_at": result.get("created_at"),
        # IMPORTANT : plaintext token returned ONCE. Caller must store it.
        "initial_admin_token": result.get("initial_admin_token"),
    }


TOOL = Tool(
    name="efe_create_tenant",
    description=(
        "Create a new tenant with plan + quotas. Returns the initial "
        "admin api_key in plaintext ONCE. WRITE — requires `confirm:true`. "
        "Default OFF via `EFE_FEATURE_D8_ADMIN_ENABLED`."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=True,
    profile_tier="agent-native-deploy",
    feature_flag="D8_ADMIN_ENABLED",
)
