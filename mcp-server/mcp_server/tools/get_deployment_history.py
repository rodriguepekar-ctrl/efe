"""efe_get_deployment_history (M18 D49).

Return the most recent N deployments. EFE 1.2 backend-only ; no
fallback because the data does not exist anywhere else.
"""
from __future__ import annotations

from typing import Any

from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeGetDeploymentHistoryInput",
    "description": "Return the most recent N deployments (newest first).",
    "properties": {
        "limit": {
            "type": "integer", "minimum": 1, "maximum": 100, "default": 10,
        },
    },
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    limit = int(args.get("limit", 10))
    try:
        deployments = await ctx.backend.get_deployment_history(
            limit=limit, auth=ctx.auth,
        )
    except NotImplementedError:
        raise ToolError(
            "deployment history requires an EFE 1.2 backend "
            "(GET /admin/deployments/history is not exposed)",
            code="not_supported", status_code=501,
        )
    return {
        "count": len(deployments),
        "limit": limit,
        "deployments": deployments,
    }


TOOL = Tool(
    name="efe_get_deployment_history",
    description=(
        "Return the most recent N deployments (newest first) : "
        "deployed_at, deployed_by, bundle slug+version, outcome, "
        "duration. Requires EFE 1.2 backend. Read-only."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    profile_tier="agent-native",
)
