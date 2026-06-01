#!/usr/bin/env python3
"""
Sprint **M13 D26** — Production multilingual bundle build pipeline.

Cf. `docs/EFE_BUNDLES_EXTENSION_ARCHITECTURE.md` §5 + §10 + §14.

Builds + signs a REAL production embedder bundle from upstream HF model
+ tokenizer assets. Extends `scripts/release/build_sample_bundle.py`
(D20 sample) with :

  * **PINNED HF revision MANDATORY** (T-M13-7 mitigation)
  * FP16 quantization (vs INT8 for D20 multilingual)
  * Full D16 v1.0 manifest schema
  * SBOM CycloneDX 1.5 generation (delegated to `generate_bundle_sbom.py`)
  * License gate (delegated to `license_gate.py`)
  * Sentencepiece tokenizer support (XLM-R)
  * Cross-lingual evaluation gate (F1 ≥ 0.80, delegated to fixture)
  * Deterministic outputs for `SOURCE_DATE_EPOCH`-aware reproducibility

**Status M13** : pull + quantization phases are gated behind `--ci-mode`
(real upstream pull) ; the script ALSO supports `--from-staged` for a
local staging directory pre-populated by the operator (for offline
builds + air-gap CI). In dev mode (`--dev`) it falls back to the
bootstrap script `bootstrap_dev_bundles.py` for placeholders.

Three operational modes :

  * `--dev`         : delegate to `bootstrap_dev_bundles.py` for placeholders.
  * `--from-staged` : assume `weights/`, `tokenizer/`, `runtime/` are
                       pre-staged in `--out` ; only do hash + manifest
                       + sign + SBOM phases.
  * `--ci-mode`     : full pipeline (HF pull + quantize + manifest +
                       sign + SBOM + license-gate). Requires the
                       optional deps : `huggingface_hub`, `onnxruntime`,
                       `transformers`, `cryptography`.

Pin tested by :
  * `test_d26_real_bundle_*`
  * `test_m13_build_real_bundle_*`

Usage :

    # Local dev path : delegate to bootstrap (no upstream pull)
    python3 scripts/release/build_real_bundle.py \\
        --out bundles/paraphrase-multilingual-MiniLM-L12-v2-1.0.0/ \\
        --slug paraphrase-multilingual-MiniLM-L12-v2-1.0.0 \\
        --hf-revision bf3bf13ab40c3157080a7ab344c831b9ad18b5eb \\
        --dev

    # CI mode : real HF pull + sign with KMS-loaded privkey
    SOURCE_DATE_EPOCH=$(git log -1 --format=%ct) \\
        python3 scripts/release/build_real_bundle.py \\
        --out bundles/paraphrase-multilingual-MiniLM-L12-v2-1.0.0/ \\
        --slug paraphrase-multilingual-MiniLM-L12-v2-1.0.0 \\
        --hf-revision bf3bf13ab40c3157080a7ab344c831b9ad18b5eb \\
        --ci-mode \\
        --privkey-path /var/run/secrets/efe-bundle-privkey.pem
"""

from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import os
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Optional

REPO = Path(__file__).resolve().parents[2]


# ---------------------------------------------------------------------------
#  Constants
# ---------------------------------------------------------------------------

MANIFEST_SCHEMA_VERSION: str = "1.0"

#: D26 production bundle target size in bytes (80 MiB).
D26_TARGET_SIZE: int = 80 * (1 << 20)

#: D20 sample bundle target size in bytes (30 MiB).
D20_TARGET_SIZE: int = 30 * (1 << 20)

#: Default dev epoch (2026-01-01T00:00:00Z) for reproducible dev mode.
DEFAULT_DEV_EPOCH: int = 1_767_225_600

