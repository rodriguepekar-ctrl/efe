"""efe_diff_bundles (M18 D47).

Semantic diff between two bundles : manifest fields + SBOM components +
SBOM vulnerabilities + size deltas. Identifies breaking changes
(dim, framework) vs safe upgrades (CVE resolved, minor SBOM change).
"""
from __future__ import annotations

from typing import Any

from .inspect_bundle import handler as _inspect_handler
from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeDiffBundlesInput",
    "description": "Semantic diff between two bundle versions.",
    "properties": {
        "slug_a": {
            "type": "string", "pattern": r"^[a-z][a-z0-9\-]{0,99}$",
            "minLength": 1, "maxLength": 100,
        },
        "version_a": {
            "type": "string", "pattern": r"^v\d+\.\d+\.\d+(-[a-z0-9\-]{1,15})?$",
            "minLength": 1, "maxLength": 50,
        },
        "slug_b": {
            "type": "string", "pattern": r"^[a-z][a-z0-9\-]{0,99}$",
            "minLength": 1, "maxLength": 100,
        },
        "version_b": {
            "type": "string", "pattern": r"^v\d+\.\d+\.\d+(-[a-z0-9\-]{1,15})?$",
            "minLength": 1, "maxLength": 50,
        },
        "registry": {"type": "string", "minLength": 1, "maxLength": 200},
    },
    "required": ["slug_a", "version_a", "slug_b", "version_b"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    slug_a = args["slug_a"]
    version_a = args["version_a"]
    slug_b = args["slug_b"]
    version_b = args["version_b"]

    inspect_args_common = {"include_full_sbom": True}
    if args.get("registry"):
        inspect_args_common["registry"] = args["registry"]

    info_a = await _inspect_handler(
        {"slug": slug_a, "version": version_a, **inspect_args_common}, ctx,
    )
    info_b = await _inspect_handler(
        {"slug": slug_b, "version": version_b, **inspect_args_common}, ctx,
    )

    # Manifest diff
    manifest_a = info_a.get("manifest", {})
    manifest_b = info_b.get("manifest", {})
    manifest_changes = {
        "model_name_changed": manifest_a.get("model_name") != manifest_b.get("model_name"),
        "dim_changed": manifest_a.get("dim") != manifest_b.get("dim"),
        "framework_changed": manifest_a.get("framework") != manifest_b.get("framework"),
        "produced_by_changed": manifest_a.get("produced_by") != manifest_b.get("produced_by"),
    }

    # SBOM diff
    sbom_a = info_a.get("sbom") or {"components": [], "vulnerabilities": []}
    sbom_b = info_b.get("sbom") or {"components": [], "vulnerabilities": []}
    sbom_diff = _sbom_diff(sbom_a, sbom_b)

    # Size diff — M18-D47-A2 fix : use REAL OCI layer sizes from each
    # bundle's manifest. Pre-fix used a hardcoded 10KB-per-component
    # estimate, returning fictitious deltas to the agent.
    size_a = int(manifest_a.get("total_layer_size") or 0)
    size_b = int(manifest_b.get("total_layer_size") or 0)
    # Fallback for backends/manifests that don't expose layer sizes :
    # downgrade to component-count estimate but flag the result.
    size_estimated = False
    if size_a == 0 and size_b == 0:
        size_a = sum(_estimate_component_size(c) for c in sbom_a.get("components", []))
        size_b = sum(_estimate_component_size(c) for c in sbom_b.get("components", []))
        size_estimated = True
    size_diff_bytes = size_b - size_a
    size_diff_percent = (
        (size_diff_bytes / size_a * 100.0) if size_a > 0 else 0.0
    )

    breaking = bool(manifest_changes["dim_changed"]) or bool(
        manifest_changes["framework_changed"]
    )
    recommendation = _recommendation(breaking, sbom_diff)

    return {
        "slug_a": slug_a,
        "version_a": version_a,
        "slug_b": slug_b,
        "version_b": version_b,
        "manifest_diff": manifest_changes,
        "sbom_diff": sbom_diff,
        "size_a_bytes": size_a,
        "size_b_bytes": size_b,
        "size_diff_bytes": size_diff_bytes,
        "size_diff_percent": round(size_diff_percent, 2),
        "size_estimated": size_estimated,
        "compatibility_breaking": breaking,
        "recommendation": recommendation,
    }


def _sbom_diff(sbom_a: dict[str, Any], sbom_b: dict[str, Any]) -> dict[str, Any]:
    comps_a = {(c.get("name", ""), c.get("version", "")): c
               for c in sbom_a.get("components", [])
               if isinstance(c, dict) and c.get("name")}
    comps_b = {(c.get("name", ""), c.get("version", "")): c
               for c in sbom_b.get("components", [])
               if isinstance(c, dict) and c.get("name")}
    names_a = {k[0] for k in comps_a.keys()}
    names_b = {k[0] for k in comps_b.keys()}

    added: list[dict[str, Any]] = []
    removed: list[dict[str, Any]] = []
    upgraded: list[dict[str, Any]] = []

    for key, comp in comps_b.items():
        name, version = key
        if name not in names_a:
            added.append(comp)

    for key, comp in comps_a.items():
        name, version = key
        if name not in names_b:
            removed.append(comp)

    # Upgraded = same name, different version
    for name in names_a & names_b:
        v_a = next((k[1] for k in comps_a if k[0] == name), None)
        v_b = next((k[1] for k in comps_b if k[0] == name), None)
        if v_a != v_b:
            upgraded.append({"name": name, "from": v_a, "to": v_b})

    # Vulnerability diff (by id)
    vulns_a = {v.get("id"): v for v in sbom_a.get("vulnerabilities", []) if isinstance(v, dict)}
    vulns_b = {v.get("id"): v for v in sbom_b.get("vulnerabilities", []) if isinstance(v, dict)}
    resolved = [
        {
            "cve": vid, "severity": vulns_a[vid].get("severity"),
            "in_component": (
                vulns_a[vid].get("affects", [None])[0]
                if vulns_a[vid].get("affects") else None
            ),
        }
        for vid in (set(vulns_a) - set(vulns_b))
        if vid
    ]
    introduced = [
        {
            "cve": vid, "severity": vulns_b[vid].get("severity"),
            "in_component": (
                vulns_b[vid].get("affects", [None])[0]
                if vulns_b[vid].get("affects") else None
            ),
        }
        for vid in (set(vulns_b) - set(vulns_a))
        if vid
    ]

    return {
        "components_added": added,
        "components_removed": removed,
        "components_upgraded": upgraded,
        "vulnerabilities_added": introduced,
        "vulnerabilities_resolved": resolved,
    }


def _estimate_component_size(component: dict[str, Any]) -> int:
    """Crude size estimate per component (~10 KB metadata + version dependent)."""
    return 10_000


def _recommendation(breaking: bool, sbom_diff: dict[str, Any]) -> str:
    if breaking:
        return (
            "BREAKING upgrade : dim or framework changed. "
            "Run efe_check_bundle_compatibility before deploy."
        )
    introduced = sbom_diff.get("vulnerabilities_added") or []
    high_intro = [v for v in introduced if v.get("severity") in ("HIGH", "CRITICAL")]
    if high_intro:
        return (
            f"Caution : {len(high_intro)} HIGH/CRITICAL CVEs introduced ; "
            "review the vulnerabilities_added list."
        )
    resolved = sbom_diff.get("vulnerabilities_resolved") or []
    if resolved:
        return (
            f"Safe upgrade : {len(resolved)} CVE(s) resolved, "
            "no breaking changes detected."
        )
    return "Safe upgrade : no breaking changes detected."


TOOL = Tool(
    name="efe_diff_bundles",
    description=(
        "Compute a semantic diff between two bundle versions : manifest "
        "fields, SBOM components added/removed/upgraded, vulnerabilities "
        "introduced/resolved. Read-only."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    profile_tier="agent-native",
)
