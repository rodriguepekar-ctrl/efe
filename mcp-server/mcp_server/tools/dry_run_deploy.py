"""efe_dry_run_deploy (M18 D49).

Compute the semantic diff between the current deployment and a target
bundle without applying anything. Two paths :
  1. EFE 1.2 backend-mediated (`POST /admin/deployments/dry-run`) for
     full server-side compute (catalog drift estimates, etc.).
  2. Local fallback via `mcp_server/deploy/dry_run.compute_diff`,
     composing `inspect_bundle` + `get_deployment_current`.

The output is identical-shape across both paths so the agent can
treat the result uniformly.
"""
from __future__ import annotations

from typing import Any

from ..deploy import compute_diff
from .inspect_bundle import handler as _inspect_handler
from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeDryRunDeployInput",
    "description": (
        "Simulate a deployment of `slug:version` and return a semantic "
        "diff vs the current deployment. Read-only ; never applies."
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
        "environment": {"type": "string", "maxLength": 50, "default": "production"},
        "registry": {"type": "string", "minLength": 1, "maxLength": 200},
        "rolling_update": {"type": "boolean", "default": True},
        "desired_replicas": {
            "type": "integer", "minimum": 1, "maximum": 100, "default": 3,
        },
    },
    "required": ["slug", "version"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    slug = args["slug"]
    version = args["version"]
    environment = args.get("environment", "production")
    desired_replicas = int(args.get("desired_replicas", 3))
    rolling = bool(args.get("rolling_update", True))

    # 1) Backend-mediated path. We always inspect the target locally
    # first to get the manifest_sha256 ; this gives us the correct
    # value to pass to the backend (defends against backend bugs that
    # might mis-fetch the manifest).
    inspect_args: dict[str, Any] = {"slug": slug, "version": version}
    if args.get("registry"):
        inspect_args["registry"] = args["registry"]
    target_info = await _inspect_handler(inspect_args, ctx)
    target_manifest = target_info.get("manifest", {})
    target_sha = target_manifest.get("manifestSha256")

    # Audit fix M18-D49-A7 (HIGH) : map inspect_bundle's signature
    # summary to a tri-state for compute_diff. `verified=False` from
    # inspect can mean (a) cryptographic failure, (b) trust store
    # missing, (c) crypto extra missing, (d) no signature artifact.
    # Only (a) is a BLOCKER ; (b)/(c)/(d) are MEDIUM "unverifiable".
    sig_data = target_info.get("signature") or {}
    sig_verified = sig_data.get("verified")
    sig_error = (sig_data.get("error") or "").lower()
    _UNVERIFIABLE_MARKERS = (
        "trust_store_unavailable",
        "cryptography extra not installed",
        "no signature artifact",
        "verification unavailable",
        "unsigned_bundle",
    )
    sig_unverifiable = (
        sig_verified is not True
        and any(marker in sig_error for marker in _UNVERIFIABLE_MARKERS)
    )

    try:
        backend_result = await ctx.backend.dry_run_deployment(
            target_slug=slug,
            target_version=version,
            target_manifest_sha256=target_sha,
            environment=environment,
            auth=ctx.auth,
        )
        backend_result.setdefault("source", "backend")
        return backend_result
    except NotImplementedError:
        pass

    # 2) Local fallback : compute the diff in-process.
    try:
        current = await ctx.backend.get_deployment_current(auth=ctx.auth)
    except NotImplementedError:
        current = None
    except Exception:  # noqa: BLE001 — defensive
        current = None

    diff = compute_diff(
        current=current,
        target_manifest={
            "slug": slug,
            "version": version,
            **target_manifest,
        },
        target_signature_verified=sig_verified,
        target_signature_unverifiable=sig_unverifiable,
        desired_replicas=desired_replicas,
        rolling_update=rolling,
    )
    out = diff.to_dict()
    out["source"] = "fallback_local"
    out["environment"] = environment
    return out


TOOL = Tool(
    name="efe_dry_run_deploy",
    description=(
        "Simulate the deployment of slug:version against the current "
        "deployment. Returns manifest_changed, expected_replica_restarts, "
        "risks (LOW/MEDIUM/HIGH/BLOCKER), blocking_issues, and a "
        "recommended_actions list. Read-only ; never applies."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    profile_tier="agent-native",
)
