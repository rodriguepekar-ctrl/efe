"""Sprint **M13 D26** — Pin tests : production multilingual bundle
`bundles/paraphrase-multilingual-MiniLM-L12-v2-1.0.0/`.

Cf. `docs/EFE_BUNDLES_EXTENSION_ARCHITECTURE.md` §5 + §16.3.

10 tests covering :
  1. Bundle directory present
  2. All canonical D16 v1.0 + sentencepiece-specific files present
  3. Manifest schema_version == "1.0"
  4. SBOM CycloneDX 1.5 format valid
  5. SBOM declares a `machine-learning-model` component
  6. SBOM declares a tokenizer `data` component
  7. Ed25519 signature verifies against the dev pubkey
  8. Total bundle size under 80 MiB target
  9. License SPDX == Apache-2.0
  10. License gate accepts the bundle
"""

from __future__ import annotations

import base64
import hashlib
import json
import sys
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
BUNDLES = REPO / "bundles"
BUNDLE_D26 = BUNDLES / "paraphrase-multilingual-MiniLM-L12-v2-1.0.0"


def _load_pubkey_b64(key_id: str) -> str:
    trust = json.loads((BUNDLES / "pubkeys.json").read_text(encoding="utf-8"))
    for k in trust["keys"]:
        if k["key_id"] == key_id:
            return k["ed25519_public"]
    raise AssertionError(f"key_id {key_id!r} not in trust store")


# ─────────────────────────────────────────────────────────────────────
# Test 1 : bundle present
# ─────────────────────────────────────────────────────────────────────


def test_d26_bundle_directory_present() -> None:
    assert BUNDLE_D26.is_dir(), (
        f"M13 D26 bundle missing : {BUNDLE_D26}\n"
        f"Re-run : python3 scripts/release/bootstrap_dev_bundles.py"
    )


# ─────────────────────────────────────────────────────────────────────
# Test 2 : required files (D26 has sentencepiece, D20 has vocab.txt)
# ─────────────────────────────────────────────────────────────────────


def test_d26_bundle_required_files_present() -> None:
    """Multilingual layout : XLM-R sentencepiece vs WordPiece for D20."""
    required = [
        "manifest.json",
        "manifest.json.sig",
        "weights/model.onnx",
        "tokenizer/tokenizer.json",
        "tokenizer/tokenizer_config.json",
        "tokenizer/special_tokens_map.json",
        "tokenizer/sentencepiece.bpe.model",  # XLM-R specific
        "runtime/runtime_config.json",
        "LICENSE",
        "README.md",
        "SBOM.json",
    ]
    missing = [r for r in required if not (BUNDLE_D26 / r).is_file()]
    assert not missing, f"D26 missing : {missing}"


# ─────────────────────────────────────────────────────────────────────
# Test 3 : manifest schema
# ─────────────────────────────────────────────────────────────────────


def test_d26_manifest_schema_v1_0() -> None:
    manifest = json.loads(
        (BUNDLE_D26 / "manifest.json").read_text(encoding="utf-8")
    )
    assert manifest["manifest_version"] == "1.0"
    assert manifest["framework"] == "onnx"
    assert manifest["dim"] == 384
    # D26 uses FP16 (not INT8 like D20) for multilingual quality
    assert manifest["quantization"] == "fp16"
    assert manifest["runtime"]["dtype"] == "float16"
    assert manifest["runtime"]["max_seq_len"] == 128
    assert "paraphrase-multilingual-MiniLM-L12-v2" in manifest["bundle_id"]


# ─────────────────────────────────────────────────────────────────────
# Test 4 : SBOM CycloneDX 1.5
# ─────────────────────────────────────────────────────────────────────


def test_d26_sbom_cyclonedx_1_5() -> None:
    """SBOM must declare CycloneDX 1.5 format."""
    sbom = json.loads(
        (BUNDLE_D26 / "SBOM.json").read_text(encoding="utf-8")
    )
    assert sbom["bomFormat"] == "CycloneDX"
    assert sbom["specVersion"] == "1.5"
    assert "components" in sbom and len(sbom["components"]) >= 2
    # Serial number must be a urn:uuid:* (CycloneDX spec)
    assert sbom["serialNumber"].startswith("urn:uuid:")


