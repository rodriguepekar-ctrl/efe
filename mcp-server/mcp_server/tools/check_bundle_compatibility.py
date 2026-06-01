"""efe_check_bundle_compatibility (M18 D47).

Cross-check a bundle's annotations against the live EFE backend `/info`.
"""
from __future__ import annotations

import os
from typing import Any

from ..bundle import (
    InvalidRegistryArg,
    OCIArtifactNotFound,
    OCIRegistryClient,
    OCIRegistryError,
    check_compatibility,
    parse_manifest,
    parse_registry_allowlist,
    split_registry,
)
from ..bundle.registry_arg import default_oci_factory
from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeCheckBundleCompatibilityInput",
    "description": "Cross-check a bundle's manifest against the live EFE server.",
    "properties": {
        "slug": {
            "type": "string", "pattern": r"^[a-z][a-z0-9\-]{0,99}$",
            "minLength": 1, "maxLength": 100,
        },
        "version": {
            "type": "string", "pattern": r"^v\d+\.\d+\.\d+(-[a-z0-9\-]{1,15})?$",
            "minLength": 1, "maxLength": 50,
        },
        "target_environment": {"type": "string", "maxLength": 50, "default": "production"},
        "registry": {"type": "string", "minLength": 1, "maxLength": 200},
    },
    "required": ["slug", "version"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    slug = args["slug"]
    version = args["version"]
    target_env = args.get("target_environment", "production")
    registry = (
        args.get("registry")
        or os.environ.get("EFE_OCI_DEFAULT_REGISTRY")
    )
    if not registry:
        raise ToolError(
            "no registry specified ; set EFE_OCI_DEFAULT_REGISTRY or pass `registry` arg",
            code="invalid_arguments", status_code=400,
        )

    try:
        _host, repo, full_url = split_registry(registry)
    except InvalidRegistryArg as exc:
        raise ToolError(
            str(exc), code="invalid_arguments", status_code=400,
        ) from exc

    factory = ctx.oci_factory or default_oci_factory
    try:
        async with factory(full_url) as client:
            manifest_bytes, _, manifest_sha = await client.get_manifest(
                repo=repo, reference=f"{slug}-{version}",
            )
            try:
                bundle_manifest = parse_manifest(
                    raw_bytes=manifest_bytes, manifest_sha256=manifest_sha,
                )
            except ValueError as exc:
                raise ToolError(
                    f"failed to parse OCI manifest: {exc}",
                    code="invalid_arguments", status_code=502,
                ) from exc
    except OCIArtifactNotFound as exc:
        raise ToolError(
            f"bundle {slug}:{version} not found in {registry}",
            code="not_found", status_code=404,
        ) from exc
    except OCIRegistryError as exc:
        raise ToolError(
            f"OCI registry error: {exc}",
            code=_oci_error_to_code(exc),
            status_code=getattr(exc, "status_code", None),
        ) from exc

    info = await ctx.backend.get_info(auth=ctx.auth)
    server_version = (
        info.get("server_version")
        or info.get("contract_version")
        or "1.0.0"
    )
    embedder = info.get("embedder") or {}
    server_dim = embedder.get("dim")
    server_features = info.get("features") or info.get("supported_features") or []
    if isinstance(server_features, dict):
        # Some backends expose features as {name: enabled?}
        server_features = [k for k, v in server_features.items() if v]

    check = check_compatibility(
        manifest=bundle_manifest,
        efe_server_version=str(server_version),
        efe_server_features=list(server_features),
        efe_server_dim=server_dim,
    )

    return {
        "slug": slug,
        "version": version,
        "target_environment": target_env,
        "compatible": check.compatible,
        "checks": [
            {
                "name": c.name,
                "required": c.required,
                "actual": c.actual,
                "ok": c.ok,
                "message": c.message,
            }
            for c in check.checks
        ],
        "warnings": list(check.warnings),
        "blocking_issues": list(check.blocking_issues),
        "server_version": str(server_version),
        "server_dim": server_dim,
        "server_features": list(server_features),
    }


def _oci_error_to_code(exc: OCIRegistryError) -> str:
    from ..bundle import OCIAccessForbidden, OCIRegistryNotInAllowlist
    if isinstance(exc, OCIRegistryNotInAllowlist):
        return "forbidden"
    if isinstance(exc, OCIAccessForbidden):
        return "forbidden"
    if isinstance(exc, OCIArtifactNotFound):
        return "not_found"
    return "backend_unavailable"


TOOL = Tool(
    name="efe_check_bundle_compatibility",
    description=(
        "Cross-check a bundle's required version + dim + features against "
        "the live EFE backend /info. Returns compatible:true with a list of "
        "checks + any blocking issues. Read-only."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    profile_tier="agent-native",
)
