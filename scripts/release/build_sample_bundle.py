#!/usr/bin/env python3
"""**Sprint D20 — DT-PC-1** — Sample bundle build pipeline.

Builds + signs the canonical sample embedder bundle shipped with the
SDK quickstart : `bundles/all-MiniLM-L6-v2-en-1.0.0/`. The bundle is
the FIRST real bundle of the project ; without it the quickstart
`efe-admin classify "hello"` cannot run end-to-end.

**Post-audit pass 1** — defects M8-A1, M8-A2, M8-A3 (all CRIT) fixed.
Cf. CHANGELOG audit fixes section.

**Why a build script** ?

* Reproducibility : `docs/bundles/sample.md` (DT-PC-2) pins the
  SHA-256 of every artefact. The build must be deterministic
  bit-for-bit so the auditor's `sha256sum` matches the doc.
  **Audit fix M8-A1** : `built_at` honours `SOURCE_DATE_EPOCH`
  (Reproducible Builds standard https://reproducible-builds.org/specs/source-date-epoch/)
  so two CI runs from the same git tag produce byte-identical
  manifests.
* Signing : the bundle is Ed25519-signed by the CI release key
  (offline ; never in dev machines). The script supports BOTH
  paths : (a) generate manifest + sign with the real Ed25519
  key in CI, OR (b) in dev, produce an UNSIGNED bundle marked
  `is_dev_unsigned=True` for quickstart tests.
  **Audit fix M8-A3** : the CI path now performs a REAL
  Ed25519 signature using the `cryptography` library when
  available, instead of writing a placeholder.
* CI integration : the script is invoked from
  `.github/workflows/release.yml` ; same script, two modes.

**Sub-tasks** :

  * Phase 1 — pull the upstream model
    (`sentence-transformers/all-MiniLM-L6-v2`) into a temp dir
    (no-op in dev mode).
  * Phase 2 — quantize to INT8 dynamic (PyTorch quant) — target
    ≤ 30 MiB on disk (deferred to D26, no-op stub).
  * Phase 3 — write canonical `manifest.json` (D16 schema v1) :
      `{ embedder_family, embedder_model, dim, normalize, files,
         sha256_per_file, total_size_bytes, ed25519_pubkey_id, ... }`
    **Audit fix M8-A2** : `bundle_slug` is now derived from the
    `--out` directory name (or overridable via `--slug`), so the
    manifest does NOT lie about its identity when the operator
    builds a custom-named bundle.
  * Phase 4 — sign `manifest.json` with the Ed25519 key
    (`--privkey-path`) → write `manifest.sig` next to it.
  * Phase 5 — emit a one-line summary on stdout :
      `BUNDLE_OK path=... sha256_manifest=... sig=...`

Usage :

  # Dev mode — builds an UNSIGNED bundle for local quickstart tests.
  $ ./scripts/release/build_sample_bundle.py \\
        --out bundles/all-MiniLM-L6-v2-en-1.0.0/ \\
        --dev

  # CI release mode — builds + signs deterministically.
  $ SOURCE_DATE_EPOCH=$(git log -1 --format=%ct) \\
        ./scripts/release/build_sample_bundle.py \\
        --out bundles/all-MiniLM-L6-v2-en-1.0.0/ \\
        --privkey-path /var/run/secrets/efe-release-key

Pin tested by :
  * `scripts/tests/test_d20_sample_bundle_present_and_signed`
  * `scripts/tests/test_d20_sample_bundle_doc_present`
  * `scripts/tests/test_d20_sample_bundle_reproducibility_check`
  * `scripts/tests/test_m8_audit_fix_a1_source_date_epoch_honoured`
  * `scripts/tests/test_m8_audit_fix_a2_bundle_slug_derived_from_out_dir`
  * `scripts/tests/test_m8_audit_fix_a3_real_ed25519_signature`
"""

from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import os
import sys
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any


# ---------------------------------------------------------------------------
#  Constants — pinned by tests
# ---------------------------------------------------------------------------

