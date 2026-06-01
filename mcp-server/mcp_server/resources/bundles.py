"""MCP resource handlers for `efe://bundles/...` (M18 D48).

Four URI types are exposed when profile is `agent-native` or above :

  - efe://bundles                           liste de tous les bundles connus
  - efe://bundles/{slug}/{version}          full inspect (manifest+sig+sbom+slsa)
  - efe://bundles/{slug}/{version}/sbom     parsed CycloneDX SBOM (+ summary)
  - efe://bundles/{slug}/{version}/signature signature artifact + SLSA summary

These reuse the same OCI registry client + bundle cache infrastructure
as the M18 tools — read-only, no subprocess, allowlist enforced.
"""
from __future__ import annotations

import os
from typing import Any

from ..auth import AuthContext
from ..backend_client import BackendClient
from ..bundle import (
    InvalidRegistryArg,
    OCIArtifactNotFound,
    OCIRegistryError,
    parse_manifest,
    parse_provenance,
    parse_sbom,
    split_registry,
    validate_subject_digest,
    verify_ed25519_signature,
)
from ..bundle.registry_arg import default_oci_factory
from ..bundle.slsa import infer_slsa_level, is_builder_trusted, SLSAParseError
from ..bundle.verify import _CRYPTO_AVAILABLE  # type: ignore[attr-defined]


class BundleResourceError(Exception):
    """Raised when a bundle resource cannot be served (mapped to MCP error)."""

    def __init__(self, message: str, *, code: str = "not_found") -> None:
        super().__init__(message)
        self.code = code


# ─────────────────────────────────────────────────────────────────────
# Resolution helpers
# ─────────────────────────────────────────────────────────────────────


def _resolve_registry(*, registry_arg: str | None) -> str:
    """Pick the registry URL to use, falling back to env."""
    return (
        registry_arg
        or os.environ.get("EFE_OCI_DEFAULT_REGISTRY")
        or ""
    )


def _make_factory(oci_factory):
    return oci_factory or default_oci_factory


def _public_key_pem() -> bytes | None:
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


# ─────────────────────────────────────────────────────────────────────
# 1. efe://bundles  — list known bundles
# ─────────────────────────────────────────────────────────────────────


async def list_bundles(
    *,
    auth: AuthContext,
    backend: BackendClient,
    bundle_cache=None,                # noqa: ARG001 — kept for symmetry
    oci_factory=None,
    registry_arg: str | None = None,
) -> dict[str, Any]:
    """Return the list of bundles known to the backend (or fall back to OCI tags)."""
    # Backend-mediated path first.
    try:
        result = await backend.list_known_bundles(
            registry_filter=registry_arg,
            slug_filter=None,
            limit=100,
            auth=auth,
        )
        return {
            "source": "backend",
            "bundles": result.get("bundles", []),
            "count": len(result.get("bundles", [])),
        }
    except NotImplementedError:
        pass

    # Direct OCI fallback : we need a registry URL.
    registry = _resolve_registry(registry_arg=registry_arg)
    if not registry:
        raise BundleResourceError(
            "no registry available : pass `?registry=` query or set "
            "EFE_OCI_DEFAULT_REGISTRY",
            code="invalid_resource_uri",
        )
    try:
        _host, repo, full_url = split_registry(registry)
    except InvalidRegistryArg as exc:
        raise BundleResourceError(str(exc), code="invalid_resource_uri") from exc

    factory = _make_factory(oci_factory)
    try:
        async with factory(full_url) as client:
            tags = await client.list_tags(repo=repo, n=100)
    except OCIRegistryError as exc:
        raise BundleResourceError(
            f"OCI registry list failed: {exc}",
            code="backend_unavailable",
        ) from exc

    return {
        "source": "oci_direct",
        "registry": registry,
        "bundles": [{"slug_version_tag": t} for t in tags],
        "count": len(tags),
    }


# ─────────────────────────────────────────────────────────────────────
# 2. efe://bundles/{slug}/{version} — full bundle metadata
# ─────────────────────────────────────────────────────────────────────


