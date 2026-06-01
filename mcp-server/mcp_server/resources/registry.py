"""Resource URI parsing + dispatch (M17 D44).

URI grammar (per M17 architecture §7.2 + Annex B) :

  efe-uri    = "efe://" path [ "?" query ]
  path       = catalogs / audit / quotas / samples
  catalogs   = "catalogs/" name [ "/versions/" version
                                / "/classes/" class ]
  audit      = "audit/recent"
  quotas     = "quotas"
  samples    = "samples/" sample-id

The parser is strict : anything outside this grammar → 400.
TM-M17-E-01 mitigation : no traversal, no special chars.
"""
from __future__ import annotations

import json
import re
from dataclasses import dataclass
from typing import Any
from urllib.parse import parse_qs, urlparse

from ..auth import AuthContext
from ..backend_client import BackendClient


# ─────────────────────────────────────────────────────────────────────
# URI grammar
# ─────────────────────────────────────────────────────────────────────

_NAME_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_\-]{0,99}$")
_CLASS_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.\-]{0,99}$")
_VERSION_RE = re.compile(r"^[1-9]\d{0,4}$")
_SAMPLE_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_\-]{0,49}$")
# M18 D48 — bundle URI grammar.
# Audit fix M18-D49-A4 (LOW) : version regex matches the tool schemas
# (up to 5 digits per part). Previously `^v\d{1,3}` rejected v1234.0.0
# even though the tool schemas accepted it ; URI grammar must NOT be
# stricter than the equivalent tool input.
_BUNDLE_SLUG_RE = re.compile(r"^[a-z][a-z0-9\-]{0,99}$")
_BUNDLE_VERSION_RE = re.compile(
    r"^v\d{1,5}\.\d{1,5}\.\d{1,5}(-[a-z0-9\-]{1,15})?$"
)
_BUNDLE_SUBRESOURCES = frozenset({"sbom", "signature"})
# `registry` query key added in D48 to let resource readers override the
# default registry env-var on a per-call basis.
_ALLOWED_QUERY_KEYS = frozenset({"limit", "since", "format", "cursor", "registry"})


class InvalidResourceUri(ValueError):
    """Raised when a URI does not match the formal grammar."""


@dataclass(frozen=True)
class ParsedUri:
    kind: str
    # M16/M17 fields
    name: str | None = None
    version: int | None = None
    class_name: str | None = None
    sample_id: str | None = None
    # M18 D48 bundle fields
    bundle_slug: str | None = None
    bundle_version: str | None = None
    bundle_subresource: str | None = None
    query: dict[str, str] | None = None


def parse_resource_uri(uri: str) -> ParsedUri:
    if not isinstance(uri, str) or not uri.startswith("efe://"):
        raise InvalidResourceUri(f"expected URI starting with 'efe://', got {uri!r}")
    parsed = urlparse(uri)
    if parsed.scheme != "efe":
        raise InvalidResourceUri(f"scheme must be 'efe', got {parsed.scheme!r}")

    # urlparse on `efe://path` puts everything after `efe://` into `netloc`+`path`.
    # We canonicalise by joining and stripping leading slash.
    raw_path = (parsed.netloc + parsed.path).lstrip("/")
    if not raw_path:
        raise InvalidResourceUri("empty path")
    if ".." in raw_path.split("/"):
        raise InvalidResourceUri("path traversal not allowed")

    # Validate query
    query_dict: dict[str, str] = {}
    if parsed.query:
        qs = parse_qs(parsed.query, keep_blank_values=False, strict_parsing=False)
        for k, vals in qs.items():
            if k not in _ALLOWED_QUERY_KEYS:
                raise InvalidResourceUri(f"unknown query key {k!r}")
            if len(vals) != 1:
                raise InvalidResourceUri(f"duplicate query key {k!r}")
            query_dict[k] = vals[0]

    parts = raw_path.split("/")
    if parts[0] == "quotas" and len(parts) == 1:
        return ParsedUri(kind="quotas", query=query_dict)

    if parts[0] == "audit" and len(parts) == 2 and parts[1] == "recent":
        return ParsedUri(kind="audit_recent", query=query_dict)

    if parts[0] == "samples" and len(parts) == 2:
        if not _SAMPLE_RE.match(parts[1]):
            raise InvalidResourceUri(f"invalid sample-id: {parts[1]!r}")
        return ParsedUri(kind="sample", sample_id=parts[1], query=query_dict)

    # M18 D49 — `efe://deployments/current`
    if parts[0] == "deployments":
        if len(parts) == 2 and parts[1] == "current":
            return ParsedUri(kind="deployment_current", query=query_dict)
        raise InvalidResourceUri(
            f"unknown deployments sub-path: {raw_path!r} "
            "(only 'current' is supported in M18 D49)"
        )

    # M18 D48 — `efe://bundles[/{slug}/{version}[/{sub-resource}]]`
    if parts[0] == "bundles":
        if len(parts) == 1:
            return ParsedUri(kind="bundles_list", query=query_dict)
        if len(parts) < 3:
            raise InvalidResourceUri(
                "bundles URI must be `bundles/{slug}/{version}` or "
                "`bundles/{slug}/{version}/{sbom|signature}`"
            )
        slug, ver = parts[1], parts[2]
        if not _BUNDLE_SLUG_RE.match(slug):
            raise InvalidResourceUri(f"invalid bundle slug: {slug!r}")
        if not _BUNDLE_VERSION_RE.match(ver):
            raise InvalidResourceUri(f"invalid bundle version: {ver!r}")
        if len(parts) == 3:
            return ParsedUri(
                kind="bundle",
                bundle_slug=slug, bundle_version=ver,
                query=query_dict,
            )
        if len(parts) == 4:
            sub = parts[3]
            if sub not in _BUNDLE_SUBRESOURCES:
                raise InvalidResourceUri(
                    f"invalid bundle subresource: {sub!r} "
                    f"(allowed: {sorted(_BUNDLE_SUBRESOURCES)})"
                )
            return ParsedUri(
                kind=f"bundle_{sub}",
                bundle_slug=slug, bundle_version=ver,
                bundle_subresource=sub, query=query_dict,
            )
        raise InvalidResourceUri(f"unknown bundle sub-path: {raw_path!r}")

    if parts[0] == "catalogs":
        if len(parts) < 2:
            raise InvalidResourceUri("catalog URI requires a name")
        name = parts[1]
        if not _NAME_RE.match(name):
            raise InvalidResourceUri(f"invalid catalog name: {name!r}")
        if len(parts) == 2:
            return ParsedUri(kind="catalog_summary", name=name, query=query_dict)
        if len(parts) == 4 and parts[2] == "versions":
            v_str = parts[3]
            if not _VERSION_RE.match(v_str):
                raise InvalidResourceUri(f"invalid version: {v_str!r}")
            return ParsedUri(
                kind="catalog_version", name=name, version=int(v_str),
                query=query_dict,
            )
        if len(parts) == 4 and parts[2] == "classes":
            cls = parts[3]
            if not _CLASS_RE.match(cls):
                raise InvalidResourceUri(f"invalid class name: {cls!r}")
            return ParsedUri(
                kind="catalog_class", name=name, class_name=cls,
                query=query_dict,
            )
        raise InvalidResourceUri(f"unknown catalog sub-path: {raw_path!r}")

    raise InvalidResourceUri(f"unknown resource path: {raw_path!r}")