#: Bundle schema version. Bump = breaking change.
BUNDLE_SCHEMA_VERSION: int = 1

#: Canonical model family / id.
EMBEDDER_FAMILY: str = "sentence-transformers"
EMBEDDER_MODEL: str = "all-MiniLM-L6-v2"
EMBEDDER_DIM: int = 384

#: Target size on disk after INT8 quantization (≤ 30 MiB).
TARGET_SIZE_MIB: int = 30

#: Stable bundle slug shipped with the SDK quickstart.
DEFAULT_BUNDLE_SLUG: str = "all-MiniLM-L6-v2-en-1.0.0"

#: Stable epoch for dev-mode `built_at` when SOURCE_DATE_EPOCH is unset.
#: Pinned to 2026-01-01T00:00:00Z so dev rebuilds produce byte-identical
#: manifests by default. CI release should always export
#: SOURCE_DATE_EPOCH from the git tag commit timestamp.
DEFAULT_DEV_EPOCH: int = 1_767_225_600   # 2026-01-01T00:00:00Z


# ---------------------------------------------------------------------------
#  Manifest schema
# ---------------------------------------------------------------------------


@dataclass
class FileEntry:
    """One file inside the bundle, with its content hash + size."""
    name: str
    sha256: str
    size_bytes: int


@dataclass
class BundleManifest:
    """The canonical manifest written next to the bundle artefacts.

    Version 1 schema (`schema_version=1`). Pinned by
    :mod:`scripts.tests.test_d20_sample_bundle_*`.
    """
    schema_version: int = BUNDLE_SCHEMA_VERSION
    bundle_slug: str = DEFAULT_BUNDLE_SLUG
    embedder_family: str = EMBEDDER_FAMILY
    embedder_model: str = EMBEDDER_MODEL
    embedder_dim: int = EMBEDDER_DIM
    embedder_normalize: str = "l2"
    quantization: str = "int8-dynamic"
    files: list[FileEntry] = field(default_factory=list)
    total_size_bytes: int = 0
    ed25519_pubkey_id: str = "efe-release-2026"
    built_at: str = ""               # ISO-8601 UTC (SOURCE_DATE_EPOCH-aware)
    builder_version: str = "1.0.0"   # this script's version
    is_dev_unsigned: bool = False    # True when --dev (no sig)
    notes: str = ""

    def to_canonical_json(self) -> str:
        """Stable JSON serialisation : sorted keys + 2-space indent.

        Determinism guarantee : two manifests built from the same
        inputs (incl. `SOURCE_DATE_EPOCH`) produce byte-identical
        JSON. Pin tested by `test_m8_audit_fix_a1_*`.
        """
        return json.dumps(asdict(self), sort_keys=True, indent=2)


# ---------------------------------------------------------------------------
#  File hashing
# ---------------------------------------------------------------------------


def sha256_file(path: Path, *, chunk_size: int = 1 << 20) -> str:
    """Compute the SHA-256 of ``path`` (1 MiB chunks).

    Returns the hex digest (lowercase, 64 chars).
    """
    h = hashlib.sha256()
    with path.open("rb") as f:
        while True:
            chunk = f.read(chunk_size)
            if not chunk:
                break
            h.update(chunk)
    return h.hexdigest()


# ---------------------------------------------------------------------------
#  Audit fix M8-A1 — SOURCE_DATE_EPOCH-aware timestamp
# ---------------------------------------------------------------------------


