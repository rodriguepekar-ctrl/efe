"""Sprint **M13 D20** — Pin tests : sample bundle `bundles/all-MiniLM-L6-v2-en-1.0.0/`.

Cf. `docs/EFE_BUNDLES_EXTENSION_ARCHITECTURE.md` §4 + §16.2.

These pin tests verify the **structural invariants** of the M13 D20
sample bundle as bootstrapped by `scripts/release/bootstrap_dev_bundles.py`.

8 tests covering :
  1. Bundle directory present
  2. All canonical D16 v1.0 required files present
  3. Manifest schema_version == "1.0"
  4. Per-file SHA256 in manifest matches disk
  5. Ed25519 signature verifies against the dev pubkey
  6. Total bundle size under 30 MiB target (with placeholder slack)
  7. License SPDX == Apache-2.0
  8. HF revision is `dev-placeholder` or pinned 40-char SHA (T-M13-7)
"""

from __future__ import annotations

import base64
import hashlib
import json
import re
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
BUNDLES = REPO / "bundles"
BUNDLE_D20 = BUNDLES / "all-MiniLM-L6-v2-en-1.0.0"


def _sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _load_pubkey_b64(key_id: str) -> str:
    trust = json.loads((BUNDLES / "pubkeys.json").read_text(encoding="utf-8"))
    for k in trust["keys"]:
        if k["key_id"] == key_id:
            return k["ed25519_public"]
    raise AssertionError(f"key_id {key_id!r} not in trust store")


# ─────────────────────────────────────────────────────────────────────
# Test 1 : bundle directory present
# ─────────────────────────────────────────────────────────────────────


def test_d20_bundle_directory_present() -> None:
    """The canonical sample bundle directory must exist post-bootstrap."""
    assert BUNDLE_D20.is_dir(), (
        f"M13 D20 bundle missing : {BUNDLE_D20}\n"
        f"Re-run : python3 scripts/release/bootstrap_dev_bundles.py"
    )


# ─────────────────────────────────────────────────────────────────────
# Test 2 : all D16 v1.0 required files present
# ─────────────────────────────────────────────────────────────────────


def test_d20_bundle_required_files_present() -> None:
    """Canonical D16 v1.0 layout : 9 mandatory files."""
    required = [
        "manifest.json",
        "manifest.json.sig",
        "weights/model.onnx",
        "tokenizer/tokenizer.json",
        "tokenizer/tokenizer_config.json",
        "tokenizer/special_tokens_map.json",
        "tokenizer/vocab.txt",
        "runtime/runtime_config.json",
        "LICENSE",
        "README.md",
        "SBOM.json",
    ]
    missing = [r for r in required if not (BUNDLE_D20 / r).is_file()]
    assert not missing, f"D20 missing required files : {missing}"


# ─────────────────────────────────────────────────────────────────────
# Test 3 : manifest schema_version == "1.0"
# ─────────────────────────────────────────────────────────────────────


def test_d20_manifest_schema_v1_0() -> None:
    """D16 v1.0 schema invariants."""
    manifest = json.loads(
        (BUNDLE_D20 / "manifest.json").read_text(encoding="utf-8")
    )
    assert manifest["manifest_version"] == "1.0"
    assert manifest["framework"] == "onnx"
    assert manifest["dim"] == 384
    assert manifest["quantization"] == "int8-dynamic"
    # Bundle ID must match canonical regex
    bundle_id = manifest["bundle_id"]
    assert re.match(
        r"^[a-zA-Z0-9._/-]+@[0-9]+\.[0-9]+\.[0-9]+/[a-z]+/[0-9]+$",
        bundle_id,
    ), f"bundle_id {bundle_id!r} does not match D16 §3.2"
    assert "all-MiniLM-L6-v2" in bundle_id


# ─────────────────────────────────────────────────────────────────────
# Test 4 : per-file SHA256 matches disk
# ─────────────────────────────────────────────────────────────────────


