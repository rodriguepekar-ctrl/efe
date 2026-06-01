"""Ed25519 verification pin tests (M18 D47)."""
from __future__ import annotations

import hashlib

import pytest

from mcp_server.bundle.verify import (
    VerificationResult,
    compute_key_id,
    verify_ed25519_signature,
)


@pytest.fixture
def manifest_bytes() -> bytes:
    return b'{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json"}'


def test_compute_key_id_first_16_hex_chars():
    pem = b"FAKE-PEM"
    expected = hashlib.sha256(pem).hexdigest()[:16]
    assert compute_key_id(pem) == expected


def test_verify_valid_signature_returns_true(ed25519_keypair, manifest_bytes):
    signature = ed25519_keypair["sign"](manifest_bytes)
    pem = ed25519_keypair["public_pem"]
    result = verify_ed25519_signature(
        manifest_bytes=manifest_bytes,
        signature_bytes=signature,
        public_key_pem=pem,
    )
    assert result.verified is True
    assert result.signature_algorithm == "ed25519"
    assert result.signed_by_key_id is not None
    assert result.signed_by_key_id.startswith("ed25519:")
    assert result.error is None


def test_verify_invalid_signature_returns_false(ed25519_keypair, manifest_bytes):
    pem = ed25519_keypair["public_pem"]
    bogus_signature = b"\x00" * 64  # 64 zero bytes — not a valid sig
    result = verify_ed25519_signature(
        manifest_bytes=manifest_bytes,
        signature_bytes=bogus_signature,
        public_key_pem=pem,
    )
    assert result.verified is False
    assert "verification failed" in (result.error or "").lower()


def test_verify_manifest_sha_mismatch_detected(ed25519_keypair, manifest_bytes):
    signature = ed25519_keypair["sign"](manifest_bytes)
    pem = ed25519_keypair["public_pem"]
    result = verify_ed25519_signature(
        manifest_bytes=manifest_bytes,
        signature_bytes=signature,
        public_key_pem=pem,
        expected_manifest_sha256="0" * 64,
    )
    assert result.verified is False
    assert result.manifest_sha256_matches is False
    assert "mismatch" in (result.error or "").lower()


def test_verify_corrupted_signature_bytes_rejected(ed25519_keypair, manifest_bytes):
    pem = ed25519_keypair["public_pem"]
    truncated = ed25519_keypair["sign"](manifest_bytes)[:32]  # < 64 bytes
    result = verify_ed25519_signature(
        manifest_bytes=manifest_bytes,
        signature_bytes=truncated,
        public_key_pem=pem,
    )
    assert result.verified is False


def test_verify_wrong_key_rejected(manifest_bytes):
    """Sign with one key, verify with another."""
    from cryptography.hazmat.primitives import serialization
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

    signer = Ed25519PrivateKey.generate()
    other = Ed25519PrivateKey.generate()
    signature = signer.sign(manifest_bytes)
    other_pem = other.public_key().public_bytes(
        encoding=serialization.Encoding.PEM,
        format=serialization.PublicFormat.SubjectPublicKeyInfo,
    )
    result = verify_ed25519_signature(
        manifest_bytes=manifest_bytes,
        signature_bytes=signature,
        public_key_pem=other_pem,
    )
    assert result.verified is False


def test_verify_expected_sha_match_passes(ed25519_keypair, manifest_bytes):
    signature = ed25519_keypair["sign"](manifest_bytes)
    pem = ed25519_keypair["public_pem"]
    expected = hashlib.sha256(manifest_bytes).hexdigest()
    result = verify_ed25519_signature(
        manifest_bytes=manifest_bytes,
        signature_bytes=signature,
        public_key_pem=pem,
        expected_manifest_sha256=expected,
    )
    assert result.verified is True
    assert result.manifest_sha256_matches is True


def test_verify_pem_with_garbage_returns_error_not_raise(manifest_bytes, ed25519_keypair):
    signature = ed25519_keypair["sign"](manifest_bytes)
    result = verify_ed25519_signature(
        manifest_bytes=manifest_bytes,
        signature_bytes=signature,
        public_key_pem=b"-----BEGIN GARBAGE-----\nXX\n-----END GARBAGE-----",
    )
    assert result.verified is False
    assert result.error is not None


def test_verify_key_id_stable_across_calls(ed25519_keypair, manifest_bytes):
    signature = ed25519_keypair["sign"](manifest_bytes)
    pem = ed25519_keypair["public_pem"]
    r1 = verify_ed25519_signature(
        manifest_bytes=manifest_bytes,
        signature_bytes=signature,
        public_key_pem=pem,
    )
    r2 = verify_ed25519_signature(
        manifest_bytes=manifest_bytes,
        signature_bytes=signature,
        public_key_pem=pem,
    )
    assert r1.signed_by_key_id == r2.signed_by_key_id


def test_verify_no_subprocess_dependency():
    """Smoke check : verify.py never imports subprocess."""
    import mcp_server.bundle.verify as verify_module
    import inspect

    src = inspect.getsource(verify_module)
    assert "subprocess" not in src
    assert "os.system" not in src


def test_verify_uses_cryptography_lib(ed25519_keypair, manifest_bytes):
    """Confirm the verify path uses cryptography's Ed25519 verify."""
    signature = ed25519_keypair["sign"](manifest_bytes)
    pem = ed25519_keypair["public_pem"]
    result = verify_ed25519_signature(
        manifest_bytes=manifest_bytes,
        signature_bytes=signature,
        public_key_pem=pem,
    )
    assert isinstance(result, VerificationResult)
    assert result.verified is True


def test_verify_modified_manifest_after_sign_rejected(ed25519_keypair):
    original = b'{"schemaVersion":2}'
    signature = ed25519_keypair["sign"](original)
    pem = ed25519_keypair["public_pem"]
    tampered = b'{"schemaVersion":3}'
    result = verify_ed25519_signature(
        manifest_bytes=tampered,
        signature_bytes=signature,
        public_key_pem=pem,
    )
    assert result.verified is False
