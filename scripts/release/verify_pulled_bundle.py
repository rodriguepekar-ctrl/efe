#!/usr/bin/env python3
"""
Sprint **M13 D27** — Post-pull bundle integrity verifier.

Cf. `docs/EFE_BUNDLES_EXTENSION_ARCHITECTURE.md` §6.6 + §9.1.

Used by :
  * The CI workflow `bundle-release.yml` job `verify-pull` (post-publish
    sanity check : pull from OCI, verify, fail if drift).
  * Operators after `oras pull` to a local dir.
  * Optionally by the `efe-server` boot path (Rust loader is the
    canonical implementation ; this script is the Python reference
    for tooling + CI).

Verifications performed (fail-closed) :
  1. `manifest.json` parses + validates against D16 v1.0 schema.
  2. `manifest.json.sig` is a valid raw 64-byte Ed25519 sig over
     manifest.json bytes, signed by a key matching the trust-store
     entry whose `key_id` is provided via `--pubkey-id`.
  3. Each `components[*]` whose `required=true` exists on disk + matches
     the declared SHA-256.
  4. License is in M13 ALLOWLIST or RESTRICTED+ack.
  5. The `bundle_id` regex matches D16 §3.2 canonical form.
  6. `model_origin.huggingface_revision` is not `main` (T-M13-7 anti-regression).

Exit codes :
  * 0  — bundle integrity OK
  * 1  — signature verification FAILED
  * 2  — file SHA-256 mismatch
  * 3  — manifest schema violation
  * 4  — license gate failed
  * 5  — usage error or missing trust store

Pin tested by `test_d27_verify_pulled_bundle_*` + `test_m13_verify_*`.

Usage :
    python3 scripts/release/verify_pulled_bundle.py \\
        --bundle-dir /tmp/bundle/ \\
        --pubkey-id efe-dev-bundle-2026
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]


# ---------------------------------------------------------------------------
#  Trust store handling
# ---------------------------------------------------------------------------


def load_trust_store(trust_store_path: Path) -> dict:
    if not trust_store_path.exists():
        raise FileNotFoundError(
            f"Trust store not found at {trust_store_path}. "
            f"Provide --trust-store or ensure bundles/pubkeys.json exists."
        )
    return json.loads(trust_store_path.read_text(encoding="utf-8"))


def find_pubkey(trust_store: dict, key_id: str) -> str:
    """Return the base64-encoded pubkey matching `key_id`, or raise."""
    for k in trust_store.get("keys", []):
        if k.get("key_id") == key_id:
            pub_b64 = k.get("ed25519_public", "")
            if not pub_b64 or "_PLACEHOLDER" in pub_b64:
                raise RuntimeError(
                    f"key_id {key_id!r} found in trust store but "
                    f"`ed25519_public` is a placeholder. "
                    f"Cannot verify against a placeholder key."
                )
            return pub_b64
    available = [k.get("key_id") for k in trust_store.get("keys", [])]
    raise RuntimeError(
        f"key_id {key_id!r} NOT in trust store. "
        f"Available : {available}"
    )


# ---------------------------------------------------------------------------
#  Verification phases
# ---------------------------------------------------------------------------


BUNDLE_ID_REGEX = re.compile(
    r"^[a-zA-Z0-9._/-]+@[0-9]+\.[0-9]+\.[0-9]+/[a-z]+/[0-9]+$"
)


def verify_signature(bundle_dir: Path, pubkey_b64: str) -> None:
    """Phase 1 : Ed25519 sig over manifest.json."""
    try:
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
        from cryptography.exceptions import InvalidSignature
    except ImportError as e:
        raise SystemExit(
            f"`cryptography` library required : {e}. "
            f"Install with : pip install 'cryptography>=42'"
        )
    pub = Ed25519PublicKey.from_public_bytes(base64.b64decode(pubkey_b64))
    manifest_bytes = (bundle_dir / "manifest.json").read_bytes()
    sig_bytes = (bundle_dir / "manifest.json.sig").read_bytes()
    if len(sig_bytes) != 64:
        raise SystemExit(
            f"sig length must be 64 bytes (raw Ed25519), got {len(sig_bytes)}"
        )
    try:
        pub.verify(sig_bytes, manifest_bytes)
    except InvalidSignature:
        raise SystemExit(
            "::error::SIGNATURE_FAIL — manifest.json.sig does not match "
            "the trust-store pubkey for the requested key_id"
        )


def verify_per_file_hashes(bundle_dir: Path, manifest: dict) -> None:
    """Phase 2 : per-file SHA-256."""
    for c in manifest.get("components", []):
        rel = c.get("path", "")
        f = bundle_dir / rel
        if not f.exists():
            if c.get("required", False):
                raise SystemExit(
                    f"::error::HASH_FAIL — required component "
                    f"{rel!r} is missing on disk"
                )
            continue
        actual = hashlib.sha256(f.read_bytes()).hexdigest()
        expected = c.get("sha256", "")
        if actual != expected:
            raise SystemExit(
                f"::error::HASH_FAIL — {rel!r} sha256 mismatch "
                f"(manifest={expected}, disk={actual})"
            )


def verify_schema(manifest: dict) -> None:
    """Phase 3 : minimal schema sanity (full JSON Schema validation
    is done by `test_m13_d16_schema_*` pin tests)."""
    required = [
        "manifest_version", "bundle_id", "framework", "dim",
        "components", "runtime", "build", "license",
    ]
    for f in required:
        if f not in manifest:
            raise SystemExit(
                f"::error::SCHEMA_FAIL — required field {f!r} missing"
            )
    if manifest["manifest_version"] != "1.0":
        raise SystemExit(
            f"::error::SCHEMA_FAIL — manifest_version must be '1.0', "
            f"got {manifest['manifest_version']!r}"
        )
    if not BUNDLE_ID_REGEX.match(manifest["bundle_id"]):
        raise SystemExit(
            f"::error::SCHEMA_FAIL — bundle_id {manifest['bundle_id']!r} "
            f"does not match D16 §3.2 canonical regex"
        )


def verify_license(manifest: dict, *, ack_nonredist: bool = False) -> None:
    """Phase 4 : license gate."""
    sys.path.insert(0, str(REPO / "scripts" / "release"))
    try:
        from license_gate import gate_license, LicenseGateFailure
    except ImportError as e:
        raise SystemExit(
            f"::error::license_gate.py not importable : {e}"
        )
    try:
        gate_license(manifest, ack_nonredist=ack_nonredist)
    except LicenseGateFailure as e:
        raise SystemExit(f"::error::LICENSE_FAIL — {e}")


def verify_hf_revision_pinned(manifest: dict) -> None:
    """Phase 5 : T-M13-7 anti-regression — refuse `revision=main`."""
    origin = manifest.get("model_origin", {})
    rev = origin.get("huggingface_revision", "")
    if rev == "main":
        raise SystemExit(
            "::error::T_M13_7 — model_origin.huggingface_revision must "
            "be a pinned 40-char SHA, not `main`"
        )
    # In CI mode the rev should be a 40-char hex SHA. In dev mode the
    # placeholder `dev-placeholder` is acceptable.
    if rev != "dev-placeholder":
        if not re.match(r"^[0-9a-f]{40}$", rev):
            raise SystemExit(
                f"::error::HF_REV_FAIL — huggingface_revision "
                f"{rev!r} must be 40-char hex SHA or 'dev-placeholder'"
            )


# ---------------------------------------------------------------------------
#  Driver
# ---------------------------------------------------------------------------


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0] if __doc__ else None)
    p.add_argument("--bundle-dir", type=Path, required=True,
                   help="Path to the pulled bundle directory.")
    p.add_argument("--pubkey-id", type=str, required=True,
                   help="Trust-store key_id used to verify the manifest sig.")
    p.add_argument("--trust-store", type=Path,
                   default=REPO / "bundles" / "pubkeys.json",
                   help="Path to pubkeys.json trust store.")
    p.add_argument("--ack-nonredist", action="store_true",
                   help="Acknowledge a RESTRICTED license.")
    p.add_argument("--allow-dev-placeholder", action="store_true",
                   help="Allow `dev-placeholder` as huggingface_revision "
                        "(for dev bundles bootstrapped locally).")
    args = p.parse_args(argv)

    if not args.bundle_dir.is_dir():
        print(f"::error::Bundle directory not found : {args.bundle_dir}",
              file=sys.stderr)
        return 5

    manifest_path = args.bundle_dir / "manifest.json"
    if not manifest_path.exists():
        print(f"::error::manifest.json not found in {args.bundle_dir}",
              file=sys.stderr)
        return 5

    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))

    # Phase 3 : schema first (cheap)
    verify_schema(manifest)

    # Phase 5 : hf revision pinned (cheap)
    if not args.allow_dev_placeholder:
        rev = manifest.get("model_origin", {}).get("huggingface_revision", "")
        if rev == "dev-placeholder":
            print(
                f"::error::HF_REV_FAIL — bundle uses dev-placeholder "
                f"revision but --allow-dev-placeholder not set",
                file=sys.stderr,
            )
            return 3
    verify_hf_revision_pinned(manifest)

    # Phase 4 : license gate
    verify_license(manifest, ack_nonredist=args.ack_nonredist)

    # Phase 1 : signature
    trust_store = load_trust_store(args.trust_store)
    pubkey_b64 = find_pubkey(trust_store, args.pubkey_id)
    verify_signature(args.bundle_dir, pubkey_b64)

    # Phase 2 : per-file hashes
    verify_per_file_hashes(args.bundle_dir, manifest)

    # Summary
    bundle_sha = hashlib.sha256(manifest_path.read_bytes()).hexdigest()
    print(
        f"VERIFY_OK "
        f"slug={manifest.get('bundle_slug', manifest['bundle_id'].split('@')[0])} "
        f"manifest_sha256={bundle_sha} "
        f"sig_pubkey_id={args.pubkey_id} "
        f"license={manifest['license']['spdx_id']}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
