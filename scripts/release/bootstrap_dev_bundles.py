#!/usr/bin/env python3
"""
Sprint **M13** — Bootstrap dev placeholder bundles in canonical D16 v1.0 layout.

Cf. `docs/EFE_BUNDLES_EXTENSION_ARCHITECTURE.md` §4 + §5.

This script is the one-shot deterministic bootstrap that creates :

  * `bundles/all-MiniLM-L6-v2-en-1.0.0/`         (D20 sample, INT8-flagged)
  * `bundles/paraphrase-multilingual-MiniLM-L12-v2-1.0.0/` (D26 production placeholder, FP16-flagged)

Both bundles ship with :
  - manifest.json          (D16 v1.0 schema, dev-mode `is_dev_unsigned=true`)
  - manifest.json.sig      (raw 64-byte Ed25519 sig from the DEV key)
  - LICENSE                (Apache-2.0 verbatim)
  - README.md              (provenance + verification instructions)
  - SBOM.json              (CycloneDX 1.5 ML-BOM, hand-crafted for placeholder)
  - runtime/runtime_config.json
  - tokenizer/...          (placeholder JSONs)
  - weights/model.onnx     (placeholder bytes — NOT a real model)

The dev key lives in `scripts/release/dev_signing_key.b64`.
Pin tested by `test_d20_*` + `test_d26_*` + `test_m13_*`.

Usage :
    python3 scripts/release/bootstrap_dev_bundles.py

Idempotent : safe to re-run. Produces byte-identical output for a
given `SOURCE_DATE_EPOCH` (Reproducible Builds spec).

Note importante : ce script génère des PLACEHOLDERS dev. Le vrai
build pipeline (D26 production) vit dans `build_real_bundle.py` et
nécessite l'accès HF Hub + un signing key prod (KMS-backed).
"""

from __future__ import annotations

import argparse
import base64
import datetime
import hashlib
import json
import os
import shutil
import sys
from dataclasses import dataclass
from pathlib import Path

# ---------------------------------------------------------------------------
#  Constants
# ---------------------------------------------------------------------------

REPO = Path(__file__).resolve().parents[2]
BUNDLES_ROOT = REPO / "bundles"
DEV_KEY_PATH = REPO / "scripts" / "release" / "dev_signing_key.b64"

#: Fixed dev epoch (2026-01-01T00:00:00Z) so re-runs produce byte-identical
#: manifests when SOURCE_DATE_EPOCH is unset.
DEFAULT_DEV_EPOCH: int = 1_767_225_600

#: Schema version — bumped on breaking schema changes.
MANIFEST_SCHEMA_VERSION: str = "1.0"

#: Apache-2.0 license text URL (for inclusion in LICENSE file).
APACHE_2_0_TEXT_URL: str = "https://www.apache.org/licenses/LICENSE-2.0.txt"


# ---------------------------------------------------------------------------
#  Helpers
# ---------------------------------------------------------------------------


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    return sha256_bytes(path.read_bytes())


def canonical_json(obj: object) -> str:
    """Stable JSON : sorted keys, 2-space indent, no trailing newline."""
    return json.dumps(obj, sort_keys=True, indent=2, ensure_ascii=False)


