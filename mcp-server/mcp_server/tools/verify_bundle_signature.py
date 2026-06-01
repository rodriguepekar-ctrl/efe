"""efe_verify_bundle_signature (M18 D47).

Cryptographic-only check : verify that a bundle's Ed25519 signature
matches the manifest content + a trusted public key.

Returns a focused result distinct from inspect_bundle (which also
returns sig status as a side effect of its general inspection).
"""
from __future__ import annotations

import os
from typing import Any

from ..bundle import (
    InvalidRegistryArg,
    OCIArtifactNotFound,
    OCIRegistryClient,
    OCIRegistryError,
    parse_manifest,
    parse_registry_allowlist,
    split_registry,
    verify_ed25519_signature,
)
from ..bundle.registry_arg import default_oci_factory
from ..bundle.verify import _CRYPTO_AVAILABLE  # type: ignore[attr-defined]
from .types import Tool, ToolContext, ToolError


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeVerifyBundleSignatureInput",
    "description": "Verify a bundle's Ed25519 signature against the trusted public key.",
    "properties": {
        "slug": {
            "type": "string", "pattern": r"^[a-z][a-z0-9\-]{0,99}$",
            "minLength": 1, "maxLength": 100,
        },
        "version": {
            "type": "string", "pattern": r"^v\d+\.\d+\.\d+(-[a-z0-9\-]{1,15})?$",
            "minLength": 1, "maxLength": 50,
        },
        "expected_key_id": {"type": "string", "maxLength": 100},
        "registry": {"type": "string", "minLength": 1, "maxLength": 200},
    },
    "required": ["slug", "version"],
    "additionalProperties": False,
}


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    slug = args["slug"]
    version = args["version"]
    expected_key_id = args.get("expected_key_id")
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

            referrers = await client.get_referrers(
                repo=repo,
                digest=f"sha256:{manifest_sha}",
                artifact_type="application/vnd.efe.bundle.signature.ed25519+json",
            )
            sig_present = bool(referrers)
            sig_blob: bytes | None = None
            if sig_present:
                layers = referrers[0].get("layers") or []
                if layers:
                    digest = layers[0].get("digest")
                    if digest:
                        sig_blob = await client.get_blob(repo=repo, digest=digest)
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

    if not _CRYPTO_AVAILABLE:
        return _unavailable_result(
            slug, version, manifest_sha,
            error="cryptography extra not installed",
        )

    public_key_pem = _load_public_key_material()
    if public_key_pem is None:
        return _unavailable_result(
            slug, version, manifest_sha,
            error="trust_store_unavailable",
        )

    if not sig_present or sig_blob is None:
        return {
            "slug": slug,
            "version": version,
            "verified": False,
            "signature_algorithm": None,
            "signed_by_key_id": None,
            "signed_at": bundle_manifest.produced_at,
            "expected_key_id_match": None,
            "details": {
                "manifest_sha256": manifest_sha,
                "manifest_sha256_matches": True,
                "signature_present_in_oci_referrers": False,
                "key_in_trust_store": True,
            },
            "warnings": ["bundle has no Ed25519 signature artifact"],
            "error": "unsigned_bundle",
        }

    result = verify_ed25519_signature(
        manifest_bytes=manifest_bytes,
        signature_bytes=sig_blob,
        public_key_pem=public_key_pem,
        expected_manifest_sha256=manifest_sha,
    )

    expected_match: bool | None = None
    if expected_key_id and result.signed_by_key_id:
        expected_match = (
            result.signed_by_key_id == expected_key_id
            or result.signed_by_key_id == f"ed25519:{expected_key_id}"
        )

    warnings: list[str] = []
    if bundle_manifest.is_dev_unsigned and result.verified:
        warnings.append(
            "bundle is marked is_dev_unsigned=true ; do not deploy in production"
        )
    if expected_match is False:
        warnings.append(
            f"signed_by_key_id={result.signed_by_key_id!r} does not match "
            f"expected_key_id={expected_key_id!r}"
        )

    return {
        "slug": slug,
        "version": version,
        "verified": result.verified and expected_match in (None, True),
        "signature_algorithm": result.signature_algorithm,
        "signed_by_key_id": result.signed_by_key_id,
        "signed_at": bundle_manifest.produced_at,
        "expected_key_id_match": expected_match,
        "details": {
            "manifest_sha256": result.manifest_sha256,
            "manifest_sha256_matches": result.manifest_sha256_matches,
            "signature_present_in_oci_referrers": True,
            "key_in_trust_store": True,
        },
        "warnings": warnings,
        "error": result.error,
    }


def _unavailable_result(
    slug: str, version: str, manifest_sha: str, *, error: str,
) -> dict[str, Any]:
    return {
        "slug": slug,
        "version": version,
        "verified": False,
        "verification_unavailable": True,
        "signature_algorithm": "ed25519",
        "signed_by_key_id": None,
        "details": {
            "manifest_sha256": manifest_sha,
            "manifest_sha256_matches": True,
            "signature_present_in_oci_referrers": None,
            "key_in_trust_store": False,
        },
        "warnings": [
            "verification unavailable — install efe-mcp-server[bundle] "
            "or set EFE_OCI_PUBLIC_KEY{,_FILE}",
        ],
        "error": error,
    }


def _load_public_key_material() -> bytes | None:
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
    name="efe_verify_bundle_signature",
    description=(
        "Verify a bundle's Ed25519 signature against the trusted public "
        "key. Pure-Python verification, never invokes cosign CLI. "
        "Returns verified:true only when signature checks out AND public "
        "key is in trust store. Read-only."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    profile_tier="agent-native",
)