def resolve_built_at(*, dev: bool) -> str:
    """Resolve the `built_at` timestamp deterministically.

    Resolution order (Reproducible Builds spec) :
      1. `SOURCE_DATE_EPOCH` env var → use it (CI release path).
      2. `--dev` flag : fall back to `DEFAULT_DEV_EPOCH` so two
         dev rebuilds produce byte-identical manifests.
      3. Else : `datetime.now()` (interactive / one-off path).

    Pin tested by `test_m8_audit_fix_a1_source_date_epoch_honoured`.
    """
    sde = os.environ.get("SOURCE_DATE_EPOCH")
    if sde is not None:
        try:
            ts = int(sde)
        except ValueError:
            raise RuntimeError(
                f"SOURCE_DATE_EPOCH must be an integer Unix timestamp, "
                f"got {sde!r}"
            )
        return datetime.datetime.fromtimestamp(
            ts, tz=datetime.timezone.utc
        ).isoformat()
    if dev:
        # Stable epoch for dev rebuilds → byte-identical manifests.
        return datetime.datetime.fromtimestamp(
            DEFAULT_DEV_EPOCH, tz=datetime.timezone.utc
        ).isoformat()
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


# ---------------------------------------------------------------------------
#  Audit fix M8-A2 — bundle_slug derived from --out
# ---------------------------------------------------------------------------


def resolve_bundle_slug(out_dir: Path, *, override: str | None) -> str:
    """Derive the manifest's `bundle_slug` from the output directory.

    Order :
      1. Explicit `--slug` flag → use it.
      2. Else : `out_dir.name` (e.g. `bundles/my-custom/` → `my-custom`).
      3. Empty `out_dir.name` (e.g. `--out .` or `--out ./`) →
         fall back to `DEFAULT_BUNDLE_SLUG`.

    Pin tested by `test_m8_audit_fix_a2_bundle_slug_derived_from_out_dir`.
    """
    if override:
        return override
    name = out_dir.resolve().name
    return name if name else DEFAULT_BUNDLE_SLUG


# ---------------------------------------------------------------------------
#  Phase 1 — model pull (skipped in dev / no-op when already present)
# ---------------------------------------------------------------------------


def phase_pull_model(out_dir: Path, *, dev: bool) -> None:
    """Pull the upstream model + tokenizer into ``out_dir``.

    In **dev mode** this is a NO-OP — we ship a tiny placeholder
    (32-byte file `weights.bin` filled with zeros) so the
    quickstart can exercise the load path without a 30 MiB
    download. CI release mode performs the real download +
    quantization (deferred to a follow-up D26 sprint where the
    upstream pull pipeline is wired ; today the script SKIPs and
    expects the artefacts to be pre-staged in ``out_dir``).

    The function is intentionally tolerant : if ``out_dir`` already
    contains real artefacts, they are left untouched (idempotent
    re-run path).
    """
    out_dir.mkdir(parents=True, exist_ok=True)
    weights = out_dir / "weights.bin"
    tokenizer = out_dir / "tokenizer.json"
    if dev:
        if not weights.exists():
            # 32-byte placeholder ; pin tested for size + content.
            weights.write_bytes(b"\x00" * 32)
        if not tokenizer.exists():
            # Minimal HuggingFace tokenizer JSON (deterministic).
            # Not "valid HF" semantically but the load path only
            # checks well-formed JSON in dev mode.
            tokenizer.write_text(
                json.dumps(
                    {
                        "version": "1.0",
                        "model": {"type": "WordLevel", "vocab": {}},
                        "added_tokens": [],
                        "_dev_placeholder": True,
                    },
                    sort_keys=True,
                    indent=2,
                ),
                encoding="utf-8",
            )
    else:
        # CI release mode — assume pre-staged. Validate presence.
        for f in (weights, tokenizer):
            if not f.exists():
                raise FileNotFoundError(
                    f"missing required artefact in CI release mode: {f}\n"
                    f"hint: the upstream pull pipeline is wired in D26 ; "
                    f"until then, stage `weights.bin` + `tokenizer.json` "
                    f"manually before running this script."
                )


# ---------------------------------------------------------------------------
#  Phase 2 — quantize (placeholder ; real impl deferred to D26)
# ---------------------------------------------------------------------------


