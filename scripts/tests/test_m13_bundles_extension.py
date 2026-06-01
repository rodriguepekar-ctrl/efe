"""Sprint **M13** — Cross-cutting pin tests for the entire `bundles/`
extension : all bundles + trust store + JSON Schema + dev signing key
+ multilingual fixture + index README.

Cf. `docs/EFE_BUNDLES_EXTENSION_ARCHITECTURE.md` §16.5.

8 tests covering :
  1. `bundles/` directory inventory matches expected slug list
  2. `bundles/README.md` index present + lists both bundles
  3. Every bundle has manifest + sig + LICENSE
  4. Every bundle's manifest validates against D16 v1.0 JSON Schema
  5. No bundle uses revision='main' / 'master' (T-M13-7 anti-regression)
  6. No bundle uses a FORBIDDEN license (T-M13-8 anti-regression)
  7. `pubkeys.json` trust store is well-formed + has dev key
  8. `dev_signing_key.b64` ships with clear "DEV ONLY" warning
"""

from __future__ import annotations

import base64
import json
import re
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
BUNDLES = REPO / "bundles"
SCHEMAS = REPO / "crates" / "efe-server" / "schemas"
DEV_KEY = REPO / "scripts" / "release" / "dev_signing_key.b64"

EXPECTED_BUNDLES = {
    "all-MiniLM-L6-v2-en-1.0.0",
    "paraphrase-multilingual-MiniLM-L12-v2-1.0.0",
}


def _list_bundle_dirs() -> list[Path]:
    return [
        d for d in BUNDLES.iterdir()
        if d.is_dir() and (d / "manifest.json").is_file()
    ]


# ─────────────────────────────────────────────────────────────────────
# Test 1 : inventory
# ─────────────────────────────────────────────────────────────────────


def test_m13_bundles_directory_inventory() -> None:
    actual = {d.name for d in _list_bundle_dirs()}
    missing = EXPECTED_BUNDLES - actual
    assert not missing, (
        f"M13 inventory missing bundles : {missing}\n"
        f"Re-run : python3 scripts/release/bootstrap_dev_bundles.py"
    )


# ─────────────────────────────────────────────────────────────────────
# Test 2 : index README
# ─────────────────────────────────────────────────────────────────────


def test_m13_bundles_index_readme_present() -> None:
    readme = BUNDLES / "README.md"
    assert readme.is_file(), f"M13 index missing : {readme}"
    text = readme.read_text(encoding="utf-8")
    for slug in EXPECTED_BUNDLES:
        assert slug in text, (
            f"M13 index README must mention {slug}"
        )
    # Must reference D16 spec + M13 architecture
    assert "D16 v1.0" in text
    assert "EFE_BUNDLES_EXTENSION_ARCHITECTURE.md" in text


# ─────────────────────────────────────────────────────────────────────
# Test 3 : every bundle has the canonical files
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.parametrize("bundle_dir", _list_bundle_dirs(), ids=lambda b: b.name)
def test_m13_all_bundles_have_required_canonical_files(bundle_dir: Path) -> None:
    required_top_level = ["manifest.json", "manifest.json.sig", "LICENSE", "README.md"]
    missing = [f for f in required_top_level if not (bundle_dir / f).is_file()]
    assert not missing, f"{bundle_dir.name} missing : {missing}"


# ─────────────────────────────────────────────────────────────────────
# Test 4 : every manifest validates against D16 v1.0 schema
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.parametrize("bundle_dir", _list_bundle_dirs(), ids=lambda b: b.name)
def test_m13_manifest_validates_against_d16_schema(bundle_dir: Path) -> None:
    try:
        import jsonschema
    except ImportError:
        pytest.skip("jsonschema not installed")
    schema_path = SCHEMAS / "d16_manifest.schema.json"
    assert schema_path.is_file(), f"D16 schema missing : {schema_path}"
    schema = json.loads(schema_path.read_text(encoding="utf-8"))
    manifest = json.loads(
        (bundle_dir / "manifest.json").read_text(encoding="utf-8")
    )
    jsonschema.validate(instance=manifest, schema=schema)