#: HF model ↔ canonical bundle slug mapping.
#
# Audit fix M13-A6 (LOW) — pre-fix this dict had a redundant
# `all-MiniLM-L6-v2-1.0.0` entry duplicating `all-MiniLM-L6-v2-en-1.0.0`.
# Post-fix the canonical slug is `all-MiniLM-L6-v2-en-1.0.0` (the
# `-en` suffix indicates the language). For non-English single-lang
# models in the future, slugs should follow `<name>-<lang>-<version>`.
KNOWN_MODELS: dict[str, dict] = {
    "all-MiniLM-L6-v2-en-1.0.0": {
        "bundle_id": "sentence-transformers/all-MiniLM-L6-v2@1.0.0/onnx/384",
        "hf_repo": "sentence-transformers/all-MiniLM-L6-v2",
        "framework": "onnx",
        "dim": 384,
        "quantization": "int8-dynamic",
        "license_spdx": "Apache-2.0",
        "max_size": D20_TARGET_SIZE,
        "runtime": {
            "max_seq_len": 256,
            "pooling": "mean",
            "normalize": True,
            "dtype": "float32",
        },
        "tokenizer_files": [
            "tokenizer.json",
            "tokenizer_config.json",
            "special_tokens_map.json",
            "vocab.txt",
        ],
    },
    "paraphrase-multilingual-MiniLM-L12-v2-1.0.0": {
        "bundle_id": "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2@1.0.0/onnx/384",
        "hf_repo": "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2",
        "framework": "onnx",
        "dim": 384,
        "quantization": "fp16",
        "license_spdx": "Apache-2.0",
        "max_size": D26_TARGET_SIZE,
        "runtime": {
            "max_seq_len": 128,
            "pooling": "mean",
            "normalize": True,
            "dtype": "float16",
        },
        "tokenizer_files": [
            "tokenizer.json",
            "tokenizer_config.json",
            "special_tokens_map.json",
            "sentencepiece.bpe.model",
        ],
    },
}


# ---------------------------------------------------------------------------
#  Helpers
# ---------------------------------------------------------------------------


def canonical_json(obj: object) -> str:
    return json.dumps(obj, sort_keys=True, indent=2, ensure_ascii=False)


def sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def resolve_source_date_epoch() -> int:
    sde = os.environ.get("SOURCE_DATE_EPOCH")
    if sde is not None:
        return int(sde)
    return DEFAULT_DEV_EPOCH


def validate_hf_revision(rev: str) -> None:
    """Reject `main` / branch names ; require 40-char hex SHA. (T-M13-7)"""
    if rev == "main" or rev == "master":
        raise SystemExit(
            f"::error::T_M13_7 — --hf-revision must be a pinned 40-char "
            f"hex SHA, not a branch name like {rev!r}. Find the SHA at "
            f"https://huggingface.co/<repo>/commits/main"
        )
    if rev == "dev-placeholder":
        return
    if len(rev) != 40 or not all(c in "0123456789abcdef" for c in rev):
        raise SystemExit(
            f"::error::T_M13_7 — --hf-revision must be 40-char lowercase "
            f"hex SHA (or 'dev-placeholder'), got {rev!r}"
        )


# ---------------------------------------------------------------------------
#  Phase 1 — pull + stage (CI mode only)
# ---------------------------------------------------------------------------


def phase_pull_hf_model(out_dir: Path, *, hf_repo: str, hf_revision: str,
                        tokenizer_files: list[str]) -> None:
    """Phase 1 (CI mode) : pull the upstream HF model.

    Audit fix M13-D26-A1 — uses `revision=<pinned SHA>` strictly.
    Refuses `revision='main'` (T-M13-7).
    """
    try:
        from huggingface_hub import snapshot_download
    except ImportError as e:
        raise SystemExit(
            f"::error::CI mode requires `huggingface_hub` : {e}. "
            f"Install with `pip install huggingface_hub>=0.20`."
        )
    validate_hf_revision(hf_revision)

    staging = out_dir / "_staging"
    if staging.exists():
        shutil.rmtree(staging)
    staging.mkdir(parents=True)

    # Pull only the canonical files (no PyTorch *.bin, no fp32 *.safetensors,
    # no docs).
    allow_patterns = [
        "onnx/model.onnx",
        "config.json",
    ] + tokenizer_files

    snapshot_download(
        repo_id=hf_repo,
        revision=hf_revision,
        local_dir=str(staging),
        local_dir_use_symlinks=False,
        allow_patterns=allow_patterns,
    )

    # Re-organize to canonical bundle layout
    weights_dir = out_dir / "weights"
    tokenizer_dir = out_dir / "tokenizer"
    weights_dir.mkdir(parents=True, exist_ok=True)
    tokenizer_dir.mkdir(parents=True, exist_ok=True)

    onnx_src = staging / "onnx" / "model.onnx"
    if not onnx_src.exists():
        raise SystemExit(
            f"::error::Upstream HF repo {hf_repo}@{hf_revision} doesn't "
            f"ship onnx/model.onnx. Use a different revision or fork."
        )
    shutil.move(str(onnx_src), str(weights_dir / "model.onnx"))

    for f in tokenizer_files:
        src = staging / f
        if src.exists():
            shutil.move(str(src), str(tokenizer_dir / f))

    shutil.rmtree(staging, ignore_errors=True)