async def get_bundle(
    *,
    auth: AuthContext,
    backend: BackendClient,
    slug: str,
    version: str,
    bundle_cache=None,
    oci_factory=None,
    registry_arg: str | None = None,
) -> dict[str, Any]:
    """Return full bundle metadata : manifest + signature status + SBOM
    summary + SLSA provenance summary."""
    # Cache lookup — M18-D49-A2 fix : distinct namespace from the
    # inspect_bundle tool to avoid shape collision.
    if bundle_cache is not None:
        cached = bundle_cache.get(
            tenant_id=getattr(auth, "tenant_id", None) or "",
            slug=slug, version=version,
            namespace="resource_get_bundle",
        )
        if cached is not None:
            return {**cached, "cache_hit": True}

    registry = _resolve_registry(registry_arg=registry_arg)
    if not registry:
        # Try backend bundle record as fallback
        try:
            rec = await backend.get_bundle_record(
                slug=slug, version=version, auth=auth,
            )
            registry = rec.get("registry") or rec.get("registry_url") or ""
        except NotImplementedError:
            pass
    if not registry:
        raise BundleResourceError(
            "no registry available : pass `?registry=` query or set "
            "EFE_OCI_DEFAULT_REGISTRY",
            code="invalid_resource_uri",
        )

    try:
        _host, repo, full_url = split_registry(registry)
    except InvalidRegistryArg as exc:
        raise BundleResourceError(str(exc), code="invalid_resource_uri") from exc

    factory = _make_factory(oci_factory)
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
                raise BundleResourceError(
                    f"failed to parse OCI manifest: {exc}",
                    code="invalid_resource_uri",
                ) from exc

            # Signature presence check (cheap)
            sig_present = False
            try:
                sig_referrers = await client.get_referrers(
                    repo=repo,
                    digest=f"sha256:{manifest_sha}",
                    artifact_type="application/vnd.efe.bundle.signature.ed25519+json",
                )
                sig_present = bool(sig_referrers)
            except OCIRegistryError:
                pass

            # SBOM summary
            sbom_summary = await _sbom_summary(
                client=client, repo=repo, manifest_sha=manifest_sha,
                bundle_manifest=bundle_manifest,
            )
            # M18-D49-A1 fix : verify the signature before computing
            # SLSA level so level 3 is gated on actual cryptographic
            # verification.
            sig_verified = await _verify_bundle_signature_inline(
                client=client, repo=repo,
                manifest_bytes=manifest_bytes, manifest_sha=manifest_sha,
            )
            # SLSA summary (level 3 requires sig verified)
            slsa_summary = await _slsa_summary(
                client=client, repo=repo, manifest_sha=manifest_sha,
                signature_verified=sig_verified,
            )
    except OCIArtifactNotFound as exc:
        raise BundleResourceError(
            f"bundle {slug}:{version} not found in {registry}",
            code="not_found",
        ) from exc
    except OCIRegistryError as exc:
        raise BundleResourceError(
            f"OCI registry error: {exc}", code="backend_unavailable",
        ) from exc

    result = {
        "slug": slug,
        "version": version,
        "registry": registry,
        "manifest": {
            "manifestSha256": bundle_manifest.manifest_sha256,
            "framework": bundle_manifest.framework,
            "model_name": bundle_manifest.model_name,
            "dim": bundle_manifest.dim,
            "is_dev_unsigned": bundle_manifest.is_dev_unsigned,
            "schema_version": bundle_manifest.bundle_schema_version,
            "produced_at": bundle_manifest.produced_at,
            "produced_by": bundle_manifest.produced_by,
            "total_layer_size": bundle_manifest.total_layer_size,
            "layer_count": bundle_manifest.layer_count,
        },
        "signature": {
            "is_signed": sig_present,
            "verified": None,
            "note": (
                "Use efe_verify_bundle_signature for cryptographic verification."
            ),
        },
        "sbom_summary": sbom_summary,
        "slsa_provenance": slsa_summary,
        "cache_hit": False,
    }
    if bundle_cache is not None:
        bundle_cache.put(
            tenant_id=getattr(auth, "tenant_id", None) or "",
            slug=slug, version=version, value=result,
            namespace="resource_get_bundle",
        )
    return result


# ─────────────────────────────────────────────────────────────────────
# 3. efe://bundles/{slug}/{version}/sbom
# ─────────────────────────────────────────────────────────────────────