# ─────────────────────────────────────────────────────────────────────
# Resource descriptors (advertised via resources/list)
# ─────────────────────────────────────────────────────────────────────


@dataclass(frozen=True)
class ResourceDescriptor:
    uri_pattern: str
    name: str
    description: str
    mime_type: str = "application/json"

    def to_mcp_dict(self) -> dict[str, Any]:
        return {
            "uri": self.uri_pattern,
            "name": self.name,
            "description": self.description,
            "mimeType": self.mime_type,
        }


RESOURCE_DESCRIPTORS: tuple[ResourceDescriptor, ...] = (
    ResourceDescriptor(
        uri_pattern="efe://catalogs/{name}",
        name="EFE catalog summary",
        description=(
            "Catalog summary : latest version, fingerprint, class count, "
            "embedder info."
        ),
    ),
    ResourceDescriptor(
        uri_pattern="efe://catalogs/{name}/versions/{version}",
        name="EFE catalog version",
        description="A specific (immutable) version of a catalog.",
    ),
    ResourceDescriptor(
        uri_pattern="efe://catalogs/{name}/classes/{class}",
        name="EFE catalog class",
        description="A single class of a catalog : examples + threshold.",
    ),
    ResourceDescriptor(
        uri_pattern="efe://audit/recent",
        name="Recent audit events",
        description=(
            "Recent audit log entries (D14-bis HMAC-chained). "
            "Pagination via ?limit=N (max 200) and ?since=ISO8601."
        ),
    ),
    ResourceDescriptor(
        uri_pattern="efe://quotas",
        name="EFE quota usage",
        description="Current D12 quota usage and limits for the caller's tenant.",
    ),
    ResourceDescriptor(
        uri_pattern="efe://samples/{id}",
        name="EFE sample catalog",
        description="A sample catalog from the playground (e.g. support_routing).",
    ),
    # M18 D48 — bundle resources
    ResourceDescriptor(
        uri_pattern="efe://bundles",
        name="EFE bundles catalog",
        description=(
            "List of bundles known to the backend (or available in the "
            "configured OCI registry)."
        ),
    ),
    ResourceDescriptor(
        uri_pattern="efe://bundles/{slug}/{version}",
        name="EFE bundle metadata",
        description=(
            "Full metadata for one bundle : OCI manifest, signature "
            "presence, SBOM summary, SLSA provenance summary."
        ),
    ),
    ResourceDescriptor(
        uri_pattern="efe://bundles/{slug}/{version}/sbom",
        name="EFE bundle SBOM",
        description=(
            "Parsed CycloneDX 1.5 SBOM for a bundle : full components + "
            "vulnerabilities lists."
        ),
    ),
    ResourceDescriptor(
        uri_pattern="efe://bundles/{slug}/{version}/signature",
        name="EFE bundle signature",
        description=(
            "Bundle Ed25519 signature artifact + SLSA provenance summary "
            "(verified status if trust store available)."
        ),
    ),
    # M18 D49 — deployment resource
    ResourceDescriptor(
        uri_pattern="efe://deployments/current",
        name="EFE current deployment",
        description=(
            "Active deployment metadata : bundle slug+version, helm "
            "release info, replicas, health, uptime."
        ),
    ),
)


