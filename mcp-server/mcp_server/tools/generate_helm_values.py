"""efe_generate_helm_values (M18 D49).

Produce a Helm `values.yaml` body for a target bundle. Output is plain
text the operator copy-pastes / saves to `values-mcp.yaml` and runs
`helm upgrade -f` against. **The MCP server NEVER applies it.**

TM-M18-E-02 mitigation : whitelist enforced inside `mcp_server.deploy.helm_values.render`.
"""
from __future__ import annotations

from typing import Any

from ..deploy import HelmValuesInput, render_helm_values
from .inspect_bundle import handler as _inspect_handler
from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeGenerateHelmValuesInput",
    "description": (
        "Generate a Helm values.yaml body for slug:version. The output "
        "is plain text — never applied. TM-M18-E-02 enforces a strict "
        "whitelist of emitted keys."
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
        "replicas": {
            "type": "integer", "minimum": 1, "maximum": 100, "default": 3,
        },
        "include_resources": {"type": "boolean", "default": True},
        "include_pdb": {"type": "boolean", "default": True},
        "autoscaling_enabled": {"type": "boolean", "default": False},
        "autoscaling_min": {"type": "integer", "minimum": 1, "maximum": 100, "default": 1},
        "autoscaling_max": {"type": "integer", "minimum": 1, "maximum": 100, "default": 10},
        "cpu_request": {"type": "string", "maxLength": 32, "default": "500m"},
        "cpu_limit":   {"type": "string", "maxLength": 32, "default": "2000m"},
        "memory_request": {"type": "string", "maxLength": 32, "default": "1Gi"},
        "memory_limit":   {"type": "string", "maxLength": 32, "default": "4Gi"},
        "registry": {"type": "string", "minLength": 1, "maxLength": 200},
    },
    "required": ["slug", "version"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    # Pull the manifest_sha256 via inspect (so the operator's values.yaml
    # pins a specific bundle artifact, defending against tag-rewrite).
    inspect_args: dict[str, Any] = {"slug": args["slug"], "version": args["version"]}
    if args.get("registry"):
        inspect_args["registry"] = args["registry"]
    target_info = await _inspect_handler(inspect_args, ctx)
    manifest = target_info.get("manifest") or {}
    manifest_sha = manifest.get("manifestSha256")
    if not manifest_sha:
        raise ToolError(
            "could not determine target bundle manifest_sha256 ; cannot pin",
            code="invalid_arguments", status_code=502,
        )

    autoscaling_min = int(args.get("autoscaling_min", 1))
    autoscaling_max = int(args.get("autoscaling_max", 10))
    if autoscaling_min > autoscaling_max:
        raise ToolError(
            f"autoscaling_min ({autoscaling_min}) > "
            f"autoscaling_max ({autoscaling_max})",
            code="invalid_arguments", status_code=400,
        )

    helm_input = HelmValuesInput(
        bundle_slug=args["slug"],
        bundle_version=args["version"],
        bundle_manifest_sha256=manifest_sha,
        replica_count=int(args.get("replicas", 3)),
        cpu_request=args.get("cpu_request", "500m"),
        cpu_limit=args.get("cpu_limit", "2000m"),
        memory_request=args.get("memory_request", "1Gi"),
        memory_limit=args.get("memory_limit", "4Gi"),
        pdb_enabled=bool(args.get("include_pdb", True)),
        pdb_min_available=2,
        autoscaling_enabled=bool(args.get("autoscaling_enabled", False)),
        autoscaling_min=autoscaling_min,
        autoscaling_max=autoscaling_max,
        include_resources=bool(args.get("include_resources", True)),
        include_pdb=bool(args.get("include_pdb", True)),
    )
    body = render_helm_values(helm_input)
    return {
        "values_yaml": body,
        "size_bytes": len(body.encode("utf-8")),
        "manifest_sha256": manifest_sha,
        "next_action_hint": (
            f"Save the values_yaml field to values-mcp.yaml and run :\n"
            f"  helm upgrade efe ./charts/efe -f values-mcp.yaml --dry-run\n"
            f"to preview the change. Once validated, drop --dry-run.\n"
            f"The MCP server NEVER runs helm itself."
        ),
    }


TOOL = Tool(
    name="efe_generate_helm_values",
    description=(
        "Generate a Helm values.yaml body for slug:version. Returns "
        "plain text — never applied. Whitelist-only sections (bundle, "
        "replicaCount, resources, podDisruptionBudget, autoscaling, image)."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    profile_tier="agent-native",
)
