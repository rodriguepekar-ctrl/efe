"""Sprint **M13** — Pin tests anti-régression : audit chirurgical pass 1.

Cf. CHANGELOG.md `### M13 audit pass 1` for the 7 defects.

7 audit-fix pin tests (1 per defect) anchored on structural invariants
of the fix, NOT on the audit-fix marker comment that mentions the
pre-fix pattern.

  * **A1** (CRIT) build_real_bundle.py --dev syncs to --out
  * **A2** (HIGH) CI workflow OCI tag uses canonical <name>:<version>
  * **A3** (HIGH) CI workflow setup detects + strips trailing -X.Y.Z
  * **A4** (MED)  bootstrap sets is_dev_unsigned: true on placeholders
  * **A5** (MED)  build_real_bundle signing.pubkey_id is parametric
  * **A6** (LOW)  KNOWN_MODELS no `all-MiniLM-L6-v2-1.0.0` (no -en) duplicate
  * **A7** (LOW)  dev key valid_until ≤ 2030-12-31 (5-year window)
"""

from __future__ import annotations

import json
import re
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
WORKFLOW = REPO / ".github" / "workflows" / "bundle-release.yml"
BUILD_REAL = REPO / "scripts" / "release" / "build_real_bundle.py"
BOOTSTRAP = REPO / "scripts" / "release" / "bootstrap_dev_bundles.py"
PUBKEYS = REPO / "bundles" / "pubkeys.json"


# ─────────────────────────────────────────────────────────────────────
# A1 (CRIT) — build_real_bundle.py --dev mode syncs to --out
# ─────────────────────────────────────────────────────────────────────


def test_m13_audit_a1_build_real_bundle_dev_syncs_to_out() -> None:
    """A1 (CRIT) — pre-fix `build_real_bundle.py --dev` delegated to
    bootstrap (which writes to `bundles/<slug>/`) and then tried to
    read `<--out>/manifest.json`. When --out differed from the canonical
    bundles dir, the read failed with FileNotFoundError. Post-fix the
    code copies the bundle to --out after bootstrap completes."""
    body = BUILD_REAL.read_text(encoding="utf-8")
    # Anchor on the active sync logic (NOT the comment — the comment
    # may include the pre-fix pattern as documentation).
    assert "shutil.copytree(canonical_dir, out_dir)" in body, (
        "A1 regression : --dev mode must copy canonical bundle dir to --out"
    )
    # Anchor on the canonical_dir derivation
    assert "canonical_dir = bundles_root / slug" in body
    # Anchor on the .resolve() comparison (avoids same-dir shutil error)
    assert "out_dir.resolve() != canonical_dir.resolve()" in body


# ─────────────────────────────────────────────────────────────────────
# A2 (HIGH) — CI workflow OCI tag canonical <name>:<version>
# ─────────────────────────────────────────────────────────────────────


def test_m13_audit_a2_workflow_oci_tag_uses_bundle_name_not_slug() -> None:
    """A2 (HIGH) — pre-fix the OCI push tag was
    `${registry}/${bundle_slug}:${bundle_version}` which produced
    `<name>-<v>:<v>` (redundant). Post-fix uses
    `${registry}/${bundle_name}:${bundle_version}` which produces
    canonical `<name>:<v>` per arch §3.3."""
    text = WORKFLOW.read_text(encoding="utf-8")
    # The active oras push command must reference bundle_name (NOT bundle_slug)
    # in the tag construction.
    push_pattern = re.compile(
        r'oras push\s*\\\s*\n\s*"([^"]+)"',
        re.MULTILINE,
    )
    matches = push_pattern.findall(text)
    assert matches, "A2 regression : couldn't find any `oras push` invocation"
    for tag in matches:
        # Expect the form `<registry>/${{ ... bundle_name ... }}:${{ ... bundle_version ... }}`
        assert "bundle_name" in tag, (
            f"A2 regression : oras push tag must reference bundle_name "
            f"(canonical OCI), got : {tag!r}"
        )
    # The `oras pull` in verify-pull job must also use bundle_name
    pull_pattern = re.compile(r'oras pull\s+"([^"]+)"')
    pulls = pull_pattern.findall(text)
    for tag in pulls:
        assert "bundle_name" in tag, (
            f"A2 regression : oras pull tag must use bundle_name, got : {tag!r}"
        )