def phase_quantize(out_dir: Path, *, dev: bool) -> None:
    """INT8 dynamic quantization of the FP32 weights.

    Stub — real quantization is deferred to sprint D26 (where the
    upstream pull pipeline is wired). The dev-mode placeholder
    ``weights.bin`` is treated as already-quantized for round-trip
    test purposes.
    """
    weights = out_dir / "weights.bin"
    size = weights.stat().st_size
    target = TARGET_SIZE_MIB * (1 << 20)
    if not dev and size > target:
        raise RuntimeError(
            f"quantized weights {size:,} bytes > target {target:,} bytes "
            f"({TARGET_SIZE_MIB} MiB). Re-run with stricter quant."
        )


# ---------------------------------------------------------------------------
#  Phase 3 — write manifest
# ---------------------------------------------------------------------------


def phase_write_manifest(
    out_dir: Path, *, dev: bool, built_at: str, bundle_slug: str,
) -> tuple[BundleManifest, Path]:
    """Compute hashes + write `manifest.json`.

    Returns the in-memory ``BundleManifest`` and the path of the
    written file. The manifest is canonical-JSON (sorted keys) so
    re-runs produce byte-identical output (when SOURCE_DATE_EPOCH
    is held constant).
    """
    files: list[FileEntry] = []
    total = 0
    for name in ("weights.bin", "tokenizer.json"):
        p = out_dir / name
        sz = p.stat().st_size
        files.append(FileEntry(name=name, sha256=sha256_file(p), size_bytes=sz))
        total += sz
    manifest = BundleManifest(
        bundle_slug=bundle_slug,                  # Audit fix M8-A2.
        files=files,
        total_size_bytes=total,
        built_at=built_at,                        # Audit fix M8-A1.
        is_dev_unsigned=dev,
        notes=(
            "DEV MODE — placeholder artefacts. Re-build with the "
            "real upstream model in CI release before publishing."
        ) if dev else "",
    )
    manifest_path = out_dir / "manifest.json"
    manifest_path.write_text(manifest.to_canonical_json(), encoding="utf-8")
    return manifest, manifest_path


# ---------------------------------------------------------------------------
#  Phase 4 — sign (Audit fix M8-A3 : real Ed25519 signature)
# ---------------------------------------------------------------------------


class SigningUnavailable(RuntimeError):
    """Raised when CI release mode requested signing but the
    `cryptography` library isn't installed. Surfaced as exit 5."""


def _load_ed25519_privkey(path: Path):
    """Load an Ed25519 private key from a raw 32-byte file or a
    PEM/DER PKCS#8 file. Returns the ``Ed25519PrivateKey`` instance.

    The CI release path uses 32-byte raw seeds (HSM-exported) ;
    operator manual path may use PEM.
    """
    try:
        from cryptography.hazmat.primitives.asymmetric.ed25519 import (
            Ed25519PrivateKey,
        )
        from cryptography.hazmat.primitives import serialization
    except ImportError as e:
        raise SigningUnavailable(
            f"the `cryptography` library is required for CI signing but "
            f"is not installed: {e}. "
            f"hint: `pip install cryptography>=42` or use --dev."
        ) from e
    raw = path.read_bytes()
    # 32-byte raw seed (canonical CI format).
    if len(raw) == 32:
        return Ed25519PrivateKey.from_private_bytes(raw)
    # PEM / DER PKCS#8.
    try:
        return serialization.load_pem_private_key(raw, password=None)
    except ValueError:
        return serialization.load_der_private_key(raw, password=None)