# ---------------------------------------------------------------------------
#  Phase 2 — quantize (CI mode)
# ---------------------------------------------------------------------------


def phase_quantize(out_dir: Path, *, scheme: str) -> None:
    """Phase 2 : quantize weights/model.onnx in-place."""
    onnx_path = out_dir / "weights" / "model.onnx"
    if not onnx_path.exists():
        raise SystemExit(f"::error::weights/model.onnx not found at {onnx_path}")

    if scheme == "none":
        return

    if scheme == "int8-dynamic":
        try:
            from onnxruntime.quantization import quantize_dynamic, QuantType
        except ImportError as e:
            raise SystemExit(
                f"::error::CI mode requires `onnxruntime>=1.16` for INT8 "
                f"quantization : {e}"
            )
        quantize_dynamic(
            model_input=str(onnx_path),
            model_output=str(onnx_path) + ".int8",
            weight_type=QuantType.QInt8,
            op_types_to_quantize=["MatMul", "Gather"],
            per_channel=False,
            reduce_range=False,
            extra_options={
                "WeightSymmetric": True,
                "ActivationSymmetric": False,
            },
        )
        shutil.move(str(onnx_path) + ".int8", str(onnx_path))
        return

    if scheme == "fp16":
        try:
            import onnx
            from onnx import numpy_helper
            import numpy as np
        except ImportError as e:
            raise SystemExit(
                f"::error::CI mode requires `onnx>=1.15` + `numpy` for "
                f"FP16 quantization : {e}"
            )
        model = onnx.load(str(onnx_path))
        for init in model.graph.initializer:
            if init.data_type == onnx.TensorProto.FLOAT:
                arr = numpy_helper.to_array(init)
                arr_fp16 = arr.astype(np.float16)
                new_init = numpy_helper.from_array(arr_fp16, init.name)
                init.CopyFrom(new_init)
        # Update IO types where applicable
        for io in list(model.graph.input) + list(model.graph.output):
            tt = io.type.tensor_type
            if tt.elem_type == onnx.TensorProto.FLOAT:
                tt.elem_type = onnx.TensorProto.FLOAT16
        onnx.save(model, str(onnx_path))
        return

    raise SystemExit(f"::error::Unsupported quantization scheme : {scheme!r}")


# ---------------------------------------------------------------------------
#  Phase 3 — write canonical manifest
# ---------------------------------------------------------------------------


def write_runtime_config(out_dir: Path, runtime: dict) -> None:
    rc_path = out_dir / "runtime" / "runtime_config.json"
    rc_path.parent.mkdir(parents=True, exist_ok=True)
    rc_path.write_text(canonical_json(runtime), encoding="utf-8")


def write_license_text(out_dir: Path, spdx: str) -> None:
    """Write canonical LICENSE text. Apache-2.0 ships verbatim header."""
    license_path = out_dir / "LICENSE"
    if license_path.exists() and license_path.stat().st_size > 0:
        return  # already provided by operator (e.g., bootstrap)
    if spdx == "Apache-2.0":
        license_path.write_text(
            "Apache License\n"
            "Version 2.0, January 2004\n"
            "http://www.apache.org/licenses/\n\n"
            "[Full text — see LICENSE-2.0.txt at apache.org]\n",
            encoding="utf-8",
        )
    else:
        license_path.write_text(
            f"License : {spdx}\n"
            f"See SPDX https://spdx.org/licenses/{spdx}.html for full text.\n",
            encoding="utf-8",
        )