# ─────────────────────────────────────────────────────────────────────
# A3 (HIGH) — CI workflow setup detects + strips trailing -X.Y.Z
# ─────────────────────────────────────────────────────────────────────


def test_m13_audit_a3_workflow_setup_strips_trailing_version_from_slug() -> None:
    """A3 (HIGH) — pre-fix `BUNDLE_SLUG="${input}-${version}"` blindly
    appended version, doubling it when the user provided a slug with
    version (e.g., `all-MiniLM-L6-v2-en-1.0.0` + `1.0.0` →
    `all-MiniLM-L6-v2-en-1.0.0-1.0.0`). Post-fix uses bash regex
    `^(.+)-([0-9]+\\.[0-9]+\\.[0-9]+)$` to detect and strip."""
    text = WORKFLOW.read_text(encoding="utf-8")
    # Anchor on the bash regex pattern that strips the trailing version
    assert "BUNDLE_NAME_INPUT" in text, (
        "A3 regression : workflow must use a separate BUNDLE_NAME_INPUT variable"
    )
    assert r"[0-9]+\.[0-9]+\.[0-9]+" in text, (
        "A3 regression : workflow must use bash regex to detect trailing -X.Y.Z"
    )
    # Anchor on the version-mismatch check (sanity for user error)
    assert (
        "INPUT_VERSION_PART" in text
        and 'INPUT_VERSION_PART}" != "${BUNDLE_VERSION}' in text
    ), "A3 regression : workflow must validate input version matches bundle_version"
    # Setup outputs must declare bundle_name (canonical OCI name)
    assert "bundle_name=${BUNDLE_NAME}" in text, (
        "A3 regression : setup step must export bundle_name to outputs"
    )


# ─────────────────────────────────────────────────────────────────────
# A4 (MED) — bootstrap sets is_dev_unsigned: true
# ─────────────────────────────────────────────────────────────────────


def test_m13_audit_a4_bootstrap_marks_placeholders_as_dev_unsigned() -> None:
    """A4 (MED) — pre-fix the bootstrap set `is_dev_unsigned: false`
    because the bundles ARE signed (with the dev key). But the
    architecture doc semantic is `is_dev_unsigned=true` for placeholder
    bundles (no real model) so loaders refuse them in production. Pre-fix
    a placeholder could be loaded by a prod efe-server pod despite
    `EFE_BUNDLE_REQUIRE_SIGNATURE=true`. Post-fix : true for all
    bootstrap-produced bundles."""
    body = BOOTSTRAP.read_text(encoding="utf-8")
    # Anchor on the active manifest_v1 dict literal (the comment may
    # include the pre-fix pattern as audit documentation, so we look
    # for the LITERAL key=value pair).
    # The active line should be `"is_dev_unsigned": True,` (NOT False).
    # Re-bootstrap was already run, so we also verify the on-disk bundles.
    bundles_root = REPO / "bundles"
    for bundle_dir in (bundles_root / "all-MiniLM-L6-v2-en-1.0.0",
                       bundles_root / "paraphrase-multilingual-MiniLM-L12-v2-1.0.0"):
        if not bundle_dir.is_dir():
            continue
        manifest = json.loads((bundle_dir / "manifest.json").read_text(encoding="utf-8"))
        assert manifest.get("is_dev_unsigned") is True, (
            f"A4 regression : {bundle_dir.name} manifest has "
            f"is_dev_unsigned={manifest.get('is_dev_unsigned')!r}, "
            f"expected True for placeholders"
        )
    # Source-level anchor : the bootstrap source must contain `True` value
    # for the is_dev_unsigned key.
    source_pattern = re.compile(
        r'"is_dev_unsigned":\s*True', re.MULTILINE
    )
    assert source_pattern.search(body), (
        "A4 regression : bootstrap_dev_bundles.py must set "
        "`is_dev_unsigned`: True on placeholder manifests"
    )


# ─────────────────────────────────────────────────────────────────────
# A5 (MED) — build_real_bundle signing.pubkey_id parametric
# ─────────────────────────────────────────────────────────────────────


