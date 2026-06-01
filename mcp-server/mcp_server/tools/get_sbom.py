"""efe_get_sbom (M18 D48).

Pull + parse a bundle's CycloneDX SBOM, optionally filtered by
severity. The full component list is returned for inspection ; for
huge bundles the `max_components` cap keeps the response bounded
(input schema enforces 1..5000).

Distinct from `efe_inspect_bundle` (which returns only a summary) :
this tool gives the agent the full list of components + vulnerabilities
so it can grep for specific CVEs or analyse dependency graphs.
"""
from __future__ import annotations

import os
from typing import Any

from ..bundle import (
    InvalidRegistryArg,
    OCIArtifactNotFound,
    OCIRegistryClient,
    OCIRegistryError,
    filter_by_severity,
    parse_manifest,
    parse_sbom,
    split_registry,
)
from ..bundle.registry_arg import default_oci_factory
from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeGetSbomInput",
    "description": (
        "Pull + parse a bundle's CycloneDX SBOM. Returns components + "
        "vulnerabilities, optionally filtered by minimum severity."
    ),
    "properties": {
        "slug": {
            "type": "string",
            "pattern": r"^[a-z][a-z0-9\-]{0,99}$",
            "minLength": 1,
            "maxLength": 100,
        },
        "version": {
            "type": "string",
            "pattern": r"^v\d+\.\d+\.\d+(-[a-z0-9\-]{1,15})?$",
            "minLength": 1,
            "maxLength": 50,
        },
        "filter_severity": {
            "type": "string",
            "enum": ["LOW", "MEDIUM", "HIGH", "CRITICAL"],
        },
        "max_components": {
            "type": "integer",
            "minimum": 1,
            "maximum": 5000,
            "default": 200,
        },
        "registry": {"type": "string", "minLength": 1, "maxLength": 200},
    },
    "required": ["slug", "version"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    slug = args["slug"]
    version = args["version"]
    filter_sev = args.get("filter_severity")
    max_components = int(args.get("max_components", 200))
    registry = (
        args.get("registry")
        or os.environ.get("EFE_OCI_DEFAULT_REGISTRY")
    )
    if not registry:
        raise ToolError(
            "no registry specified ; set EFE_OCI_DEFAULT_REGISTRY or pass `registry` arg",
            code="invalid_arguments",
            status_code=400,
        )
    try:
        _host, repo, full_url = split_registry(registry)
    except InvalidRegistryArg as exc:
        raise ToolError(str(exc), code="invalid_arguments", status_code=400) from exc

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

            sbom_blob = await _pull_sbom_blob(
                client=client, repo=repo, manifest_sha=manifest_sha,
                bundle_manifest=bundle_manifest,
            )
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

    if sbom_blob is None:
        return {
            "slug": slug,
            "version": version,
            "sbom_format": None,
            "spec_version": None,
            "components": [],
            "vulnerabilities": [],
            "summary": {
                "total_components": 0,
                "components_with_vulns": 0,
                "highest_severity": None,
            },
            "warnings": ["no SBOM artifact found for this bundle"],
        }

    try:
        parsed = parse_sbom(sbom_blob)
    except ValueError as exc:
        raise ToolError(
            f"SBOM parse failed: {exc}",
            code="invalid_arguments", status_code=502,
        ) from exc

    vulns = parsed.vulnerabilities
    if filter_sev:
        vulns = tuple(filter_by_severity(parsed.vulnerabilities, filter_sev))

    components = parsed.components[:max_components]
    truncated = len(parsed.components) > max_components

    by_sev = parsed.by_severity()
    highest = None
    for sev in ("CRITICAL", "HIGH", "MEDIUM", "LOW"):
        if by_sev.get(sev, 0) > 0:
            highest = sev
            break

    affected_components: set[str] = set()
    for v in parsed.vulnerabilities:
        for ref in v.affects:
            affected_components.add(ref)

    return {
        "slug": slug,
        "version": version,
        "sbom_format": "CycloneDX",
        "spec_version": parsed.spec_version,
        "components": [
            {
                "name": c.name,
                "version": c.version,
                "type": c.type,
                "purl": c.purl,
            }
            for c in components
        ],
        "components_truncated": truncated,
        "components_total": len(parsed.components),
        "vulnerabilities": [
            {
                "id": v.id,
                "severity": v.severity,
                "affects": list(v.affects),
            }
            for v in vulns
        ],
        "summary": {
            "total_components": len(parsed.components),
            "components_with_vulns": len(affected_components),
            "highest_severity": highest,
            "by_severity": by_sev,
        },
        "filter_severity": filter_sev,
    }


async def _pull_sbom_blob(
    *, client: OCIRegistryClient, repo: str, manifest_sha: str, bundle_manifest,
) -> bytes | None:
    # 1) Annotation-embedded sbom_digest takes precedence.
    if bundle_manifest.sbom_digest:
        try:
            return await client.get_blob(repo=repo, digest=bundle_manifest.sbom_digest)
        except OCIRegistryError:
            pass
    # 2) Referrer artifact (cyclonedx).
    try:
        referrers = await client.get_referrers(
            repo=repo,
            digest=f"sha256:{manifest_sha}",
            artifact_type="application/vnd.cyclonedx+json",
        )
    except OCIRegistryError:
        return None
    if not referrers:
        return None
    layers = referrers[0].get("layers") or []
    if not layers:
        return None
    digest = layers[0].get("digest")
    if not digest:
        return None
    try:
        return await client.get_blob(repo=repo, digest=digest)
    except OCIRegistryError:
        return None


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
    name="efe_get_sbom",
    description=(
        "Pull + parse a bundle's CycloneDX SBOM. Returns the full "
        "component list (capped by `max_components`) and vulnerabilities, "
        "optionally filtered by minimum severity. Read-only."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    profile_tier="agent-native",
)