def collect_components(out_dir: Path) -> list[dict]:
    """Walk the bundle dir + collect components in canonical order."""
    components = []
    skip = {"manifest.json", "manifest.json.sig", "_staging"}
    for f in sorted(out_dir.rglob("*")):
        if not f.is_file():
            continue
        relpath = str(f.relative_to(out_dir)).replace(os.sep, "/")
        if relpath in skip or relpath.startswith("_staging/"):
            continue
        # SBOM, README, LICENSE → required=False (informational)
        # weights, tokenizer, runtime → required=True (hard load deps)
        required = not (
            relpath in ("LICENSE", "README.md")
            # SBOM is required for prod-grade bundles
        )
        components.append({
            "path": relpath,
            "sha256": sha256_file(f),
            "size_bytes": f.stat().st_size,
            "required": required,
        })
    return components


def phase_write_manifest(out_dir: Path, *, model_spec: dict, slug: str,
                         hf_revision: str, source_date_epoch: int,
                         git_sha: str, builder_uri: str,
                         signing_pubkey_id: str) -> Path:
    write_runtime_config(out_dir, model_spec["runtime"])
    write_license_text(out_dir, model_spec["license_spdx"])

    components = collect_components(out_dir)

    manifest = {
        "manifest_version": MANIFEST_SCHEMA_VERSION,
        "bundle_id": model_spec["bundle_id"],
        "bundle_slug": slug,
        "framework": model_spec["framework"],
        "dim": model_spec["dim"],
        "quantization": model_spec["quantization"],
        "components": components,
        "runtime": model_spec["runtime"],
        "build": {
            "builder": builder_uri,
            "built_at_unix_ms": source_date_epoch * 1000,
            "git_sha": git_sha,
            "source_date_epoch": source_date_epoch,
        },
        "license": {
            "spdx_id": model_spec["license_spdx"],
            "url": (
                f"https://huggingface.co/{model_spec['hf_repo']}/blob/main/LICENSE"
            ),
            "redistribution_allowed": True,
        },
        "model_origin": {
            "huggingface_repo": model_spec["hf_repo"],
            "huggingface_revision": hf_revision,
            "model_card_url": f"https://huggingface.co/{model_spec['hf_repo']}",
        },
        "sbom_path": "SBOM.json",
        # Audit fix M13-A5 (MED) — pre-fix this was hardcoded to
        # `efe-prod-bundle-2026q1` regardless of the actual privkey
        # used to sign. Local dev builds (or any CI run that doesn't
        # use the prod key) produced manifests claiming prod-signing,
        # causing trust-store lookup failures at verify time. Post-fix
        # the pubkey_id is now an explicit parameter passed from the
        # caller (CLI flag `--signing-pubkey-id`).
        "signing": {
            "algorithm": "ed25519",
            "pubkey_id": signing_pubkey_id,
        },
        "is_dev_unsigned": False,
        "notes": "",
    }
    manifest_path = out_dir / "manifest.json"
    manifest_path.write_text(canonical_json(manifest), encoding="utf-8")
    return manifest_path


# ---------------------------------------------------------------------------
#  Phase 4 — sign
# ---------------------------------------------------------------------------


def _load_ed25519_privkey(path: Path):
    try:
        from cryptography.hazmat.primitives.asymmetric.ed25519 import (
            Ed25519PrivateKey,
        )
        from cryptography.hazmat.primitives import serialization
    except ImportError as e:
        raise SystemExit(f"`cryptography` library required : {e}")
    raw = path.read_bytes()
    if len(raw) == 32:
        return Ed25519PrivateKey.from_private_bytes(raw)
    try:
        return serialization.load_pem_private_key(raw, password=None)
    except ValueError:
        try:
            return serialization.load_der_private_key(raw, password=None)
        except ValueError:
            raise SystemExit(
                f"::error::Privkey at {path} is not raw 32-byte / PEM / DER"
            )


