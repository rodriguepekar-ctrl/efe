"""efe_request_deployment (M18 D50, write).

File a deployment-request ticket with the configured tracker
(Linear / Jira / GitHub Issue / internal). The backend wraps the
third-party API under the organisation's existing credentials ; the
MCP server itself NEVER holds tracker secrets or creates tickets
directly.

This is the **closest M18 ever gets to "deploy"** : it creates a ticket
for human review. The operator still has to run `helm upgrade` after
the ticket is approved.

Confirm-required + agent-native-deploy profile only.
"""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


_ALLOWED_TRACKERS = ("linear", "jira", "github", "internal")


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeRequestDeploymentInput",
    "description": (
        "File a deployment-request ticket with the configured tracker. "
        "The backend creates the ticket under the org's credentials ; "
        "the MCP server NEVER holds tracker secrets. WRITE — requires "
        "confirm:true. Profile=agent-native-deploy only."
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
        "environment": {
            "type": "string",
            # Same DNS-1123-ish pattern as rollback_plan (M18-D49-A5).
            "pattern": r"^[a-z0-9][a-z0-9\-]{0,49}$",
            "default": "production",
        },
        "rationale": {
            "type": "string",
            # Free text but no CR/LF (anti markdown injection in ticket
            # body) ; bounded length to fit typical ticket-system limits.
            "pattern": r"^[^\r\n]{10,2000}$",
            "minLength": 10, "maxLength": 2000,
        },
        "tracker": {
            "type": "string",
            "enum": list(_ALLOWED_TRACKERS),
        },
        "idempotency_key": {
            "type": "string",
            "pattern": r"^[A-Za-z0-9_\-]{1,64}$",
            "maxLength": 64,
        },
        "confirm": {"type": "boolean"},
    },
    "required": ["slug", "version", "rationale", "tracker", "confirm"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    if not args.get("confirm"):
        raise ToolError(
            "request_deployment requires confirm:true (write operation)",
            code="confirm_required",
        )

    slug = args["slug"]
    version = args["version"]
    environment = args.get("environment", "production")
    rationale = args["rationale"]
    tracker = args["tracker"]
    idempotency_key = args.get("idempotency_key")

    try:
        result = await ctx.backend.request_deployment_ticket(
            slug=slug, version=version,
            environment=environment, rationale=rationale,
            tracker=tracker, idempotency_key=idempotency_key,
            auth=ctx.auth,
        )
    except NotImplementedError:
        raise ToolError(
            "request_deployment requires an EFE 1.2 backend "
            "(POST /admin/tickets not exposed)",
            code="not_supported", status_code=501,
        )

    return {
        "requested": True,
        "tracker": tracker,
        "ticket_id": result.get("ticket_id"),
        "ticket_url": result.get("ticket_url"),
        "status": result.get("status", "open"),
        "slug": slug,
        "version": version,
        "environment": environment,
        "audit_event_id": result.get("audit_event_id"),
        "next_action_hint": (
            "Deployment ticket filed. A human reviewer must approve. "
            "Once approved, the operator runs efe_dry_run_deploy + "
            "efe_generate_helm_values + helm upgrade manually. "
            "The MCP server NEVER deploys directly."
        ),
    }


TOOL = Tool(
    name="efe_request_deployment",
    description=(
        "File a deployment-request ticket (Linear / Jira / GitHub / "
        "internal) for human review. The MCP server creates the ticket "
        "via the backend ; it NEVER deploys. WRITE — requires "
        "confirm:true. Profile=agent-native-deploy only."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    is_write=True,
    profile_tier="agent-native-deploy",
)