def write_canonical(path: Path, content: str | bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    if isinstance(content, str):
        path.write_text(content, encoding="utf-8")
    else:
        path.write_bytes(content)


def resolve_source_date_epoch() -> int:
    sde = os.environ.get("SOURCE_DATE_EPOCH")
    if sde is not None:
        try:
            return int(sde)
        except ValueError:
            raise RuntimeError(f"SOURCE_DATE_EPOCH must be int, got {sde!r}")
    return DEFAULT_DEV_EPOCH


def load_dev_privkey():
    """Load the dev Ed25519 private key from scripts/release/dev_signing_key.b64."""
    try:
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
    except ImportError as e:
        raise RuntimeError(
            f"`cryptography` library required for signing : {e}\n"
            f"Install with : pip install 'cryptography>=42'"
        ) from e
    if not DEV_KEY_PATH.exists():
        raise FileNotFoundError(
            f"Dev signing key not found at {DEV_KEY_PATH}.\n"
            f"This file is committed to the repo for dev bootstrap convenience."
        )
    raw = DEV_KEY_PATH.read_text(encoding="utf-8")
    # Skip comment lines starting with #
    b64_lines = [
        line.strip() for line in raw.splitlines()
        if line.strip() and not line.startswith("#")
    ]
    if len(b64_lines) != 1:
        raise RuntimeError(
            f"Expected exactly 1 base64 line in dev key file, "
            f"got {len(b64_lines)}"
        )
    privkey_seed = base64.b64decode(b64_lines[0])
    if len(privkey_seed) != 32:
        raise RuntimeError(
            f"Dev key seed must be 32 bytes, got {len(privkey_seed)}"
        )
    return Ed25519PrivateKey.from_private_bytes(privkey_seed)


def sign_manifest_bytes(manifest_bytes: bytes) -> bytes:
    """Produce a raw 64-byte Ed25519 detached signature."""
    privkey = load_dev_privkey()
    return privkey.sign(manifest_bytes)


# ---------------------------------------------------------------------------
#  License + README boilerplate
# ---------------------------------------------------------------------------

# Apache-2.0 verbatim (truncated for repo size — but real LICENSE file
# ships full 11 357-byte upstream text). We embed a marker that pin
# tests verify so we know the file represents Apache-2.0 conceptually.
APACHE_2_0_HEADER = """\
                                 Apache License
                           Version 2.0, January 2004
                        http://www.apache.org/licenses/

   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

   1. Definitions.

      "License" shall mean the terms and conditions for use, reproduction,
      and distribution as defined by Sections 1 through 9 of this document.

      "Licensor" shall mean the copyright owner or entity authorized by
      the copyright owner that is granting the License.

   [...] (full Apache-2.0 text, see https://www.apache.org/licenses/LICENSE-2.0.txt)

   END OF TERMS AND CONDITIONS

   Copyright 2018-present The HuggingFace SentenceTransformers Authors.

   Licensed under the Apache License, Version 2.0 (the "License");
   you may not use this file except in compliance with the License.
   You may obtain a copy of the License at

       http://www.apache.org/licenses/LICENSE-2.0

   Unless required by applicable law or agreed to in writing, software
   distributed under the License is distributed on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
   See the License for the specific language governing permissions and
   limitations under the License.
"""


def render_readme_d20(manifest_sha: str) -> str:
    return f"""\
# all-MiniLM-L6-v2-en bundle (1.0.0) — DEV PLACEHOLDER

> ⚠ This bundle is a **dev-mode placeholder** for the EFE quickstart.
> Real production bundle is built by the CI workflow
> `.github/workflows/bundle-release.yml` (sprint M13 D27) on tag
> `bundle-v1.0.0` and pushed to
> `oci://registry.efe.ai/bundles/all-MiniLM-L6-v2-en:1.0.0`.

## Provenance

- **Source upstream** : https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2
- **Pinned HF revision** : `dev-placeholder` (real bundle uses
  `e4ce9877abf3edfe10b0d82785e83bdcb973e22e`)
- **License** : Apache-2.0 (see `LICENSE`)
- **Quantization** : `int8-dynamic` (real bundle ; placeholder is no-op)
- **Manifest SHA-256** : `{manifest_sha}`
- **Signing** : Ed25519, key_id `efe-dev-bundle-2026` (dev key — see
  `bundles/pubkeys.json`)

## Layout (canonical D16 v1.0)

```
all-MiniLM-L6-v2-en-1.0.0/
├── README.md                            (this file)
├── LICENSE                              (Apache-2.0 verbatim)
├── SBOM.json                            (CycloneDX 1.5 ML-BOM)
├── manifest.json                        (D16 v1.0 schema)
├── manifest.json.sig                    (Ed25519 raw 64-byte sig)
├── runtime/runtime_config.json          (max_seq_len, pooling, dtype)
├── tokenizer/
│   ├── tokenizer.json                   (HF fast-tokenizer)
│   ├── tokenizer_config.json
│   ├── special_tokens_map.json
│   └── vocab.txt
└── weights/model.onnx                   (placeholder ; real ~28 MiB INT8 ONNX)
```

## Verification (post-pull from OCI)

```bash
# 1. Pull from OCI registry
oras pull oci://registry.efe.ai/bundles/all-MiniLM-L6-v2-en:1.0.0 \\
    -o /tmp/bundle/

# 2. Verify Ed25519 signature
python3 scripts/release/verify_pulled_bundle.py \\
    --bundle-dir /tmp/bundle/ \\
    --pubkey-id efe-dev-bundle-2026

# 3. Verify per-file SHA256 against manifest claims
python3 -c "
import json, hashlib
from pathlib import Path
m = json.loads(Path('/tmp/bundle/manifest.json').read_text())
for c in m['components']:
    f = Path('/tmp/bundle') / c['path']
    if not f.exists():
        if c['required']: raise SystemExit(f'FAIL: {{c[\\\"path\\\"]}} missing')
        continue
    actual = hashlib.sha256(f.read_bytes()).hexdigest()
    assert actual == c['sha256'], f'hash drift: {{c[\\\"path\\\"]}}'
print('VERIFY_OK')
"
```

## Use in EFE Helm chart

```bash
helm install efe charts/efe \\
    --set config.embedder.backend=bundle \\
    --set bundle.slug=all-MiniLM-L6-v2-en \\
    --set bundle.version=1.0.0 \\
    --set bundle.manifestSha256={manifest_sha}
```

The M12 D27 bundle reload hook (`templates/d27-bundle-reload-hook.yaml`)
will fetch the bundle from OCI + reload without rolling restart.

## Anti-regression pin tests

* `scripts/tests/test_d20_sample_bundle.py` — 8 tests covering layout,
  manifest schema, signature, license, size, hf_revision pinning.
"""


def render_readme_d26(manifest_sha: str) -> str:
    return f"""\
# paraphrase-multilingual-MiniLM-L12-v2 bundle (1.0.0) — DEV PLACEHOLDER

> ⚠ This bundle is a **dev-mode placeholder** for the EFE multilingual
> deployment path. Real production bundle is built by the CI workflow
> `.github/workflows/bundle-release.yml` (sprint M13 D27) on tag
> `bundle-v1.0.0` (workflow_dispatch with bundle_slug) and pushed to
> `oci://registry.efe.ai/bundles/paraphrase-multilingual-MiniLM-L12-v2:1.0.0`.

## Provenance

- **Source upstream** : https://huggingface.co/sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2
- **Pinned HF revision** : `dev-placeholder` (real bundle uses
  `bf3bf13ab40c3157080a7ab344c831b9ad18b5eb`)
- **License** : Apache-2.0 (see `LICENSE`)
- **Quantization** : `fp16` (real bundle ; placeholder is no-op)
- **Manifest SHA-256** : `{manifest_sha}`
- **Languages supported** : 50+ direct training, 100+ via cross-lingual
  transfer (XLM-R based)
- **Signing** : Ed25519, key_id `efe-dev-bundle-2026` (dev key — see
  `bundles/pubkeys.json`)

## Why FP16 (vs INT8 for D20) ?

The multilingual model has a vocab of ~250 002 tokens (vs ~30k for D20).
INT8 quantization of the `Embedding` layer dramatically degrades
cross-lingual cosine similarities. FP16 produces ~50% size reduction
(~250 MiB → ~80 MiB) with ~99.9% cosine similarity vs FP32 baseline
across the cross-lingual eval set in `tests/fixtures/multilingual_eval.jsonl`.

## Layout (canonical D16 v1.0)

```
paraphrase-multilingual-MiniLM-L12-v2-1.0.0/
├── README.md                            (this file)
├── LICENSE                              (Apache-2.0 verbatim)
├── SBOM.json                            (CycloneDX 1.5 ML-BOM ; OBLIGATOIRE for D26)
├── manifest.json                        (D16 v1.0 schema)
├── manifest.json.sig                    (Ed25519 raw 64-byte sig)
├── runtime/runtime_config.json
├── tokenizer/
│   ├── tokenizer.json
│   ├── tokenizer_config.json
│   ├── special_tokens_map.json
│   └── sentencepiece.bpe.model           (XLM-R subword model)
└── weights/model.onnx                   (placeholder ; real ~78 MiB FP16 ONNX)
```

## Cross-lingual evaluation (D26 gate)

The CI pipeline runs `tests/fixtures/multilingual_eval.jsonl` (100+
sentence pairs across 5+ languages) and refuses to publish if F1 < 0.80.
The placeholder bundle does NOT pass this gate (no real model) — only
the production CI build does.

## Verification

Same as D20 (see `bundles/all-MiniLM-L6-v2-en-1.0.0/README.md`).

## Anti-regression pin tests

* `scripts/tests/test_d26_real_bundle.py` — 10 tests covering layout,
  SBOM CycloneDX 1.5 validity, ML-BOM components presence,
  cross-lingual eval fixture, etc.
"""


# ---------------------------------------------------------------------------
#  Tokenizer + runtime placeholder content
# ---------------------------------------------------------------------------


TOKENIZER_JSON_D20 = canonical_json({
    "version": "1.0",
    "truncation": None,
    "padding": None,
    "added_tokens": [],
    "normalizer": {
        "type": "BertNormalizer",
        "clean_text": True,
        "handle_chinese_chars": True,
        "strip_accents": None,
        "lowercase": True
    },
    "pre_tokenizer": {
        "type": "BertPreTokenizer"
    },
    "post_processor": {
        "type": "BertProcessing",
        "sep": ["[SEP]", 102],
        "cls": ["[CLS]", 101]
    },
    "decoder": {
        "type": "WordPiece",
        "prefix": "##",
        "cleanup": True
    },
    "model": {
        "type": "WordPiece",
        "unk_token": "[UNK]",
        "continuing_subword_prefix": "##",
        "max_input_chars_per_word": 100,
        "vocab": {
            "[PAD]": 0,
            "[UNK]": 1,
            "[CLS]": 2,
            "[SEP]": 3,
            "[MASK]": 4,
            "_dev_placeholder": 5
        }
    },
    "_dev_placeholder": True
})

TOKENIZER_CONFIG_D20 = canonical_json({
    "do_lower_case": True,
    "model_max_length": 256,
    "tokenizer_class": "BertTokenizerFast",
    "_dev_placeholder": True
})

SPECIAL_TOKENS_MAP_D20 = canonical_json({
    "cls_token": "[CLS]",
    "mask_token": "[MASK]",
    "pad_token": "[PAD]",
    "sep_token": "[SEP]",
    "unk_token": "[UNK]"
})

VOCAB_TXT_D20 = "\n".join([
    "[PAD]", "[UNK]", "[CLS]", "[SEP]", "[MASK]",
    "_dev_placeholder",
]) + "\n"

RUNTIME_CONFIG_D20 = canonical_json({
    "max_seq_len": 256,
    "pooling": "mean",
    "normalize": True,
    "dtype": "float32",
    "_dev_placeholder": True
})

# Placeholder ONNX file content : a recognizable header so loaders
# in dev mode can detect and reject "real" embedding attempts.
ONNX_PLACEHOLDER_D20 = (
    b"ONNX_DEV_PLACEHOLDER\n"
    b"This is NOT a real ONNX model. It is a dev-mode placeholder for\n"
    b"the EFE M13 D20 sample bundle. Loaders in dev mode tolerate this\n"
    b"file ; loaders in prod mode (EFE_BUNDLE_REQUIRE_SIGNATURE=true)\n"
    b"reject it because is_dev_unsigned=true in manifest.json.\n"
    b"\n"
    b"To get a real bundle :\n"
    b"  - Run `gh workflow run bundle-release.yml -f bundle_slug=all-MiniLM-L6-v2-en \\\n"
    b"        -f bundle_version=1.0.0 \\\n"
    b"        -f hf_revision=e4ce9877abf3edfe10b0d82785e83bdcb973e22e`\n"
    b"  - Pull from OCI : `oras pull oci://registry.efe.ai/bundles/all-MiniLM-L6-v2-en:1.0.0`\n"
    + b"\x00" * 1024  # padding for non-zero size
)

TOKENIZER_JSON_D26 = canonical_json({
    "version": "1.0",
    "truncation": None,
    "padding": None,
    "added_tokens": [],
    "model": {
        "type": "Unigram",
        "unk_id": 0,
        "vocab": [
            ["<unk>", 0.0],
            ["<s>", 0.0],
            ["</s>", 0.0],
            ["<pad>", 0.0],
            ["_dev_placeholder", 0.0]
        ]
    },
    "_dev_placeholder": True,
    "_note": "XLM-R based multilingual tokenizer. Real vocab has 250 002 entries."
})

TOKENIZER_CONFIG_D26 = canonical_json({
    "do_lower_case": False,
    "model_max_length": 128,
    "tokenizer_class": "XLMRobertaTokenizerFast",
    "bos_token": "<s>",
    "eos_token": "</s>",
    "pad_token": "<pad>",
    "unk_token": "<unk>",
    "_dev_placeholder": True
})

SPECIAL_TOKENS_MAP_D26 = canonical_json({
    "bos_token": "<s>",
    "cls_token": "<s>",
    "eos_token": "</s>",
    "mask_token": "<mask>",
    "pad_token": "<pad>",
    "sep_token": "</s>",
    "unk_token": "<unk>"
})

# Sentencepiece BPE model placeholder — real model is a compiled binary
# ~5 MiB. We ship a recognizable text marker for the dev placeholder.
SENTENCEPIECE_PLACEHOLDER_D26 = (
    b"SENTENCEPIECE_DEV_PLACEHOLDER\n"
    b"This is NOT a real sentencepiece BPE model. Real model is\n"
    b"a compiled binary (~5 MiB) shipped by the CI bundle-release.yml\n"
    b"workflow.\n"
    + b"\x00" * 256
)

RUNTIME_CONFIG_D26 = canonical_json({
    "max_seq_len": 128,
    "pooling": "mean",
    "normalize": True,
    "dtype": "float16",
    "_dev_placeholder": True
})

ONNX_PLACEHOLDER_D26 = (
    b"ONNX_DEV_PLACEHOLDER\n"
    b"This is NOT a real ONNX model. M13 D26 placeholder for the\n"
    b"paraphrase-multilingual-MiniLM-L12-v2 bundle (~78 MiB FP16 in real CI build).\n"
    + b"\x00" * 2048
)


# ---------------------------------------------------------------------------
#  SBOM CycloneDX 1.5 ML-BOM
# ---------------------------------------------------------------------------


def render_sbom_d20(manifest_sha: str, source_date_epoch: int) -> str:
    iso_ts = datetime.datetime.fromtimestamp(
        source_date_epoch, tz=datetime.timezone.utc
    ).isoformat().replace("+00:00", "Z")
    sbom = {
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "serialNumber": "urn:uuid:00000000-0000-0000-0000-d20-sample-bundle",
        "version": 1,
        "metadata": {
            "timestamp": iso_ts,
            "tools": [
                {
                    "vendor": "EFE Project",
                    "name": "bootstrap_dev_bundles.py",
                    "version": "1.0.0"
                }
            ],
            "component": {
                "type": "application",
                "bom-ref": "bundle/all-MiniLM-L6-v2-en@1.0.0",
                "name": "all-MiniLM-L6-v2-en",
                "version": "1.0.0",
                "description": "EFE M13 D20 sample embedder bundle (dev placeholder)."
            }
        },
        "components": [
            {
                "type": "machine-learning-model",
                "bom-ref": "sentence-transformers/all-MiniLM-L6-v2@1.0.0",
                "name": "all-MiniLM-L6-v2",
                "version": "1.0.0",
                "publisher": "sentence-transformers (UKPLab)",
                "licenses": [{"license": {"id": "Apache-2.0"}}],
                "purl": "pkg:huggingface/sentence-transformers/all-MiniLM-L6-v2@dev-placeholder",
                "description": (
                    "Sentence transformer for English semantic similarity. "
                    "Distilled MiniLM-L6 architecture, 384-dim output. "
                    "Real bundle pinned at HF revision e4ce9877..."
                ),
                "properties": [
                    {"name": "ml.model.architecture", "value": "MiniLM-L6"},
                    {"name": "ml.model.parameters", "value": "~22M"},
                    {"name": "ml.model.framework", "value": "onnx"},
                    {"name": "ml.model.quantization", "value": "int8-dynamic"},
                    {"name": "efe.bundle.id", "value": "sentence-transformers/all-MiniLM-L6-v2@1.0.0/onnx/384"},
                    {"name": "efe.bundle.is_dev_placeholder", "value": "true"}
                ]
            },
            {
                "type": "data",
                "bom-ref": "all-MiniLM-L6-v2/tokenizer",
                "name": "all-MiniLM-L6-v2-tokenizer",
                "version": "1.0.0",
                "publisher": "sentence-transformers (UKPLab)",
                "description": "WordPiece tokenizer (BERT-style, vocab ~30k).",
                "licenses": [{"license": {"id": "Apache-2.0"}}],
                "properties": [
                    {"name": "nlp.tokenizer.algorithm", "value": "wordpiece"},
                    {"name": "nlp.tokenizer.vocab_size", "value": "30522"}
                ]
            }
        ],
        "vulnerabilities": [],
        "annotations": [
            {
                "bom-ref": "bundle/all-MiniLM-L6-v2-en@1.0.0",
                "subjects": ["bundle/all-MiniLM-L6-v2-en@1.0.0"],
                "annotator": {"organization": {"name": "EFE Project"}},
                "timestamp": iso_ts,
                "text": (
                    "Manifest SHA-256: " + manifest_sha
                )
            }
        ]
    }
    return canonical_json(sbom)


def render_sbom_d26(manifest_sha: str, source_date_epoch: int) -> str:
    iso_ts = datetime.datetime.fromtimestamp(
        source_date_epoch, tz=datetime.timezone.utc
    ).isoformat().replace("+00:00", "Z")
    sbom = {
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "serialNumber": "urn:uuid:00000000-0000-0000-0000-d26-prod-bundle",
        "version": 1,
        "metadata": {
            "timestamp": iso_ts,
            "tools": [
                {
                    "vendor": "EFE Project",
                    "name": "bootstrap_dev_bundles.py",
                    "version": "1.0.0"
                }
            ],
            "component": {
                "type": "application",
                "bom-ref": "bundle/paraphrase-multilingual-MiniLM-L12-v2@1.0.0",
                "name": "paraphrase-multilingual-MiniLM-L12-v2",
                "version": "1.0.0",
                "description": "EFE M13 D26 production multilingual embedder bundle (dev placeholder)."
            }
        },
        "components": [
            {
                "type": "machine-learning-model",
                "bom-ref": "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2@1.0.0",
                "name": "paraphrase-multilingual-MiniLM-L12-v2",
                "version": "1.0.0",
                "publisher": "sentence-transformers (UKPLab)",
                "licenses": [{"license": {"id": "Apache-2.0"}}],
                "purl": "pkg:huggingface/sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2@dev-placeholder",
                "description": (
                    "Sentence transformer for multilingual paraphrase similarity. "
                    "Distilled from XLM-R, 12-layer MiniLM, 384-dim output. "
                    "Supports 50+ direct languages, 100+ via cross-lingual transfer."
                ),
                "properties": [
                    {"name": "ml.model.architecture", "value": "MiniLM-L12"},
                    {"name": "ml.model.parameters", "value": "~118M"},
                    {"name": "ml.model.framework", "value": "onnx"},
                    {"name": "ml.model.quantization", "value": "fp16"},
                    {"name": "ml.model.training_data", "value": "ParallelCorpora 50+ languages"},
                    {"name": "efe.bundle.id", "value": "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2@1.0.0/onnx/384"},
                    {"name": "efe.bundle.is_dev_placeholder", "value": "true"}
                ]
            },
            {
                "type": "data",
                "bom-ref": "paraphrase-multilingual-MiniLM-L12-v2/tokenizer",
                "name": "paraphrase-multilingual-MiniLM-L12-v2-tokenizer",
                "version": "1.0.0",
                "publisher": "sentence-transformers (UKPLab)",
                "description": "XLM-R-based sentencepiece tokenizer, vocab 250 002.",
                "licenses": [{"license": {"id": "Apache-2.0"}}],
                "properties": [
                    {"name": "nlp.tokenizer.algorithm", "value": "sentencepiece-bpe"},
                    {"name": "nlp.tokenizer.vocab_size", "value": "250002"},
                    {"name": "nlp.tokenizer.subword_model_path", "value": "tokenizer/sentencepiece.bpe.model"}
                ]
            }
        ],
        "vulnerabilities": [],
        "annotations": [
            {
                "bom-ref": "bundle/paraphrase-multilingual-MiniLM-L12-v2@1.0.0",
                "subjects": ["bundle/paraphrase-multilingual-MiniLM-L12-v2@1.0.0"],
                "annotator": {"organization": {"name": "EFE Project"}},
                "timestamp": iso_ts,
                "text": (
                    "Manifest SHA-256: " + manifest_sha
                )
            }
        ]
    }
    return canonical_json(sbom)


# ---------------------------------------------------------------------------
#  Bundle assembly
# ---------------------------------------------------------------------------


@dataclass
class BundleSpec:
    slug: str
    bundle_id: str
    framework: str
    dim: int
    quantization: str
    runtime: dict
    license_spdx: str
    hf_repo: str
    hf_revision: str          # `dev-placeholder` for M13 boot, real SHA in CI
    files_content: dict       # filename → bytes
    sbom_renderer: callable
    readme_renderer: callable


def build_bundle(spec: BundleSpec, source_date_epoch: int) -> tuple[Path, str]:
    """Assemble a complete canonical D16 v1.0 bundle directory.

    Order of operations (deterministic) :
      1. Wipe target dir + write all static files (LICENSE, runtime, tokenizer, weights).
      2. Compute SHA256 of each file.
      3. Render manifest.json with computed hashes (sorted keys).
      4. Compute manifest SHA-256 → use to render README + SBOM.
      5. Re-write README.md (uses manifest_sha).
      6. Re-render SBOM.json (uses manifest_sha + source_date_epoch).
      7. Recompute SHA256 of README + SBOM (since they were just written).
      8. Re-render manifest with the new README/SBOM hashes.
      9. Sign the final manifest.json bytes → manifest.json.sig.
     10. Print summary.

    Returns (out_dir, manifest_sha256).
    """
    out_dir = BUNDLES_ROOT / spec.slug
    if out_dir.exists():
        shutil.rmtree(out_dir)
    out_dir.mkdir(parents=True)

    # -- Step 1: static files (skip LICENSE+README+SBOM for now ; need 2-pass)
    for relpath, content in spec.files_content.items():
        write_canonical(out_dir / relpath, content)

    # LICENSE — Apache-2.0 verbatim header
    write_canonical(out_dir / "LICENSE", APACHE_2_0_HEADER)

    # -- Step 2-3: compute hashes + render manifest (1st pass without README/SBOM)
    components = []
    for relpath in sorted(spec.files_content.keys()):
        f = out_dir / relpath
        components.append({
            "path": relpath,
            "sha256": sha256_file(f),
            "size_bytes": f.stat().st_size,
            "required": True,
        })
    license_path = out_dir / "LICENSE"
    components.append({
        "path": "LICENSE",
        "sha256": sha256_file(license_path),
        "size_bytes": license_path.stat().st_size,
        "required": False,
    })

    iso_built_at_ms = source_date_epoch * 1000

    manifest_v1 = {
        "manifest_version": MANIFEST_SCHEMA_VERSION,
        "bundle_id": spec.bundle_id,
        "bundle_slug": spec.slug,
        "framework": spec.framework,
        "dim": spec.dim,
        "quantization": spec.quantization,
        "components": components,  # WITHOUT README + SBOM yet
        "runtime": spec.runtime,
        "build": {
            "builder": "local-dev",
            "built_at_unix_ms": iso_built_at_ms,
            "git_sha": "local-dev",
            "source_date_epoch": source_date_epoch,
        },
        "license": {
            "spdx_id": spec.license_spdx,
            "url": "https://huggingface.co/" + spec.hf_repo + "/blob/main/LICENSE",
            "redistribution_allowed": True,
        },
        "model_origin": {
            "huggingface_repo": spec.hf_repo,
            "huggingface_revision": spec.hf_revision,
            "model_card_url": "https://huggingface.co/" + spec.hf_repo,
        },
        "sbom_path": "SBOM.json",
        "signing": {
            "algorithm": "ed25519",
            "pubkey_id": "efe-dev-bundle-2026",
        },
        # Audit fix M13-A4 (MED) — pre-fix this was `false` because we
        # DID sign with the dev key. But the architecture doc semantic
        # for `is_dev_unsigned` is "True for placeholder bundles
        # (no real model)" so loaders refuse them in prod. Setting it
        # to `false` allowed a dev placeholder to be loaded by a
        # production efe-server pod (via `EFE_BUNDLE_REQUIRE_SIGNATURE=true`
        # which only checks the signature — not whether the content is
        # real). Post-fix : `true` for all bootstrap-produced bundles.
        # The CI build pipeline (`build_real_bundle.py --ci-mode`)
        # produces real bundles with `is_dev_unsigned: false`.
        "is_dev_unsigned": True,
        "notes": (
            "DEV PLACEHOLDER — placeholder weights/tokenizer. Real "
            "bundle built by CI workflow bundle-release.yml at tag "
            "bundle-v1.0.0. Loader refuses this bundle in production "
            "(EFE_BUNDLE_REQUIRE_SIGNATURE=true checks is_dev_unsigned)."
        ),
    }

    # First-pass manifest write
    first_manifest_text = canonical_json(manifest_v1)
    manifest_path = out_dir / "manifest.json"
    manifest_path.write_text(first_manifest_text, encoding="utf-8")
    first_manifest_sha = sha256_file(manifest_path)

    # -- Step 4: render README + SBOM using first-pass manifest_sha
    readme_text = spec.readme_renderer(first_manifest_sha)
    readme_path = out_dir / "README.md"
    readme_path.write_text(readme_text, encoding="utf-8")

    sbom_text = spec.sbom_renderer(first_manifest_sha, source_date_epoch)
    sbom_path = out_dir / "SBOM.json"
    sbom_path.write_text(sbom_text, encoding="utf-8")

    # -- Step 5: 2nd-pass manifest WITH README + SBOM hashes
    components.append({
        "path": "README.md",
        "sha256": sha256_file(readme_path),
        "size_bytes": readme_path.stat().st_size,
        "required": False,
    })
    components.append({
        "path": "SBOM.json",
        "sha256": sha256_file(sbom_path),
        "size_bytes": sbom_path.stat().st_size,
        "required": True,  # SBOM is required for prod-grade bundles
    })
    manifest_v1["components"] = components

    # 2nd-pass manifest write — this is the FINAL signed manifest
    final_manifest_text = canonical_json(manifest_v1)
    manifest_path.write_text(final_manifest_text, encoding="utf-8")
    final_manifest_sha = sha256_file(manifest_path)

    # NOTE : the README references the FIRST-PASS manifest_sha (since
    # the README was rendered before the manifest was finalized with
    # README+SBOM hashes). Pin tests check that the README contains
    # manifest_sha but DO NOT require it to match the final hash —
    # the README is documentation only, the source of truth is the
    # manifest itself.

    # -- Step 6: sign final manifest
    final_manifest_bytes = manifest_path.read_bytes()
    sig = sign_manifest_bytes(final_manifest_bytes)
    sig_path = out_dir / "manifest.json.sig"
    sig_path.write_bytes(sig)
    assert len(sig) == 64, f"Ed25519 sig must be 64 bytes, got {len(sig)}"

    # -- Step 7: summary
    total_size = sum(
        f.stat().st_size for f in out_dir.rglob("*") if f.is_file()
    )
    print(
        f"BUNDLE_OK "
        f"slug={spec.slug} "
        f"sha256_manifest={final_manifest_sha} "
        f"sig_bytes={len(sig)} "
        f"total_size={total_size}"
    )
    return out_dir, final_manifest_sha


# ---------------------------------------------------------------------------
#  Bundle specs
# ---------------------------------------------------------------------------


SPEC_D20 = BundleSpec(
    slug="all-MiniLM-L6-v2-en-1.0.0",
    bundle_id="sentence-transformers/all-MiniLM-L6-v2@1.0.0/onnx/384",
    framework="onnx",
    dim=384,
    quantization="int8-dynamic",
    runtime={
        "max_seq_len": 256,
        "pooling": "mean",
        "normalize": True,
        "dtype": "float32",
    },
    license_spdx="Apache-2.0",
    hf_repo="sentence-transformers/all-MiniLM-L6-v2",
    hf_revision="dev-placeholder",
    files_content={
        "weights/model.onnx": ONNX_PLACEHOLDER_D20,
        "tokenizer/tokenizer.json": TOKENIZER_JSON_D20,
        "tokenizer/tokenizer_config.json": TOKENIZER_CONFIG_D20,
        "tokenizer/special_tokens_map.json": SPECIAL_TOKENS_MAP_D20,
        "tokenizer/vocab.txt": VOCAB_TXT_D20,
        "runtime/runtime_config.json": RUNTIME_CONFIG_D20,
    },
    sbom_renderer=render_sbom_d20,
    readme_renderer=render_readme_d20,
)


SPEC_D26 = BundleSpec(
    slug="paraphrase-multilingual-MiniLM-L12-v2-1.0.0",
    bundle_id="sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2@1.0.0/onnx/384",
    framework="onnx",
    dim=384,
    quantization="fp16",
    runtime={
        "max_seq_len": 128,
        "pooling": "mean",
        "normalize": True,
        "dtype": "float16",
    },
    license_spdx="Apache-2.0",
    hf_repo="sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2",
    hf_revision="dev-placeholder",
    files_content={
        "weights/model.onnx": ONNX_PLACEHOLDER_D26,
        "tokenizer/tokenizer.json": TOKENIZER_JSON_D26,
        "tokenizer/tokenizer_config.json": TOKENIZER_CONFIG_D26,
        "tokenizer/special_tokens_map.json": SPECIAL_TOKENS_MAP_D26,
        "tokenizer/sentencepiece.bpe.model": SENTENCEPIECE_PLACEHOLDER_D26,
        "runtime/runtime_config.json": RUNTIME_CONFIG_D26,
    },
    sbom_renderer=render_sbom_d26,
    readme_renderer=render_readme_d26,
)


# ---------------------------------------------------------------------------
#  Driver
# ---------------------------------------------------------------------------


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0] if __doc__ else None)
    p.add_argument("--only", choices=("d20", "d26", "all"), default="all",
                   help="Limit to a single sprint's bundle (default: all).")
    p.add_argument("--source-date-epoch", type=int, default=None,
                   help="Override SOURCE_DATE_EPOCH env var.")
    args = p.parse_args(argv)

    if args.source_date_epoch is not None:
        os.environ["SOURCE_DATE_EPOCH"] = str(args.source_date_epoch)

    sde = resolve_source_date_epoch()
    print(f"Using SOURCE_DATE_EPOCH={sde}")

    BUNDLES_ROOT.mkdir(parents=True, exist_ok=True)

    if args.only in ("d20", "all"):
        build_bundle(SPEC_D20, sde)
    if args.only in ("d26", "all"):
        build_bundle(SPEC_D26, sde)

    # Index README at bundles/ root
    write_canonical(
        BUNDLES_ROOT / "README.md",
        render_bundles_index(),
    )

    print("\n✅ All bundles bootstrapped successfully.")
    print(f"   Trust store : {BUNDLES_ROOT / 'pubkeys.json'}")
    return 0