# ─────────────────────────────────────────────────────────────────────
# Test 5 : SBOM has machine-learning-model component
# ─────────────────────────────────────────────────────────────────────


def test_d26_sbom_includes_machine_learning_model() -> None:
    """ML-BOM extension : at least one component with type=machine-learning-model."""
    sbom = json.loads(
        (BUNDLE_D26 / "SBOM.json").read_text(encoding="utf-8")
    )
    ml_models = [
        c for c in sbom["components"]
        if c.get("type") == "machine-learning-model"
    ]
    assert len(ml_models) >= 1, "SBOM must include at least one ML model component"
    # Verify the ML model has core ML-BOM properties
    model = ml_models[0]
    assert "purl" in model
    assert model["purl"].startswith("pkg:huggingface/")
    assert model.get("licenses"), "ML model must declare a license"


# ─────────────────────────────────────────────────────────────────────
# Test 6 : SBOM has tokenizer data component
# ─────────────────────────────────────────────────────────────────────


def test_d26_sbom_includes_tokenizer_data_component() -> None:
    sbom = json.loads(
        (BUNDLE_D26 / "SBOM.json").read_text(encoding="utf-8")
    )
    data_comps = [c for c in sbom["components"] if c.get("type") == "data"]
    assert len(data_comps) >= 1, "SBOM must include tokenizer data component"
    tokenizer = next(
        (c for c in data_comps if "tokenizer" in c.get("bom-ref", "")),
        None,
    )
    assert tokenizer is not None, "SBOM must declare a tokenizer data component"


# ─────────────────────────────────────────────────────────────────────
# Test 7 : signature verifies
# ─────────────────────────────────────────────────────────────────────


def test_d26_signature_verifies() -> None:
    try:
        from cryptography.hazmat.primitives.asymmetric.ed25519 import (
            Ed25519PublicKey,
        )
    except ImportError:
        pytest.skip("cryptography not installed")
    pub = Ed25519PublicKey.from_public_bytes(
        base64.b64decode(_load_pubkey_b64("efe-dev-bundle-2026"))
    )
    sig = (BUNDLE_D26 / "manifest.json.sig").read_bytes()
    assert len(sig) == 64
    pub.verify(sig, (BUNDLE_D26 / "manifest.json").read_bytes())


# ─────────────────────────────────────────────────────────────────────
# Test 8 : size budget 80 MiB
# ─────────────────────────────────────────────────────────────────────


def test_d26_total_size_under_80mib() -> None:
    total = sum(
        f.stat().st_size for f in BUNDLE_D26.rglob("*") if f.is_file()
    )
    target = 80 * (1 << 20)
    assert total <= target, (
        f"D26 bundle size {total:,} > 80 MiB target {target:,}"
    )


# ─────────────────────────────────────────────────────────────────────
# Test 9 : license Apache-2.0
# ─────────────────────────────────────────────────────────────────────


def test_d26_license_apache_2_0() -> None:
    manifest = json.loads(
        (BUNDLE_D26 / "manifest.json").read_text(encoding="utf-8")
    )
    assert manifest["license"]["spdx_id"] == "Apache-2.0"
    assert manifest["license"]["redistribution_allowed"] is True


# ─────────────────────────────────────────────────────────────────────
# Test 10 : license_gate accepts
# ─────────────────────────────────────────────────────────────────────


def test_d26_license_gate_accepts() -> None:
    """Integration with license_gate.py — Apache-2.0 must pass."""
    sys.path.insert(0, str(REPO / "scripts" / "release"))
    try:
        from license_gate import gate_license, ALLOWLIST
    finally:
        sys.path.pop(0)
    manifest = json.loads(
        (BUNDLE_D26 / "manifest.json").read_text(encoding="utf-8")
    )
    spdx = gate_license(manifest, ack_nonredist=False)
    assert spdx == "Apache-2.0"
    assert spdx in ALLOWLIST