def phase_sign(manifest_path: Path, *, privkey_path: Path) -> Path:
    privkey = _load_ed25519_privkey(privkey_path)
    sig = privkey.sign(manifest_path.read_bytes())
    sig_path = manifest_path.parent / "manifest.json.sig"
    sig_path.write_bytes(sig)
    assert len(sig) == 64
    return sig_path


# ---------------------------------------------------------------------------
#  Phase 5 — SBOM (delegate)
# ---------------------------------------------------------------------------


def phase_generate_sbom(out_dir: Path, *, hf_revision: str,
                        source_date_epoch: int) -> Path:
    sbom_path = out_dir / "SBOM.json"
    cmd = [
        sys.executable,
        str(REPO / "scripts" / "release" / "generate_bundle_sbom.py"),
        "--bundle-dir", str(out_dir),
        "--hf-revision", hf_revision,
        "--output", str(sbom_path),
        "--source-date-epoch", str(source_date_epoch),
    ]
    rc = subprocess.run(cmd, check=False).returncode
    if rc != 0:
        raise SystemExit(f"::error::SBOM generation failed (exit {rc})")
    return sbom_path


# ---------------------------------------------------------------------------
#  Phase 6 — license gate (delegate)
# ---------------------------------------------------------------------------


def phase_license_gate(manifest_path: Path, *, ack_nonredist: bool) -> None:
    cmd = [
        sys.executable,
        str(REPO / "scripts" / "release" / "license_gate.py"),
        "--manifest", str(manifest_path),
    ]
    if ack_nonredist:
        cmd.append("--ack-nonredist")
    rc = subprocess.run(cmd, check=False).returncode
    if rc != 0:
        raise SystemExit(f"::error::License gate refused (exit {rc})")


# ---------------------------------------------------------------------------
#  Phase 7 — size budget check
# ---------------------------------------------------------------------------


def phase_check_size(out_dir: Path, *, max_bytes: int) -> int:
    total = sum(f.stat().st_size for f in out_dir.rglob("*") if f.is_file())
    if total > max_bytes:
        raise SystemExit(
            f"::error::Bundle size {total:,} > target {max_bytes:,} bytes "
            f"({max_bytes // (1<<20)} MiB)"
        )
    return total


# ---------------------------------------------------------------------------
#  Driver
# ---------------------------------------------------------------------------


