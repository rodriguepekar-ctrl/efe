"""efe_generate_rollback_plan (M18 D49).

Produce an emergency-rollback runbook (text only). Pre-checks include
audit chain integrity verification (D14-bis carry-over). The MCP
server NEVER executes helm/kubectl ; the operator copy-pastes the
commands under their own credentials.
"""
from __future__ import annotations

from typing import Any

from ..deploy import generate_rollback_plan
from .types import Tool, ToolContext


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeGenerateRollbackPlanInput",
    "description": (
        "Generate an emergency-rollback runbook : pre-checks, helm "
        "commands, post-checks. Read-only ; never executes."
    ),
    "properties": {
        # Audit fix M18-D49-A5 (HIGH) : strict patterns prevent shell
        # metacharacter injection through values that end up embedded
        # in the runbook commands the operator copy-pastes.
        "to_version": {
            "type": "string",
            # `previous` literal OR semver vX.Y.Z[-tag]
            "pattern": r"^(previous|v\d{1,5}\.\d{1,5}\.\d{1,5}(-[a-z0-9\-]{1,15})?)$",
            "default": "previous",
        },
        "environment": {
            "type": "string",
            # lowercase alphanumeric + hyphen, 1-50 chars
            "pattern": r"^[a-z0-9][a-z0-9\-]{0,49}$",
            "default": "production",
        },
        "namespace": {
            "type": "string",
            # Kubernetes DNS-1123 label : lowercase alphanumeric +
            # hyphen, must start AND end with alphanumeric, max 63.
            "pattern": r"^[a-z0-9]([-a-z0-9]{0,61}[a-z0-9])?$",
            "default": "efe-prod",
        },
        "release_name": {
            "type": "string",
            # Helm release name (DNS-1123 subdomain rules, capped at
            # 53 to leave room for helm-generated suffixes like
            # `-test-connection`).
            "pattern": r"^[a-z0-9]([-a-z0-9]{0,52}[a-z0-9])?$",
            "default": "efe",
        },
        "reason": {
            # Free text but no CR/LF (which could break runbook
            # markdown structure) ; bounded length.
            "type": "string",
            "pattern": r"^[^\r\n]{1,500}$",
            "minLength": 1, "maxLength": 500,
        },
    },
    "required": ["reason"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    to_version = args.get("to_version", "previous")
    environment = args.get("environment", "production")
    namespace = args.get("namespace", "efe-prod")
    release_name = args.get("release_name", "efe")
    reason = args["reason"]

    # Best-effort : pull current deployment to populate from_version
    # in the plan. If the backend is too old, fall through with None.
    from_version = "unknown"
    from_sha: str | None = None
    try:
        current = await ctx.backend.get_deployment_current(auth=ctx.auth)
        from_version = (current.get("bundle") or {}).get("version") or "unknown"
        from_sha = (current.get("bundle") or {}).get("manifest_sha256")
    except NotImplementedError:
        pass
    except Exception:  # noqa: BLE001
        pass

    plan = generate_rollback_plan(
        from_version=from_version,
        to_version=to_version,
        environment=environment,
        namespace=namespace,
        release_name=release_name,
        reason=reason,
        from_manifest_sha256=from_sha,
        to_manifest_sha256=None,    # filled by operator after lookup
        helm_history_lookup=True,
    )
    return {
        "from_version": plan.from_version,
        "to_version": plan.to_version,
        "from_manifest_sha256": plan.from_manifest_sha256,
        "to_manifest_sha256": plan.to_manifest_sha256,
        "environment": plan.environment,
        "reason": plan.reason,
        "pre_checks": plan.pre_checks,
        "rollback_commands": plan.rollback_commands,
        "post_checks": plan.post_checks,
        "estimated_duration_seconds": plan.estimated_duration_seconds,
        "blast_radius": plan.blast_radius,
        "warnings": [
            "The MCP server NEVER runs helm/kubectl. Operator must "
            "execute the commands under their own credentials.",
        ],
    }


TOOL = Tool(
    name="efe_generate_rollback_plan",
    description=(
        "Generate an emergency-rollback runbook (text). Includes "
        "pre-checks (signature, audit chain), the helm commands the "
        "operator must run, and post-checks. NEVER executes anything."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    profile_tier="agent-native",
)