def phase_sign(
    manifest_path: Path, *, privkey_path: Path | None, dev: bool,
) -> Path | None:
    """Ed25519-sign the canonical manifest JSON.

    **Audit fix M8-A3** : produces a REAL Ed25519 signature in CI
    release mode (via the `cryptography` library) instead of a
    `<TO_BE_FILLED_BY_CI_RELEASE>` placeholder.

    Returns the path of the `manifest.sig` file, or ``None`` when
    `--dev` (no signing — the load path tolerates `is_dev_unsigned=True`
    bundles in non-prod environments).

    Raises :
      * `SigningUnavailable` — `cryptography` not installed in CI mode.
      * `FileNotFoundError` — `privkey_path` doesn't exist.
    """
    sig_path = manifest_path.parent / "manifest.sig"
    if dev or privkey_path is None:
        # Dev / no-key path : no sig file. The load path detects the
        # absence + reads `is_dev_unsigned=True` from the manifest.
        if sig_path.exists():
            sig_path.unlink()
        return None
    # CI release path : produce the real Ed25519 signature.
    if not privkey_path.exists():
        raise FileNotFoundError(
            f"--privkey-path does not exist : {privkey_path}"
        )
    privkey = _load_ed25519_privkey(privkey_path)
    canonical = manifest_path.read_bytes()
    raw_sig = privkey.sign(canonical)
    import base64 as _b64
    sig_b64 = _b64.b64encode(raw_sig).decode("ascii")
    sig_path.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "algorithm": "ed25519",
                "pubkey_id": "efe-release-2026",
                "signature_base64": sig_b64,
                "manifest_sha256": sha256_file(manifest_path),
            },
            sort_keys=True,
            indent=2,
        ),
        encoding="utf-8",
    )
    return sig_path


# ---------------------------------------------------------------------------
#  Driver
# ---------------------------------------------------------------------------


def build(
    out_dir: Path,
    *,
    dev: bool,
    privkey_path: Path | None,
    slug_override: str | None,
) -> int:
    """Run all phases. Returns shell exit code.

    * 0 : OK
    * 2 : usage error (caught by argparse, never returned here)
    * 3 : missing artefact in CI mode
    * 4 : quant size budget exceeded
    * 5 : signing unavailable (cryptography lib missing)
    """
    out_dir.mkdir(parents=True, exist_ok=True)

    bundle_slug = resolve_bundle_slug(out_dir, override=slug_override)
    built_at = resolve_built_at(dev=dev)

    try:
        phase_pull_model(out_dir, dev=dev)
        phase_quantize(out_dir, dev=dev)
        manifest, manifest_path = phase_write_manifest(
            out_dir,
            dev=dev,
            built_at=built_at,
            bundle_slug=bundle_slug,
        )
        sig_path = phase_sign(
            manifest_path, privkey_path=privkey_path, dev=dev,
        )
    except FileNotFoundError as e:
        print(f"BUNDLE_FAIL missing_artefact: {e}", file=sys.stderr)
        return 3
    except RuntimeError as e:
        if isinstance(e, SigningUnavailable):
            print(f"BUNDLE_FAIL signing_unavailable: {e}", file=sys.stderr)
            return 5
        print(f"BUNDLE_FAIL quant_budget: {e}", file=sys.stderr)
        return 4

    sha = sha256_file(manifest_path)
    print(
        f"BUNDLE_OK "
        f"path={out_dir} "
        f"slug={bundle_slug} "
        f"sha256_manifest={sha} "
        f"sig={sig_path or '<unsigned-dev>'} "
        f"total_size={manifest.total_size_bytes}"
    )
    return 0


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(
        description=__doc__.split("\n")[0] if __doc__ else None,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    p.add_argument(
        "--out", type=Path, required=True,
        help="Output directory (typically `bundles/<slug>/`).",
    )
    p.add_argument(
        "--dev", action="store_true",
        help="Build UNSIGNED dev bundle with 32-byte placeholders.",
    )
    p.add_argument(
        "--privkey-path", type=Path, default=None,
        help=(
            "Ed25519 priv key (CI release only ; raw 32-byte or PEM/DER "
            "PKCS#8). Required for signed output ; absent = unsigned bundle."
        ),
    )
    p.add_argument(
        "--slug", default=None,
        help=(
            "Override the manifest `bundle_slug` (default: derived from "
            "`--out` directory name). Useful when staging multiple builds."
        ),
    )
    args = p.parse_args(argv)
    return build(
        args.out,
        dev=args.dev,
        privkey_path=args.privkey_path,
        slug_override=args.slug,
    )


if __name__ == "__main__":
    sys.exit(main())