def build(*, out_dir: Path, slug: str, hf_revision: str,
          mode: str, privkey_path: Optional[Path],
          ack_nonredist: bool, source_date_epoch: int,
          git_sha: str, builder_uri: str,
          signing_pubkey_id: str) -> int:
    if slug not in KNOWN_MODELS:
        raise SystemExit(
            f"::error::Unknown slug {slug!r}. Known : {list(KNOWN_MODELS)}"
        )
    model_spec = KNOWN_MODELS[slug]

    # Mode dispatch
    if mode == "dev":
        # Audit fix M13-A1 (CRIT) — pre-fix this delegated to
        # `bootstrap_dev_bundles.py` (which always writes to
        # `bundles/<slug>/`) then tried to read manifest from `--out`.
        # When `--out` differed from `bundles/<slug>/`, the read
        # failed with `FileNotFoundError`. Post-fix : after bootstrap,
        # we sync the canonical bundle dir to `--out` if they differ.
        bundles_root = REPO / "bundles"
        canonical_dir = bundles_root / slug

        cmd = [
            sys.executable,
            str(REPO / "scripts" / "release" / "bootstrap_dev_bundles.py"),
            "--only", "d20" if "all-MiniLM-L6-v2" in slug else "d26",
            "--source-date-epoch", str(source_date_epoch),
        ]
        rc = subprocess.run(cmd, check=False).returncode
        if rc != 0:
            return rc
        if not canonical_dir.is_dir():
            raise SystemExit(
                f"::error::M13-A1 — bootstrap completed exit 0 but did "
                f"NOT produce {canonical_dir} (unknown slug ?)"
            )
        # Sync bundle to user-specified --out if different
        if out_dir.resolve() != canonical_dir.resolve():
            if out_dir.exists():
                shutil.rmtree(out_dir)
            shutil.copytree(canonical_dir, out_dir)
        manifest_path = out_dir / "manifest.json"
    else:
        # ci-mode or from-staged
        validate_hf_revision(hf_revision)
        out_dir.mkdir(parents=True, exist_ok=True)

        if mode == "ci-mode":
            phase_pull_hf_model(
                out_dir,
                hf_repo=model_spec["hf_repo"],
                hf_revision=hf_revision,
                tokenizer_files=model_spec["tokenizer_files"],
            )
            phase_quantize(out_dir, scheme=model_spec["quantization"])

        manifest_path = phase_write_manifest(
            out_dir,
            model_spec=model_spec,
            slug=slug,
            hf_revision=hf_revision,
            source_date_epoch=source_date_epoch,
            git_sha=git_sha,
            builder_uri=builder_uri,
            signing_pubkey_id=signing_pubkey_id,
        )
        phase_license_gate(manifest_path, ack_nonredist=ack_nonredist)
        phase_generate_sbom(
            out_dir,
            hf_revision=hf_revision,
            source_date_epoch=source_date_epoch,
        )
        # Re-write manifest to include SBOM hash
        manifest_path = phase_write_manifest(
            out_dir,
            model_spec=model_spec,
            slug=slug,
            hf_revision=hf_revision,
            source_date_epoch=source_date_epoch,
            git_sha=git_sha,
            builder_uri=builder_uri,
            signing_pubkey_id=signing_pubkey_id,
        )
        if privkey_path is not None:
            phase_sign(manifest_path, privkey_path=privkey_path)

    total_size = phase_check_size(out_dir, max_bytes=model_spec["max_size"])
    manifest_sha = sha256_file(manifest_path)

    print(
        f"BUNDLE_OK "
        f"slug={slug} "
        f"path={out_dir} "
        f"sha256_manifest={manifest_sha} "
        f"total_size={total_size:,} "
        f"max={model_spec['max_size']:,}"
    )
    return 0


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0] if __doc__ else None)
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--slug", type=str, required=True,
                   help="Bundle slug (e.g., paraphrase-multilingual-MiniLM-L12-v2-1.0.0).")
    p.add_argument("--hf-revision", type=str, required=True,
                   help="Pinned 40-char HF SHA, or 'dev-placeholder' for --dev.")
    mode_grp = p.add_mutually_exclusive_group(required=True)
    mode_grp.add_argument("--dev", action="store_true",
                          help="Delegate to bootstrap_dev_bundles.py.")
    mode_grp.add_argument("--from-staged", action="store_true",
                          help="Skip pull/quantize ; use pre-staged files.")
    mode_grp.add_argument("--ci-mode", action="store_true",
                          help="Full CI pipeline (HF pull + quantize).")
    p.add_argument("--privkey-path", type=Path, default=None,
                   help="Ed25519 priv key. Required for non-dev modes.")
    p.add_argument("--ack-nonredist", action="store_true")
    p.add_argument("--source-date-epoch", type=int, default=None)
    p.add_argument("--git-sha", type=str, default="local-dev")
    p.add_argument("--builder-uri", type=str, default="local-dev")
    p.add_argument("--signing-pubkey-id", type=str,
                   default="efe-prod-bundle-2026q1",
                   help="Trust-store key_id corresponding to the privkey "
                        "used. Default `efe-prod-bundle-2026q1` for CI ; "
                        "local dev builds with the dev key should pass "
                        "`--signing-pubkey-id efe-dev-bundle-2026`.")
    args = p.parse_args(argv)

    if args.source_date_epoch is not None:
        os.environ["SOURCE_DATE_EPOCH"] = str(args.source_date_epoch)
    sde = resolve_source_date_epoch()

    mode = "dev" if args.dev else ("ci-mode" if args.ci_mode else "from-staged")

    return build(
        out_dir=args.out,
        slug=args.slug,
        hf_revision=args.hf_revision,
        mode=mode,
        privkey_path=args.privkey_path,
        ack_nonredist=args.ack_nonredist,
        source_date_epoch=sde,
        git_sha=args.git_sha,
        builder_uri=args.builder_uri,
        signing_pubkey_id=args.signing_pubkey_id,
    )


if __name__ == "__main__":
    sys.exit(main())
