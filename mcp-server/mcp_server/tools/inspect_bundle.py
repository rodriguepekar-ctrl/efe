"""efe_inspect_bundle (M18 D47).

Pull all metadata for a single bundle :
  - OCI manifest + EFE annotations (slug/version/framework/dim/...)
  - Signature artifact metadata (verified status if trust store available)
  - SBOM summary (counts ; full SBOM via efe_get_sbom in D48)
  - SLSA provenance summary (level + builder URL)
  - Compatibility hints

Caches results in `ctx.bundle_cache` to dedupe repeated inspects in
the same session. Bundles are immutable in OCI so caching is safe.
"""
from __future__ import annotations

import os
from typing import Any

from ..bundle import (
    InvalidRegistryArg,
    OCIArtifactNotFound,
    OCIRegistryClient,
    OCIRegistryError,
    SLSAParseError,
    infer_slsa_level,
    is_builder_trusted,
    parse_manifest,
    parse_provenance,
    parse_registry_allowlist,
    parse_sbom,
    split_registry,
    validate_subject_digest,
    verify_ed25519_signature,
)
from ..bundle.registry_arg import default_oci_factory
from ..bundle.verify import _CRYPTO_AVAILABLE  # type: ignore[attr-defined]
from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeInspectBundleInput",
    "description": "Pull full metadata for one bundle (manifest + sig + SBOM summary).",
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
        "include_full_sbom": {"type": "boolean", "default": False},
        "registry": {"type": "string", "minLength": 1, "maxLength": 200},
    },
    "required": ["slug", "version"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    slug = args["slug"]
    version = args["version"]
    include_full_sbom = bool(args.get("include_full_sbom", False))
    registry = (
        args.get("registry")
        or os.environ.get("EFE_OCI_DEFAULT_REGISTRY")
    )

    # Cache lookup — M18-D49-A2 fix : use a tool-specific namespace
    # so the cached shape never collides with the resource handler's
    # cached shape under the same (slug, version).
    if ctx.bundle_cache is not None:
        cached = ctx.bundle_cache.get(
            tenant_id=ctx.tenant_id, slug=slug, version=version,
            namespace="inspect_tool",
        )
        if cached is not None and not include_full_sbom:
            # Cache stores the inspect result without full SBOM ; if user
            # asked for full SBOM, re-pull.
            return {**cached, "cache_hit": True}

    # Try the EFE 1.2 backend record first (server may have done its
    # own verification at register-time, saving us a round-trip).
    backend_record: dict[str, Any] | None = None
    try:
        backend_record = await ctx.backend.get_bundle_record(
            slug=slug, version=version, auth=ctx.auth,
        )
    except NotImplementedError:
        pass
    except Exception:  # noqa: BLE001 — fall through to OCI
        backend_record = None

    if not registry and backend_record:
        registry = backend_record.get("registry") or backend_record.get("registry_url")
    if not registry:
        raise ToolError(
            "no registry specified ; set EFE_OCI_DEFAULT_REGISTRY or pass `registry` arg",
            code="invalid_arguments",
            status_code=400,
        )

    # Pull from OCI (M18-D47-A1+A4 fix : split + scheme-strip via helper,
    # pass FULL URL to factory so allowlist enforces path scope).
    try:
        _host, repo, full_url = split_registry(registry)
    except InvalidRegistryArg as exc:
        raise ToolError(
            str(exc), code="invalid_arguments", status_code=400,
        ) from exc

    factory = ctx.oci_factory or default_oci_factory
    try:
        async with factory(full_url) as client:  # type: ignore[union-attr]
            manifest_bytes, manifest_json, manifest_sha = await client.get_manifest(
                repo=repo, reference=_make_reference(slug, version),
            )
            try:
                bundle_manifest = parse_manifest(
                    raw_bytes=manifest_bytes, manifest_sha256=manifest_sha,
                )
            except ValueError as exc:
                raise ToolError(
                    f"failed to parse OCI manifest: {exc}",
                    code="invalid_arguments",
                    status_code=502,
                ) from exc

            # Signature : look up referrers with sigType filter.
            sig_summary = await _lookup_signature(
                client=client, repo=repo, manifest_sha=manifest_sha,
                manifest_bytes=manifest_bytes,
            )

            # SBOM : look up referrers with cyclonedx artifact type, OR
            # the manifest itself can encode sbom_digest in annotations.
            sbom_summary, sbom_full = await _lookup_sbom(
                client=client, repo=repo, manifest_sha=manifest_sha,
                bundle_manifest=bundle_manifest,
                include_full_sbom=include_full_sbom,
            )
            # M18 D48 : SLSA provenance attestation lookup + parse.
            # M18-D49-A1 fix : pass the signature verification result
            # so SLSA level 3 is only claimed when crypto-verified.
            slsa_summary = await _lookup_slsa_provenance(
                client=client, repo=repo, manifest_sha=manifest_sha,
                signature_verified=bool(sig_summary.get("verified", False)),
            )
    except OCIArtifactNotFound as exc:
        raise ToolError(
            f"bundle {slug}:{version} not found in {registry}",
            code="not_found",
            status_code=404,
        ) from exc
    except OCIRegistryError as exc:
        raise ToolError(
            f"OCI registry error: {exc}",
            code=_oci_error_to_code(exc),
            status_code=getattr(exc, "status_code", None),
        ) from exc

    result: dict[str, Any] = {
        "slug": slug,
        "version": version,
        "registry": registry,
        "manifest": _manifest_to_dict(bundle_manifest),
        "signature": sig_summary,
        "sbom_summary": sbom_summary,
        "slsa_provenance": slsa_summary,
        "compatibility": {
            "min_efe_server_version": bundle_manifest.min_efe_server_version,
            "max_efe_server_version": bundle_manifest.max_efe_server_version,
            "required_features": list(bundle_manifest.required_features),
        },
        "cache_hit": False,
    }
    if include_full_sbom and sbom_full is not None:
        result["sbom"] = sbom_full

    # Cache (without full sbom — keep cache small) — namespace=inspect_tool.
    if ctx.bundle_cache is not None:
        without_full = {k: v for k, v in result.items() if k != "sbom"}
        ctx.bundle_cache.put(
            tenant_id=ctx.tenant_id, slug=slug, version=version,
            value=without_full, namespace="inspect_tool",
        )
    return result


def _make_reference(slug: str, version: str) -> str:
    """OCI tag reference : `slug-vX.Y.Z` (kebab-joined)."""
    return f"{slug}-{version}"


async def _lookup_signature(
    *,
    client: OCIRegistryClient,
    repo: str,
    manifest_sha: str,
    manifest_bytes: bytes,
) -> dict[str, Any]:
    """Look up + verify the bundle's signature artifact.

    Returns a summary dict. `verified` is `True` only if the public
    key was available AND the signature checked out. If the public
    key is missing, `verified=False, error="trust_store_unavailable"`.
    """
    summary: dict[str, Any] = {
        "is_signed": False,
        "signature_algorithm": None,
        "verified": False,
        "signed_by_key_id": None,
        "error": None,
    }
    try:
        referrers = await client.get_referrers(
            repo=repo,
            digest=f"sha256:{manifest_sha}",
            artifact_type="application/vnd.efe.bundle.signature.ed25519+json",
        )
    except OCIRegistryError as exc:
        summary["error"] = f"referrer lookup failed: {exc}"
        return summary

    if not referrers:
        return summary

    summary["is_signed"] = True
    summary["signature_algorithm"] = "ed25519"

    if not _CRYPTO_AVAILABLE:
        summary["error"] = (
            "cryptography extra not installed ; cannot verify signatures"
        )
        return summary

    public_key = _load_public_key_material()
    if public_key is None:
        summary["error"] = "trust_store_unavailable"
        return summary

    # Pull the signature blob
    sig_manifest = referrers[0]
    sig_layers = sig_manifest.get("layers") or []
    if not sig_layers:
        summary["error"] = "signature manifest has no layers"
        return summary
    sig_digest = sig_layers[0].get("digest")
    if not sig_digest:
        summary["error"] = "signature layer missing digest"
        return summary
    try:
        signature_bytes = await client.get_blob(repo=repo, digest=sig_digest)
    except OCIRegistryError as exc:
        summary["error"] = f"failed to pull signature blob: {exc}"
        return summary

    result = verify_ed25519_signature(
        manifest_bytes=manifest_bytes,
        signature_bytes=signature_bytes,
        public_key_pem=public_key,
        expected_manifest_sha256=manifest_sha,
    )
    summary["verified"] = result.verified
    summary["signed_by_key_id"] = result.signed_by_key_id
    if result.error:
        summary["error"] = result.error
    return summary


async def _lookup_sbom(
    *,
    client: OCIRegistryClient,
    repo: str,
    manifest_sha: str,
    bundle_manifest,
    include_full_sbom: bool,
) -> tuple[dict[str, Any], dict[str, Any] | None]:
    """Pull + parse the SBOM artifact ; return (summary, full_or_None)."""
    summary: dict[str, Any] = {
        "format": None,
        "spec_version": None,
        "components_count": 0,
        "vulnerabilities_count": 0,
        "dependencies_summary": {},
        "by_severity": {},
        "error": None,
    }

    sbom_blob: bytes | None = None

    # 1) Annotation-embedded sbom_digest
    if bundle_manifest.sbom_digest:
        try:
            sbom_blob = await client.get_blob(
                repo=repo, digest=bundle_manifest.sbom_digest,
            )
        except OCIRegistryError as exc:
            summary["error"] = f"sbom blob fetch failed: {exc}"

    # 2) Referrer artifact
    if sbom_blob is None:
        try:
            referrers = await client.get_referrers(
                repo=repo,
                digest=f"sha256:{manifest_sha}",
                artifact_type="application/vnd.cyclonedx+json",
            )
            if referrers:
                layers = referrers[0].get("layers") or []
                if layers:
                    digest = layers[0].get("digest")
                    if digest:
                        sbom_blob = await client.get_blob(repo=repo, digest=digest)
        except OCIRegistryError as exc:
            summary["error"] = summary["error"] or f"sbom referrer fetch failed: {exc}"

    if sbom_blob is None:
        return summary, None

    try:
        parsed = parse_sbom(sbom_blob)
    except ValueError as exc:
        summary["error"] = f"sbom parse failed: {exc}"
        return summary, None

    summary["format"] = "CycloneDX"
    summary["spec_version"] = parsed.spec_version
    summary["components_count"] = parsed.components_count
    summary["vulnerabilities_count"] = parsed.vulnerabilities_count
    summary["dependencies_summary"] = parsed.dependencies_summary()
    summary["by_severity"] = parsed.by_severity()

    full = None
    if include_full_sbom:
        full = {
            "components": [
                {
                    "name": c.name,
                    "version": c.version,
                    "type": c.type,
                    "purl": c.purl,
                }
                for c in parsed.components
            ],
            "vulnerabilities": [
                {
                    "id": v.id,
                    "severity": v.severity,
                    "affects": list(v.affects),
                }
                for v in parsed.vulnerabilities
            ],
        }
    return summary, full


def _manifest_to_dict(m) -> dict[str, Any]:
    return {
        "manifestSha256": m.manifest_sha256,
        "framework": m.framework,
        "model_name": m.model_name,
        "dim": m.dim,
        "is_dev_unsigned": m.is_dev_unsigned,
        "schema_version": m.bundle_schema_version,
        "produced_at": m.produced_at,
        "produced_by": m.produced_by,
        # M18-D47-A2 fix : real bundle size from OCI manifest.
        "total_layer_size": m.total_layer_size,
        "layer_count": m.layer_count,
    }


async def _lookup_slsa_provenance(
    *,
    client: OCIRegistryClient,
    repo: str,
    manifest_sha: str,
    signature_verified: bool = False,
) -> dict[str, Any]:
    """Pull + parse the SLSA provenance attestation referrer (M18 D48).

    Looks for an OCI referrer with `artifactType =
    application/vnd.in-toto+json`, which is the canonical Sigstore-style
    attestation envelope. Falls back to a placeholder summary when no
    attestation is found or parsing fails.
    """
    summary: dict[str, Any] = {
        "present": False,
        "level": 0,
        "builder": None,
        "build_type": None,
        "subject_digest_match": None,
        "builder_trusted": None,
        "invocation_id": None,
        "started_on": None,
        "finished_on": None,
        "warnings": [],
        "error": None,
    }

    try:
        referrers = await client.get_referrers(
            repo=repo,
            digest=f"sha256:{manifest_sha}",
            artifact_type="application/vnd.in-toto+json",
        )
    except OCIRegistryError as exc:
        summary["error"] = f"referrer lookup failed: {exc}"
        return summary

    if not referrers:
        return summary

    layers = referrers[0].get("layers") or []
    if not layers:
        summary["error"] = "attestation manifest has no layers"
        return summary
    digest = layers[0].get("digest")
    if not digest:
        summary["error"] = "attestation layer missing digest"
        return summary

    try:
        attestation_blob = await client.get_blob(repo=repo, digest=digest)
    except OCIRegistryError as exc:
        summary["error"] = f"attestation blob fetch failed: {exc}"
        return summary

    try:
        provenance = parse_provenance(attestation_blob)
    except SLSAParseError as exc:
        summary["error"] = f"attestation parse failed: {exc}"
        return summary

    summary["present"] = True
    summary["builder"] = provenance.builder_id
    summary["build_type"] = provenance.build_type
    summary["invocation_id"] = provenance.invocation_id
    summary["started_on"] = provenance.started_on
    summary["finished_on"] = provenance.finished_on

    # TM-M18-E-03 mitigation : cross-check subject digest
    ok, reason = validate_subject_digest(
        provenance=provenance, expected_sha256=manifest_sha,
    )
    summary["subject_digest_match"] = ok
    if not ok:
        summary["warnings"].append(reason or "subject digest does not match manifest")

    summary["builder_trusted"] = is_builder_trusted(provenance=provenance)
    if not summary["builder_trusted"]:
        summary["warnings"].append(
            f"builder {provenance.builder_id!r} is not in the trusted-builder allowlist "
            "(set EFE_SLSA_BUILDER_ALLOWLIST to extend)"
        )

    summary["level"] = infer_slsa_level(
        provenance, signature_verified=signature_verified,
    )
    summary["signature_verified_for_level"] = signature_verified
    return summary


def _load_public_key_material() -> bytes | None:
    """Read trust store from env. D48 will extend with backend trust-store."""
    pem = os.environ.get("EFE_OCI_PUBLIC_KEY")
    if pem:
        return pem.encode("utf-8")
    path = os.environ.get("EFE_OCI_PUBLIC_KEY_FILE")
    if path:
        try:
            with open(path, "rb") as fh:
                return fh.read()
        except OSError:
            return None
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
    name="efe_inspect_bundle",
    description=(
        "Pull full metadata for one EFE bundle : OCI manifest, signature "
        "status, SBOM summary, SLSA provenance, compatibility hints. "
        "Caches results per session ; bundles are immutable. Read-only."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    profile_tier="agent-native",
)