def test_m13_audit_a5_build_real_bundle_signing_pubkey_id_is_parametric() -> None:
    """A5 (MED) — pre-fix `signing.pubkey_id` was hardcoded to
    `efe-prod-bundle-2026q1` regardless of the actual privkey used.
    Post-fix : it's a CLI flag `--signing-pubkey-id` threaded through
    `build()` + `phase_write_manifest()`."""
    body = BUILD_REAL.read_text(encoding="utf-8")
    # CLI flag must be declared
    assert "--signing-pubkey-id" in body, (
        "A5 regression : build_real_bundle.py must accept --signing-pubkey-id flag"
    )
    # Function signatures must accept the parameter
    assert "signing_pubkey_id: str" in body, (
        "A5 regression : signing_pubkey_id must be a typed function parameter"
    )
    # The manifest dict literal must reference the parameter (not a literal)
    pat = re.compile(
        r'"pubkey_id":\s*signing_pubkey_id,', re.MULTILINE
    )
    assert pat.search(body), (
        "A5 regression : manifest's signing.pubkey_id must reference the "
        "signing_pubkey_id parameter (not a hardcoded literal)"
    )


# ─────────────────────────────────────────────────────────────────────
# A6 (LOW) — KNOWN_MODELS de-duplication
# ─────────────────────────────────────────────────────────────────────


def test_m13_audit_a6_known_models_no_duplicate_no_lang_entry() -> None:
    """A6 (LOW) — pre-fix `KNOWN_MODELS` had a redundant
    `all-MiniLM-L6-v2-1.0.0` entry (without `-en` lang suffix)
    duplicating `all-MiniLM-L6-v2-en-1.0.0`. Post-fix only the
    canonical `<name>-<lang>-<version>` form remains."""
    body = BUILD_REAL.read_text(encoding="utf-8")
    # The bare (no -en) key must NOT be present as a dict key.
    # Anchor on the literal `"all-MiniLM-L6-v2-1.0.0":` form (with quote+colon)
    # which only matches a dict-key declaration.
    no_lang_pattern = re.compile(
        r'^\s+"all-MiniLM-L6-v2-1\.0\.0":\s*\{', re.MULTILINE
    )
    assert not no_lang_pattern.search(body), (
        "A6 regression : KNOWN_MODELS must not have a no-lang entry "
        "`all-MiniLM-L6-v2-1.0.0` (use the canonical `-en` variant)"
    )
    # The canonical -en variant MUST still exist
    canonical_pattern = re.compile(
        r'^\s+"all-MiniLM-L6-v2-en-1\.0\.0":\s*\{', re.MULTILINE
    )
    assert canonical_pattern.search(body), (
        "A6 regression : KNOWN_MODELS must have the canonical "
        "`all-MiniLM-L6-v2-en-1.0.0` entry"
    )


# ─────────────────────────────────────────────────────────────────────
# A7 (LOW) — dev key valid_until ≤ 2030 (5-year window)
# ─────────────────────────────────────────────────────────────────────


def test_m13_audit_a7_dev_key_valid_until_realistic_horizon() -> None:
    """A7 (LOW) — pre-fix the dev key `valid_until` was
    `2099-12-31T23:59:59Z` (75-year window — excessive). Post-fix
    uses a realistic 5-year horizon (2030-12-31) matching prod-grade
    key rotation conventions."""
    trust = json.loads(PUBKEYS.read_text(encoding="utf-8"))
    dev_key = next(
        (k for k in trust["keys"] if k["key_id"] == "efe-dev-bundle-2026"),
        None,
    )
    assert dev_key is not None, "Dev key entry must be present in trust store"
    valid_until = dev_key["valid_until"]
    # Refuse 2099 / any year > 2031
    year_match = re.match(r"^(\d{4})-", valid_until)
    assert year_match, f"valid_until format invalid : {valid_until!r}"
    year = int(year_match.group(1))
    assert year <= 2031, (
        f"A7 regression : dev key valid_until = {valid_until} "
        f"({year}) ; expected ≤ 2031 (5-year horizon from 2026)"
    )
    assert year >= 2027, (
        f"A7 regression : dev key valid_until = {valid_until} "
        f"({year}) ; must be at least 2027 to allow 1-year grace"
    )
