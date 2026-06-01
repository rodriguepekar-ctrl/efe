"""efe_promote_bundle (M18 D50, write).

Mark a registered bundle as 'current' for the given scope (tenant or
global). **NOT a deployment** — this is metadata-only ; the cluster
state is not touched. The operator must still run `helm upgrade` to
actually roll the new bundle.

Confirm-required + agent-native-deploy profile only.
"""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfePromoteBundleInput",
    "description": (
        "Mark a registered bundle as 'current' for tenant or global "
        "scope. METADATA-ONLY ; never deploys. WRITE — requires confirm:true. "
        "Profile=agent-native-deploy only."
    ),
    "properties": {
        "slug": {
            "type": "string",
            "pattern": r"^[a-z][a-z0-9\-]{0,99}$",
            "minLength": 1, "maxLength": 100,
        },
        "version": {
            "type": "string",
            "pattern": r"^v\d+\.\d+\.\d+(-[a-z0-9\-]{1,15})?$",
            "minLength": 1, "maxLength": 50,
        },
        "scope": {
            "type": "string",
            "enum": ["tenant", "global"],
            "default": "tenant",
        },
        "idempotency_key": {
            "type": "string",
            "pattern": r"^[A-Za-z0-9_\-]{1,64}$",
            "maxLength": 64,
        },
        "confirm": {"type": "boolean"},
    },
    "required": ["slug", "version", "confirm"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not args.get("confirm"):
        raise ToolError(
            "promote_bundle requires confirm:true (write operation)",
            code="confirm_required",
        )

    slug = args["slug"]
    version = args["version"]
    scope = args.get("scope", "tenant")
    idempotency_key = args.get("idempotency_key")

    try:
        result = await ctx.backend.promote_bundle(
            slug=slug, version=version, scope=scope,
            idempotency_key=idempotency_key, auth=ctx.auth,
        )
    except NotImplementedError:
        raise ToolError(
            "promote_bundle requires an EFE 1.2 backend "
            "(POST /admin/bundles/<slug>/<version>/promote not exposed)",
            code="not_supported", status_code=501,
        )

    return {
        "promoted": True,
        "scope": scope,
        "slug": slug,
        "version": version,
        "tenant_id": result.get("tenant_id"),
        "previous_current_version": result.get("previous_current_version"),
        "new_current_version": version,
        "audit_event_id": result.get("audit_event_id"),
        "next_action_hint": (
            "Bundle metadata updated. To actually deploy this version, "
            "the operator must run :\n"
            f"  helm upgrade efe ./charts/efe \\\n"
            f"    --set bundle.slug={slug} \\\n"
            f"    --set bundle.version={version}\n"
            "Run efe_dry_run_deploy first to preview the change. "
            "The MCP server NEVER applies kubectl/helm itself."
        ),
    }


TOOL = Tool(
    name="efe_promote_bundle",
    description=(
        "Mark a registered bundle as 'current' for the caller's tenant "
        "(or globally with scope='global'). METADATA-ONLY — does NOT "
        "deploy ; the operator must run helm upgrade. WRITE — requires "
        "confirm:true. Profile=agent-native-deploy only."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=True,
    profile_tier="agent-native-deploy",
)