# ─────────────────────────────────────────────────────────────────────
# Dispatch (resources/read)
# ─────────────────────────────────────────────────────────────────────


async def read_resource(
    *,
    uri: str,
    auth: AuthContext,
    backend: BackendClient,
    bundle_cache=None,                   # M18 D48 — optional BundleCache
    oci_factory=None,                    # M18 D48 — optional OCI factory
) -> dict[str, Any]:
    """Resolve a resource URI to its content (MCP `resources/read` payload).

    M18 D48 : `bundle_cache` + `oci_factory` are passed through to the
    bundle resource handlers when a `efe://bundles/...` URI is seen.
    They are optional ; when absent the handlers use sensible defaults
    (no cache + the standard `default_oci_factory`).
    """
    parsed = parse_resource_uri(uri)

    # M18 D49 — deployment resource
    if parsed.kind == "deployment_current":
        from . import deployments as _deployments
        try:
            content = await _deployments.get_current_deployment(
                auth=auth, backend=backend,
            )
        except _deployments.DeploymentResourceError as exc:
            raise InvalidResourceUri(str(exc)) from exc
        return _wrap(uri, content)

    # M18 D48 — bundle resources delegated to mcp_server.resources.bundles.
    if parsed.kind in ("bundles_list", "bundle", "bundle_sbom", "bundle_signature"):
        from . import bundles as _bundles

        registry_arg = (parsed.query or {}).get("registry")
        common = {
            "auth": auth, "backend": backend,
            "bundle_cache": bundle_cache, "oci_factory": oci_factory,
            "registry_arg": registry_arg,
        }
        try:
            if parsed.kind == "bundles_list":
                content = await _bundles.list_bundles(**common)
            elif parsed.kind == "bundle":
                content = await _bundles.get_bundle(
                    slug=parsed.bundle_slug, version=parsed.bundle_version, **common,
                )
            elif parsed.kind == "bundle_sbom":
                content = await _bundles.get_bundle_sbom(
                    slug=parsed.bundle_slug, version=parsed.bundle_version, **common,
                )
            elif parsed.kind == "bundle_signature":
                content = await _bundles.get_bundle_signature(
                    slug=parsed.bundle_slug, version=parsed.bundle_version, **common,
                )
        except _bundles.BundleResourceError as exc:
            raise InvalidResourceUri(str(exc)) from exc
        return _wrap(uri, content)

    if parsed.kind == "catalog_summary":
        cat_list = await backend.list_catalogs(auth=auth)
        match = next((c for c in cat_list if c.get("name") == parsed.name), None)
        if match is None:
            raise InvalidResourceUri(f"catalog {parsed.name!r} not found for tenant")
        return _wrap(uri, match)

    if parsed.kind == "catalog_version":
        try:
            classes = await backend.list_catalog_classes(catalog=parsed.name, auth=auth)
        except Exception as exc:  # noqa: BLE001
            raise InvalidResourceUri(str(exc))
        return _wrap(
            uri,
            {
                "name": parsed.name,
                "version": parsed.version,
                "classes": classes,
            },
        )

    if parsed.kind == "catalog_class":
        cls = await backend.get_class_examples(
            catalog=parsed.name, class_name=parsed.class_name, auth=auth,
        )
        return _wrap(uri, cls)

    if parsed.kind == "audit_recent":
        limit = int((parsed.query or {}).get("limit", 50))
        limit = max(1, min(200, limit))
        events = await backend.get_audit_log(limit=limit, auth=auth)
        return _wrap(uri, {"count": len(events), "events": events})

    if parsed.kind == "quotas":
        quotas = await backend.get_admin_quotas(auth=auth)
        return _wrap(uri, quotas)

    if parsed.kind == "sample":
        # The backend's playground exposes /samples/<id> ; production
        # backends likely don't. We try and fall back to a 404 message.
        try:
            data = await backend._request(  # type: ignore[attr-defined]
                "GET", f"/samples/{parsed.sample_id}", auth=auth,
            )
        except Exception as exc:  # noqa: BLE001
            raise InvalidResourceUri(str(exc))
        return _wrap(uri, data)

    raise InvalidResourceUri(f"unhandled resource kind: {parsed.kind}")


def _wrap(uri: str, content: Any) -> dict[str, Any]:
    text = json.dumps(content, ensure_ascii=False, indent=2, sort_keys=True)
    return {
        "uri": uri,
        "mimeType": "application/json",
        "text": text,
    }
