#!/usr/bin/env python3
"""
Sprint **M13 D27** — CycloneDX 1.5 SBOM (ML-BOM) generator for EFE bundles.

Cf. `docs/EFE_BUNDLES_EXTENSION_ARCHITECTURE.md` §11.

Generates a `SBOM.json` next to a bundle's `manifest.json`, declaring :
  * The ML model as a `machine-learning-model` component.
  * The tokenizer as a `data` component.
  * Build-time deps (onnxruntime, transformers, etc.) as `library` components.
  * Provenance attestation linking SBOM serial → manifest SHA-256.

The SBOM is **deterministic** when SOURCE_DATE_EPOCH is fixed :
  * `metadata.timestamp` derives from SOURCE_DATE_EPOCH.
  * Components are ordered by `bom-ref` (sorted).
  * Properties within each component are ordered by `name`.
  * JSON is canonical (sort_keys=True, indent=2).

Pin tested by `test_d26_sbom_*` + `test_m13_sbom_*`.

Usage :
    python3 scripts/release/generate_bundle_sbom.py \\
        --bundle-dir bundles/<slug>/ \\
        --hf-revision <40-char SHA> \\
        --output bundles/<slug>/SBOM.json
"""

from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import os
import sys
import uuid
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]


# ---------------------------------------------------------------------------
#  Build-time deps (pinned versions for reproducibility)
# ---------------------------------------------------------------------------

BUILD_DEPS_DEFAULT = [
    ("onnxruntime", "1.16.3", "MIT"),
    ("onnx", "1.15.0", "Apache-2.0"),
    ("transformers", "4.35.2", "Apache-2.0"),
    ("sentence-transformers", "2.2.2", "Apache-2.0"),
    ("numpy", "1.26.0", "BSD-3-Clause"),
    ("torch", "2.1.0", "BSD-3-Clause"),
    ("cryptography", "42.0.0", "Apache-2.0"),
    ("huggingface_hub", "0.20.0", "Apache-2.0"),
]


# ---------------------------------------------------------------------------
#  Helpers
# ---------------------------------------------------------------------------


def canonical_json(obj: object) -> str:
    return json.dumps(obj, sort_keys=True, indent=2, ensure_ascii=False)


def resolve_source_date_epoch() -> int:
    sde = os.environ.get("SOURCE_DATE_EPOCH")
    if sde is not None:
        return int(sde)
    return 1_767_225_600  # 2026-01-01T00:00:00Z fallback


def iso_timestamp(unix_epoch: int) -> str:
    return (
        datetime.datetime.fromtimestamp(unix_epoch, tz=datetime.timezone.utc)
        .isoformat()
        .replace("+00:00", "Z")
    )


def deterministic_serial_number(bundle_slug: str, manifest_sha: str) -> str:
    """Deterministic SBOM serial UUID derived from bundle identity + manifest sha.

    Using SHA-256 truncated to 16 bytes, formatted as UUIDv5-style string.
    """
    seed = hashlib.sha256(
        f"efe-sbom-{bundle_slug}-{manifest_sha}".encode("utf-8")
    ).digest()[:16]
    # Format as UUID
    parts = [
        seed[:4].hex(),
        seed[4:6].hex(),
        seed[6:8].hex(),
        seed[8:10].hex(),
        seed[10:16].hex(),
    ]
    return f"urn:uuid:{'-'.join(parts)}"


# ---------------------------------------------------------------------------
#  Component renderers
# ---------------------------------------------------------------------------


def render_ml_model_component(manifest: dict, hf_revision: str) -> dict:
    bundle_id = manifest["bundle_id"]
    # Parse {family}/{name}@{version}/{framework}/{dim}
    family_name, rest = bundle_id.split("@", 1)
    family, name = family_name.rsplit("/", 1)
    version, framework, dim = rest.split("/")

    quantization = manifest.get("quantization", "none")
    spdx = manifest.get("license", {}).get("spdx_id", "Apache-2.0")

    properties = sorted([
        {"name": "ml.model.architecture", "value": _arch_from_name(name)},
        {"name": "ml.model.framework", "value": framework},
        {"name": "ml.model.quantization", "value": quantization},
        {"name": "ml.model.dim", "value": dim},
        {"name": "efe.bundle.id", "value": bundle_id},
        {"name": "efe.bundle.hf_revision", "value": hf_revision},
    ], key=lambda p: p["name"])

    return {
        "type": "machine-learning-model",
        "bom-ref": f"{family}/{name}@{version}",
        "name": name,
        "version": version,
        "publisher": _publisher_from_family(family),
        "licenses": [{"license": {"id": spdx}}],
        "purl": f"pkg:huggingface/{family}/{name}@{hf_revision}",
        "description": _description_from_name(name),
        "properties": properties,
    }