async def get_bundle_sbom(
    *,
    auth: AuthContext,
    backend: BackendClient,
    slug: str,
    version: str,
    bundle_cache=None,                # noqa: ARG001 — sbom blob not cached
    oci_factory=None,
    registry_arg: str | None = None,
) -> dict[str, Any]:
    registry = _resolve_registry(registry_arg=registry_arg)
    if not registry:
        raise BundleResourceError(
            "no registry available", code="invalid_resource_uri",
        )
    try:
        _host, repo, full_url = split_registry(registry)
    except InvalidRegistryArg as exc:
        raise BundleResourceError(str(exc), code="invalid_resource_uri") from exc

    factory = _make_factory(oci_factory)
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
                raise BundleResourceError(
                    f"manifest parse failed: {exc}", code="invalid_resource_uri",
                ) from exc

            sbom_blob = await _pull_sbom_blob(
                client=client, repo=repo, manifest_sha=manifest_sha,
                bundle_manifest=bundle_manifest,
            )
    except OCIArtifactNotFound as exc:
        raise BundleResourceError(
            f"bundle {slug}:{version} not found", code="not_found",
        ) from exc
    except OCIRegistryError as exc:
        raise BundleResourceError(
            f"OCI registry error: {exc}", code="backend_unavailable",
        ) from exc

    if sbom_blob is None:
        return {
            "slug": slug,
            "version": version,
            "sbom_present": False,
            "sbom_format": None,
            "components": [],
            "vulnerabilities": [],
        }
    try:
        parsed = parse_sbom(sbom_blob)
    except ValueError as exc:
        raise BundleResourceError(
            f"sbom parse failed: {exc}", code="invalid_resource_uri",
        ) from exc

    return {
        "slug": slug,
        "version": version,
        "sbom_present": True,
        "sbom_format": "CycloneDX",
        "spec_version": parsed.spec_version,
        "components": [
            {"name": c.name, "version": c.version, "type": c.type, "purl": c.purl}
            for c in parsed.components
        ],
        "vulnerabilities": [
            {"id": v.id, "severity": v.severity, "affects": list(v.affects)}
            for v in parsed.vulnerabilities
        ],
        "summary": {
            "components_count": parsed.components_count,
            "vulnerabilities_count": parsed.vulnerabilities_count,
            "by_severity": parsed.by_severity(),
        },
    }


# ─────────────────────────────────────────────────────────────────────
# 4. efe://bundles/{slug}/{version}/signature
# ─────────────────────────────────────────────────────────────────────


async def get_bundle_signature(
    *,
    auth: AuthContext,
    backend: BackendClient,
    slug: str,
    version: str,
    bundle_cache=None,                # noqa: ARG001
    oci_factory=None,
    registry_arg: str | None = None,
) -> dict[str, Any]:
    registry = _resolve_registry(registry_arg=registry_arg)
    if not registry:
        raise BundleResourceError(
            "no registry available", code="invalid_resource_uri",
        )
    try:
        _host, repo, full_url = split_registry(registry)
    except InvalidRegistryArg as exc:
        raise BundleResourceError(str(exc), code="invalid_resource_uri") from exc

    factory = _make_factory(oci_factory)
    try:
        async with factory(full_url) as client:
            manifest_bytes, _, manifest_sha = await client.get_manifest(
                repo=repo, reference=f"{slug}-{version}",
            )

            sig_referrers = await client.get_referrers(
                repo=repo,
                digest=f"sha256:{manifest_sha}",
                artifact_type="application/vnd.efe.bundle.signature.ed25519+json",
            )
            sig_blob: bytes | None = None
            if sig_referrers:
                layers = sig_referrers[0].get("layers") or []
                if layers and layers[0].get("digest"):
                    sig_blob = await client.get_blob(
                        repo=repo, digest=layers[0]["digest"],
                    )

            # Verify the signature inline so we can pass the result into
            # the SLSA level inference (M18-D49-A1 fix).
            verified = False
            signed_by_key_id: str | None = None
            error: str | None = None
            if sig_blob is None:
                error = "no signature artifact present"
            elif not _CRYPTO_AVAILABLE:
                error = "cryptography extra not installed"
            else:
                public_key = _public_key_pem()
                if public_key is None:
                    error = "trust_store_unavailable"
                else:
                    result = verify_ed25519_signature(
                        manifest_bytes=manifest_bytes,
                        signature_bytes=sig_blob,
                        public_key_pem=public_key,
                        expected_manifest_sha256=manifest_sha,
                    )
                    verified = result.verified
                    signed_by_key_id = result.signed_by_key_id
                    error = result.error

            slsa_summary = await _slsa_summary(
                client=client, repo=repo, manifest_sha=manifest_sha,
                signature_verified=verified,
            )
    except OCIArtifactNotFound as exc:
        raise BundleResourceError(
            f"bundle {slug}:{version} not found", code="not_found",
        ) from exc
    except OCIRegistryError as exc:
        raise BundleResourceError(
            f"OCI registry error: {exc}", code="backend_unavailable",
        ) from exc

    return {
        "slug": slug,
        "version": version,
        "is_signed": sig_blob is not None,
        "signature_algorithm": "ed25519" if sig_blob is not None else None,
        "verified": verified,
        "signed_by_key_id": signed_by_key_id,
        "manifest_sha256": manifest_sha,
        "slsa_provenance": slsa_summary,
        "error": error,
    }