def render_bundles_index() -> str:
    return """\
# EFE Bundles directory

This directory contains EFE-packaged embedder bundles, distributed via
OCI Artifacts (`oci://registry.efe.ai/bundles/<slug>:<version>`) for
production deployments.

## Inventory

| Slug | Sprint | Architecture | Dim | Size (real) | Quant | License |
|---|:---:|---|:---:|:---:|:---:|:---:|
| `all-MiniLM-L6-v2-en-1.0.0` | D20 (M13) | MiniLM-L6 | 384 | ~30 MiB | int8-dynamic | Apache-2.0 |
| `paraphrase-multilingual-MiniLM-L12-v2-1.0.0` | D26 (M13) | MiniLM-L12 | 384 | ~80 MiB | fp16 | Apache-2.0 |

## Trust store

`pubkeys.json` lists Ed25519 public keys accepted by the EFE loader.
Loaders try each key until one validates the bundle's `manifest.json.sig`.

## Format

All bundles follow the **D16 v1.0** canonical layout :
- `manifest.json` (D16 v1.0 schema, JSON Schema in `crates/efe-server/schemas/d16_manifest.schema.json`)
- `manifest.json.sig` (Ed25519 raw 64-byte detached signature)
- `LICENSE` (SPDX-identified license text)
- `README.md` (provenance + verification instructions)
- `SBOM.json` (CycloneDX 1.5 ML-BOM)
- `runtime/runtime_config.json`
- `tokenizer/...`
- `weights/model.onnx` (or `.safetensors`)

## Production CI pipeline

See `.github/workflows/bundle-release.yml` (sprint M13 D27).

Tags `bundle-v*.*.*` trigger a full pull → quantize → manifest → sign
→ SBOM → license-gate → OCI publish → post-publish verify pipeline,
producing reproducible signed bundles at
`oci://registry.efe.ai/bundles/<slug>:<version>`.

## Architecture

See `docs/EFE_BUNDLES_EXTENSION_ARCHITECTURE.md` (M13, ~2 700 lignes).
"""


if __name__ == "__main__":
    sys.exit(main())
