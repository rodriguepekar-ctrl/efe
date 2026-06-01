"""efe_list_available_bundles (M18 D47).

Enumerate bundles available in the configured OCI registry. Two paths:

  1. **Backend-mediated** (EFE 1.2) : `GET /admin/bundles` proxies the
     OCI registry list, returns annotated bundle records.
  2. **Direct OCI** (fallback) : pull `/v2/<repo>/tags/list` via the
     `mcp_server/bundle/oci.py` client. Strict allowlist enforced.
"""
from __future__ import annotations

import fnmatch
import os
from typing import Any

from ..bundle import (
    InvalidRegistryArg,
    OCIArtifactNotFound,
    OCIRegistryClient,
    OCIRegistryError,
    parse_registry_allowlist,
    split_registry,
)
from ..bundle.registry_arg import default_oci_factory
from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeListAvailableBundlesInput",
    "description": "Enumerate bundles available in the configured OCI registry.",
    "properties": {
        "registry": {"type": "string", "minLength": 1, "maxLength": 200},
        "filter_slug": {"type": "string", "maxLength": 100},
        "limit": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50},
    },
    "additionalProperties": False,
}


def _matches_filter(slug: str, filter_pattern: str | None) -> bool:
    if not filter_pattern:
        return True
    return fnmatch.fnmatchcase(slug, filter_pattern)


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    registry = (
        args.get("registry")
        or os.environ.get("EFE_OCI_DEFAULT_REGISTRY")
    )
    filter_slug = args.get("filter_slug")
    limit = int(args.get("limit", 50))

    # 1) Try EFE 1.2 backend-mediated listing first.
    try:
        result = await ctx.backend.list_known_bundles(
            registry_filter=registry,
            slug_filter=filter_slug,
            limit=limit,
            auth=ctx.auth,
        )
        bundles = result.get("bundles", [])
        return {
            "registry": registry,
            "count": len(bundles),
            "bundles": bundles[:limit],
            "next_cursor": result.get("next_cursor"),
            "source": "backend",
        }
    except NotImplementedError:
        # Fallback path : direct OCI registry. Requires `registry` arg
        # (or env default) since the backend can't tell us.
        pass

    if not registry:
        raise ToolError(
            "no registry specified ; set EFE_OCI_DEFAULT_REGISTRY or pass `registry` arg",
            code="invalid_arguments",
            status_code=400,
        )
    return await _list_via_oci(
        registry=registry, filter_slug=filter_slug, limit=limit, ctx=ctx,
    )


async def _list_via_oci(
    *, registry: str, filter_slug: str | None, limit: int, ctx: ToolContext,
) -> dict[str, Any]:
    """Direct OCI tag listing — interprets `registry` as `host/repo`.

    Audit fix M18-D47-A4 : `split_registry()` strips `https://` scheme.
    Audit fix M18-D47-A1 : factory receives the full URL so allowlist
    enforcement covers the path scope, not just the host.
    """
    try:
        _host, repo, full_url = split_registry(registry)
    except InvalidRegistryArg as exc:
        raise ToolError(
            str(exc), code="invalid_arguments", status_code=400,
        ) from exc

    factory = ctx.oci_factory or default_oci_factory
    try:
        async with factory(full_url) as client:  # type: ignore[union-attr]
            tags = await client.list_tags(repo=repo, n=limit)
    except OCIRegistryError as exc:
        raise ToolError(
            f"OCI registry list failed : {exc}",
            code=_oci_error_to_code(exc),
            status_code=getattr(exc, "status_code", None),
        ) from exc

    # Tags may be of the form `slug-vMAJ.MIN.PATCH` or `slug:vX.Y.Z`.
    # We don't try to parse here — just return the raw list filtered.
    bundles = [
        {"slug": tag, "version": None}
        for tag in tags
        if _matches_filter(tag, filter_slug)
    ][:limit]

    return {
        "registry": registry,
        "count": len(bundles),
        "bundles": bundles,
        "next_cursor": None,
        "source": "oci_direct",
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
    name="efe_list_available_bundles",
    description=(
        "Enumerate bundles available in the OCI registry (or via the "
        "backend's bundle catalog). Returns slug + version + signing "
        "summary. Read-only ; respects the EFE_OCI_REGISTRY_ALLOWLIST."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    profile_tier="agent-native",
)