# ─────────────────────────────────────────────────────────────────────
# Internal helpers
# ─────────────────────────────────────────────────────────────────────


async def _pull_sbom_blob(
    *, client, repo: str, manifest_sha: str, bundle_manifest,
) -> bytes | None:
    if bundle_manifest.sbom_digest:
        try:
            return await client.get_blob(
                repo=repo, digest=bundle_manifest.sbom_digest,
            )
        except OCIRegistryError:
            pass
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


async def _sbom_summary(
    *, client, repo: str, manifest_sha: str, bundle_manifest,
) -> dict[str, Any]:
    blob = await _pull_sbom_blob(
        client=client, repo=repo, manifest_sha=manifest_sha,
        bundle_manifest=bundle_manifest,
    )
    if blob is None:
        return {
            "present": False, "format": None, "spec_version": None,
            "components_count": 0, "vulnerabilities_count": 0,
            "by_severity": {},
        }
    try:
        parsed = parse_sbom(blob)
    except ValueError as exc:
        return {"present": True, "error": f"parse failed: {exc}"}
    return {
        "present": True,
        "format": "CycloneDX",
        "spec_version": parsed.spec_version,
        "components_count": parsed.components_count,
        "vulnerabilities_count": parsed.vulnerabilities_count,
        "by_severity": parsed.by_severity(),
    }


async def _verify_bundle_signature_inline(
    *, client, repo: str, manifest_bytes: bytes, manifest_sha: str,
) -> bool:
    """Inline signature verification used by `get_bundle` to gate SLSA L3.

    Returns True only if the Ed25519 signature checks out against a
    public key from the trust store. Returns False on any error
    (no signature, no trust store, no crypto lib, actual fail).
    """
    if not _CRYPTO_AVAILABLE:
        return False
    public_key = _public_key_pem()
    if public_key is None:
        return False
    try:
        sig_referrers = await client.get_referrers(
            repo=repo,
            digest=f"sha256:{manifest_sha}",
            artifact_type="application/vnd.efe.bundle.signature.ed25519+json",
        )
    except OCIRegistryError:
        return False
    if not sig_referrers:
        return False
    layers = sig_referrers[0].get("layers") or []
    if not layers or not layers[0].get("digest"):
        return False
    try:
        sig_blob = await client.get_blob(repo=repo, digest=layers[0]["digest"])
    except OCIRegistryError:
        return False
    result = verify_ed25519_signature(
        manifest_bytes=manifest_bytes,
        signature_bytes=sig_blob,
        public_key_pem=public_key,
        expected_manifest_sha256=manifest_sha,
    )
    return result.verified


async def _slsa_summary(
    *, client, repo: str, manifest_sha: str,
    signature_verified: bool = False,
) -> dict[str, Any]:
    summary: dict[str, Any] = {
        "present": False, "level": 0, "builder": None, "build_type": None,
        "subject_digest_match": None, "builder_trusted": None,
        "warnings": [], "error": None,
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
    if not layers or not layers[0].get("digest"):
        summary["error"] = "attestation manifest has no layers"
        return summary
    try:
        blob = await client.get_blob(repo=repo, digest=layers[0]["digest"])
    except OCIRegistryError as exc:
        summary["error"] = f"attestation blob fetch failed: {exc}"
        return summary
    try:
        provenance = parse_provenance(blob)
    except SLSAParseError as exc:
        summary["error"] = f"attestation parse failed: {exc}"
        return summary

    summary["present"] = True
    summary["builder"] = provenance.builder_id
    summary["build_type"] = provenance.build_type
    ok, reason = validate_subject_digest(
        provenance=provenance, expected_sha256=manifest_sha,
    )
    summary["subject_digest_match"] = ok
    if not ok:
        summary["warnings"].append(reason or "subject digest does not match manifest")
    summary["builder_trusted"] = is_builder_trusted(provenance=provenance)
    summary["level"] = infer_slsa_level(
        provenance, signature_verified=signature_verified,
    )
    summary["signature_verified_for_level"] = signature_verified
    return summary