def test_d20_manifest_per_file_sha256_matches_disk() -> None:
    """Every component's SHA-256 in manifest must match the on-disk file."""
    manifest = json.loads(
        (BUNDLE_D20 / "manifest.json").read_text(encoding="utf-8")
    )
    for c in manifest["components"]:
        f = BUNDLE_D20 / c["path"]
        if c.get("required", True):
            assert f.exists(), f"required component missing : {c['path']}"
        if not f.exists():
            continue
        actual = _sha256_file(f)
        assert actual == c["sha256"], (
            f"hash drift on {c['path']} : "
            f"manifest={c['sha256']}, disk={actual}"
        )
        assert f.stat().st_size == c["size_bytes"], (
            f"size drift on {c['path']} : "
            f"manifest={c['size_bytes']}, disk={f.stat().st_size}"
        )


# ─────────────────────────────────────────────────────────────────────
# Test 5 : Ed25519 sig verifies
# ─────────────────────────────────────────────────────────────────────


def test_d20_signature_verifies_against_pinned_pubkey() -> None:
    """Anti-regression : manifest.json.sig must validate against the
    `efe-dev-bundle-2026` pubkey from the trust store."""
    try:
        from cryptography.hazmat.primitives.asymmetric.ed25519 import (
            Ed25519PublicKey,
        )
        from cryptography.exceptions import InvalidSignature
    except ImportError:
        pytest.skip("cryptography library not installed")
    pub = Ed25519PublicKey.from_public_bytes(
        base64.b64decode(_load_pubkey_b64("efe-dev-bundle-2026"))
    )
    manifest_bytes = (BUNDLE_D20 / "manifest.json").read_bytes()
    sig_bytes = (BUNDLE_D20 / "manifest.json.sig").read_bytes()
    assert len(sig_bytes) == 64, (
        f"Ed25519 raw sig must be 64 bytes, got {len(sig_bytes)}"
    )
    pub.verify(sig_bytes, manifest_bytes)  # raises InvalidSignature on fail


# ─────────────────────────────────────────────────────────────────────
# Test 6 : total bundle size budget
# ─────────────────────────────────────────────────────────────────────


def test_d20_bundle_total_size_under_target() -> None:
    """Dev placeholder must be tiny ; real CI build ≤ 30 MiB."""
    total = sum(
        f.stat().st_size for f in BUNDLE_D20.rglob("*") if f.is_file()
    )
    target = 30 * (1 << 20)  # 30 MiB
    assert total <= target, (
        f"D20 bundle size {total:,} bytes > 30 MiB target. "
        f"Real CI build should still fit under target."
    )


# ─────────────────────────────────────────────────────────────────────
# Test 7 : license SPDX
# ─────────────────────────────────────────────────────────────────────


def test_d20_license_spdx_apache_2_0() -> None:
    """Sample bundle license must be Apache-2.0 + redistribution allowed."""
    manifest = json.loads(
        (BUNDLE_D20 / "manifest.json").read_text(encoding="utf-8")
    )
    assert manifest["license"]["spdx_id"] == "Apache-2.0"
    assert manifest["license"]["redistribution_allowed"] is True
    license_text = (BUNDLE_D20 / "LICENSE").read_text(encoding="utf-8")
    # LICENSE file must contain the Apache header marker
    assert "Apache License" in license_text


# ─────────────────────────────────────────────────────────────────────
# Test 8 : T-M13-7 — HF revision pinned (anti-regression)
# ─────────────────────────────────────────────────────────────────────


def test_d20_hf_revision_pinned_or_dev_placeholder() -> None:
    """Threat T-M13-7 mitigation : huggingface_revision must be either
    a 40-char hex SHA (CI build) or the literal `dev-placeholder` (dev
    bootstrap). NEVER `main` or `master`."""
    manifest = json.loads(
        (BUNDLE_D20 / "manifest.json").read_text(encoding="utf-8")
    )
    rev = manifest["model_origin"]["huggingface_revision"]
    assert rev not in ("main", "master"), (
        f"T-M13-7 violation : hf_revision must be pinned, got {rev!r}"
    )
    if rev != "dev-placeholder":
        assert len(rev) == 40 and all(c in "0123456789abcdef" for c in rev), (
            f"hf_revision must be 40-char hex SHA or dev-placeholder, "
            f"got {rev!r}"
        )