def render_tokenizer_component(manifest: dict) -> dict:
    bundle_id = manifest["bundle_id"]
    family_name, rest = bundle_id.split("@", 1)
    family, name = family_name.rsplit("/", 1)
    version = rest.split("/")[0]
    spdx = manifest.get("license", {}).get("spdx_id", "Apache-2.0")

    # Detect tokenizer type from manifest components
    has_sentencepiece = any(
        c["path"].endswith("sentencepiece.bpe.model")
        for c in manifest.get("components", [])
    )
    algo = "sentencepiece-bpe" if has_sentencepiece else "wordpiece"
    vocab_size = "250002" if has_sentencepiece else "30522"

    return {
        "type": "data",
        "bom-ref": f"{name}/tokenizer",
        "name": f"{name}-tokenizer",
        "version": version,
        "publisher": _publisher_from_family(family),
        "description": f"{algo} tokenizer for {name}.",
        "licenses": [{"license": {"id": spdx}}],
        "properties": sorted([
            {"name": "nlp.tokenizer.algorithm", "value": algo},
            {"name": "nlp.tokenizer.vocab_size", "value": vocab_size},
        ], key=lambda p: p["name"]),
    }


def render_build_dep_components(deps: list[tuple[str, str, str]]) -> list[dict]:
    return [
        {
            "type": "library",
            "bom-ref": f"build/{name}@{version}",
            "name": name,
            "version": version,
            "purl": f"pkg:pypi/{name}@{version}",
            "licenses": [{"license": {"id": spdx}}],
        }
        for (name, version, spdx) in sorted(deps, key=lambda t: t[0])
    ]


def _arch_from_name(name: str) -> str:
    if "L6" in name:
        return "MiniLM-L6"
    if "L12" in name:
        return "MiniLM-L12"
    if "bge-large" in name.lower():
        return "BGE-large"
    return "unknown"


def _publisher_from_family(family: str) -> str:
    if family.startswith("sentence-transformers"):
        return "sentence-transformers (UKPLab)"
    if family.startswith("BAAI"):
        return "BAAI"
    if family.startswith("intfloat"):
        return "intfloat"
    return family


def _description_from_name(name: str) -> str:
    if name == "all-MiniLM-L6-v2":
        return (
            "Sentence transformer for English semantic similarity. "
            "Distilled MiniLM-L6 architecture, 384-dim output, "
            "~22M parameters."
        )
    if name == "paraphrase-multilingual-MiniLM-L12-v2":
        return (
            "Sentence transformer for multilingual paraphrase similarity. "
            "Distilled from XLM-R, 12-layer MiniLM architecture, 384-dim, "
            "~118M parameters. Supports 50+ direct languages, 100+ via "
            "cross-lingual transfer."
        )
    return f"Sentence transformer model {name}."


# ---------------------------------------------------------------------------
#  SBOM assembly
# ---------------------------------------------------------------------------


def build_sbom(manifest: dict, *, hf_revision: str, build_deps: list,
               source_date_epoch: int) -> dict:
    bundle_slug = manifest.get("bundle_slug", "")
    manifest_sha = hashlib.sha256(
        canonical_json(manifest).encode("utf-8")
    ).hexdigest()

    iso_ts = iso_timestamp(source_date_epoch)

    components = sorted(
        [render_ml_model_component(manifest, hf_revision),
         render_tokenizer_component(manifest)]
        + render_build_dep_components(build_deps),
        key=lambda c: c["bom-ref"],
    )

    sbom = {
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "serialNumber": deterministic_serial_number(bundle_slug, manifest_sha),
        "version": 1,
        "metadata": {
            "timestamp": iso_ts,
            "tools": [
                {
                    "vendor": "EFE Project",
                    "name": "generate_bundle_sbom.py",
                    "version": "1.0.0",
                }
            ],
            "component": {
                "type": "application",
                "bom-ref": f"bundle/{bundle_slug}",
                "name": bundle_slug.rsplit("-", 1)[0] if "-" in bundle_slug else bundle_slug,
                "version": bundle_slug.rsplit("-", 1)[-1] if "-" in bundle_slug else "0.0.0",
                "description": f"EFE M13 bundle {bundle_slug}.",
            },
        },
        "components": components,
        "vulnerabilities": [],
        "annotations": [
            {
                "bom-ref": f"bundle/{bundle_slug}",
                "subjects": [f"bundle/{bundle_slug}"],
                "annotator": {"organization": {"name": "EFE Project"}},
                "timestamp": iso_ts,
                "text": f"Manifest SHA-256: {manifest_sha}",
            }
        ],
    }
    return sbom


# ---------------------------------------------------------------------------
#  Driver
# ---------------------------------------------------------------------------


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0] if __doc__ else None)
    p.add_argument("--bundle-dir", type=Path, required=True)
    p.add_argument("--hf-revision", type=str, required=True,
                   help="Pinned 40-char HF SHA, or 'dev-placeholder'.")
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--source-date-epoch", type=int, default=None)
    args = p.parse_args(argv)

    if args.source_date_epoch is not None:
        os.environ["SOURCE_DATE_EPOCH"] = str(args.source_date_epoch)
    sde = resolve_source_date_epoch()

    manifest_path = args.bundle_dir / "manifest.json"
    if not manifest_path.exists():
        print(f"::error::manifest.json not found at {manifest_path}", file=sys.stderr)
        return 1
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))

    sbom = build_sbom(
        manifest,
        hf_revision=args.hf_revision,
        build_deps=BUILD_DEPS_DEFAULT,
        source_date_epoch=sde,
    )
    args.output.write_text(canonical_json(sbom), encoding="utf-8")
    print(f"SBOM_OK path={args.output} components={len(sbom['components'])}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