# ─────────────────────────────────────────────────────────────────────
# Test 5 : T-M13-7 anti-regression
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.parametrize("bundle_dir", _list_bundle_dirs(), ids=lambda b: b.name)
def test_m13_no_bundle_uses_revision_main(bundle_dir: Path) -> None:
    manifest = json.loads(
        (bundle_dir / "manifest.json").read_text(encoding="utf-8")
    )
    rev = manifest.get("model_origin", {}).get("huggingface_revision", "")
    assert rev not in ("main", "master"), (
        f"T-M13-7 violation in {bundle_dir.name} : hf_revision = {rev!r}"
    )
    if rev != "dev-placeholder":
        assert re.match(r"^[0-9a-f]{40}$", rev), (
            f"hf_revision in {bundle_dir.name} must be 40-char hex SHA "
            f"or 'dev-placeholder', got {rev!r}"
        )


# ─────────────────────────────────────────────────────────────────────
# Test 6 : T-M13-8 anti-regression — no FORBIDDEN license
# ─────────────────────────────────────────────────────────────────────


@pytest.mark.parametrize("bundle_dir", _list_bundle_dirs(), ids=lambda b: b.name)
def test_m13_no_bundle_in_forbidden_license(bundle_dir: Path) -> None:
    import sys
    sys.path.insert(0, str(REPO / "scripts" / "release"))
    try:
        from license_gate import gate_license, LicenseGateFailure, FORBIDDEN
    finally:
        sys.path.pop(0)
    manifest = json.loads(
        (bundle_dir / "manifest.json").read_text(encoding="utf-8")
    )
    spdx = manifest["license"]["spdx_id"]
    assert spdx not in FORBIDDEN, (
        f"T-M13-8 violation in {bundle_dir.name} : license {spdx} FORBIDDEN"
    )
    # Also validate via the gate (will raise on RESTRICTED-without-ack too)
    gate_license(manifest, ack_nonredist=False)


# ─────────────────────────────────────────────────────────────────────
# Test 7 : trust store well-formed
# ─────────────────────────────────────────────────────────────────────


def test_m13_pubkeys_trust_store_present_and_well_formed() -> None:
    trust_path = BUNDLES / "pubkeys.json"
    assert trust_path.is_file(), f"M13 trust store missing : {trust_path}"
    trust = json.loads(trust_path.read_text(encoding="utf-8"))
    assert trust["schema_version"] == "1.0"
    assert isinstance(trust["keys"], list) and len(trust["keys"]) >= 1
    # Dev key must be present
    dev_key = next(
        (k for k in trust["keys"] if k["key_id"] == "efe-dev-bundle-2026"),
        None,
    )
    assert dev_key is not None, (
        "Trust store must include the M13 dev key (efe-dev-bundle-2026)"
    )
    # Pubkey must be 32 raw bytes (44 base64 chars including padding)
    pub_b64 = dev_key["ed25519_public"]
    pub_raw = base64.b64decode(pub_b64)
    assert len(pub_raw) == 32, (
        f"Ed25519 pubkey must be 32 raw bytes, got {len(pub_raw)}"
    )
    # Dev key must NOT be active for prod signing
    assert dev_key.get("active_for_signing") is False, (
        "Dev key must NEVER be active_for_signing in prod"
    )


# ─────────────────────────────────────────────────────────────────────
# Test 8 : dev_signing_key.b64 has clear warning markers
# ─────────────────────────────────────────────────────────────────────


def test_m13_dev_signing_key_present_and_clearly_marked() -> None:
    """Threat T-M13-1 / T-M13-6 anti-regression : the committed dev
    privkey MUST have unmistakable warning markers so a future engineer
    doesn't accidentally use it for production."""
    assert DEV_KEY.is_file(), f"M13 dev key missing : {DEV_KEY}"
    text = DEV_KEY.read_text(encoding="utf-8")
    assert "DO NOT USE" in text.upper() or "DEV-ONLY" in text.upper(), (
        "Dev signing key file must have unmistakable warning markers"
    )
    assert "PRODUCTION" in text.upper(), (
        "Dev signing key file must reference 'production' in warning context"
    )
    # Must contain exactly one base64 line (32-byte seed)
    b64_lines = [
        line.strip() for line in text.splitlines()
        if line.strip() and not line.strip().startswith("#")
    ]
    assert len(b64_lines) == 1, (
        f"Dev key file must have exactly 1 base64 line, got {len(b64_lines)}"
    )
    raw = base64.b64decode(b64_lines[0])
    assert len(raw) == 32, f"Dev key seed must be 32 bytes, got {len(raw)}"
