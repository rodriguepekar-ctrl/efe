"""Pure-Python Ed25519 signature verification (M18 D47).

NEVER invokes `cosign` CLI. Uses `cryptography.hazmat` for the actual
ECC math. The MCP server holds public keys only — never signs anything.

Threat mitigations :
  - TM-M18-S-01 (forged signature) : cryptography.exceptions.InvalidSignature
    is rigorously caught and surfaced as `verified=False`.
  - TM-M18-T-01 (tampered manifest) : we recompute SHA-256 over the
    raw bytes the registry served and compare with the
    `expected_manifest_sha256` (typically baked into the signature
    payload by the signer). Mismatch → `verified=False` with reason.
  - TM-M18-S-02 (key sub) : we expose `signed_by_key_id` so callers
    can cross-check against an `expected_key_id` allowlist.
"""
from __future__ import annotations

import hashlib
from dataclasses import dataclass
from typing import Optional


# `cryptography` is in the [bundle] extra. Importing lazily so the
# default install (without extras) doesn't break at import time. If
# missing, `verify_ed25519_signature` raises a structured error so
# callers can fall back to `verification_unavailable: true`.
try:
    from cryptography.exceptions import InvalidSignature
    from cryptography.hazmat.primitives.asymmetric.ed25519 import (
        Ed25519PublicKey,
    )
    from cryptography.hazmat.primitives.serialization import (
        load_pem_public_key,
        load_der_public_key,
    )
    _CRYPTO_AVAILABLE = True
    _CRYPTO_IMPORT_ERROR: Exception | None = None
except ImportError as _exc:   # pragma: no cover — exercised only when extra missing
    _CRYPTO_AVAILABLE = False
    _CRYPTO_IMPORT_ERROR = _exc

    class InvalidSignature(Exception):  # type: ignore[no-redef]
        pass


@dataclass(frozen=True)
class VerificationResult:
    verified: bool
    signature_algorithm: str           # 'ed25519'
    signed_by_key_id: Optional[str]    # 'ed25519:abc123…' (16-hex)
    manifest_sha256: str               # hash of the raw manifest bytes
    manifest_sha256_matches: bool      # comparison with caller's expectation
    error: Optional[str]


class VerificationUnavailable(RuntimeError):
    """Raised when the `cryptography` extra is not installed.

    Tools should catch this and surface `verification_unavailable: true`
    in their response — never silently return `verified: true`.
    """


# ─────────────────────────────────────────────────────────────────────
# Public API
# ─────────────────────────────────────────────────────────────────────


def compute_key_id(public_key_pem: bytes) -> str:
    """Stable 16-hex-char identifier for an Ed25519 public key.

    Used in `signed_by_key_id` and matches the `expected_key_id` user
    arg. Equal to `sha256(public_key_pem)[:16]` (first 16 hex chars).
    """
    return hashlib.sha256(public_key_pem).hexdigest()[:16]


def _load_public_key(material: bytes):
    """Load an Ed25519 public key from PEM or DER bytes."""
    if not _CRYPTO_AVAILABLE:
        raise VerificationUnavailable(
            "`cryptography` extra is not installed ; "
            "install with `pip install efe-mcp-server[bundle]`"
        )
    # Heuristic : PEM starts with `-----BEGIN`.
    try:
        if material.lstrip().startswith(b"-----BEGIN"):
            key = load_pem_public_key(material)
        else:
            key = load_der_public_key(material)
    except Exception as exc:  # noqa: BLE001 — wrapper
        raise ValueError(f"failed to load public key: {exc}") from exc
    if not isinstance(key, Ed25519PublicKey):
        raise ValueError(
            f"public key is not Ed25519 ; got {type(key).__name__}"
        )
    return key


def verify_ed25519_signature(
    *,
    manifest_bytes: bytes,
    signature_bytes: bytes,
    public_key_pem: bytes,
    expected_manifest_sha256: str | None = None,
) -> VerificationResult:
    """Verify an Ed25519 signature over OCI manifest content.

    Parameters
    ----------
    manifest_bytes
        The raw bytes the OCI registry served for the manifest. Must
        be the EXACT bytes (whitespace included) — re-serialising the
        parsed JSON would produce different bytes.
    signature_bytes
        Raw 64-byte Ed25519 signature, as published in the OCI
        referrer artifact's signature blob.
    public_key_pem
        PEM-encoded Ed25519 public key. May also be DER (auto-detected).
    expected_manifest_sha256
        Optional cross-check : if the signer baked the digest into the
        signature payload, the caller passes it here. Mismatch is
        reported in `manifest_sha256_matches`.
    """
    actual_sha = hashlib.sha256(manifest_bytes).hexdigest()
    matches = (
        expected_manifest_sha256 is None
        or expected_manifest_sha256.lower() == actual_sha.lower()
    )
    if not matches:
        return VerificationResult(
            verified=False,
            signature_algorithm="ed25519",
            signed_by_key_id=None,
            manifest_sha256=actual_sha,
            manifest_sha256_matches=False,
            error=(
                f"manifest sha256 mismatch : expected "
                f"{expected_manifest_sha256!r}, got {actual_sha!r}"
            ),
        )

    if not _CRYPTO_AVAILABLE:
        return VerificationResult(
            verified=False,
            signature_algorithm="ed25519",
            signed_by_key_id=None,
            manifest_sha256=actual_sha,
            manifest_sha256_matches=True,
            error=(
                "cryptography extra not installed ; "
                "install with pip install efe-mcp-server[bundle]"
            ),
        )

    try:
        key = _load_public_key(public_key_pem)
    except ValueError as exc:
        return VerificationResult(
            verified=False,
            signature_algorithm="ed25519",
            signed_by_key_id=None,
            manifest_sha256=actual_sha,
            manifest_sha256_matches=True,
            error=str(exc),
        )

    try:
        key.verify(signature_bytes, manifest_bytes)
    except InvalidSignature:
        return VerificationResult(
            verified=False,
            signature_algorithm="ed25519",
            signed_by_key_id=None,
            manifest_sha256=actual_sha,
            manifest_sha256_matches=True,
            error="Ed25519 signature verification failed",
        )
    except Exception as exc:  # noqa: BLE001 — defensive
        return VerificationResult(
            verified=False,
            signature_algorithm="ed25519",
            signed_by_key_id=None,
            manifest_sha256=actual_sha,
            manifest_sha256_matches=True,
            error=f"verification error: {exc}",
        )

    return VerificationResult(
        verified=True,
        signature_algorithm="ed25519",
        signed_by_key_id=f"ed25519:{compute_key_id(public_key_pem)}",
        manifest_sha256=actual_sha,
        manifest_sha256_matches=True,
        error=None,
    )
