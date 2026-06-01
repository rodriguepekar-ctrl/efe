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
- **Manifest SHA-256** : `7a0e393dc77186cf17f143067a9b29c08186e3a1f4a5a52c3ccec26346a2373e`
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
oras pull oci://registry.efe.ai/bundles/all-MiniLM-L6-v2-en:1.0.0 \
    -o /tmp/bundle/

# 2. Verify Ed25519 signature
python3 scripts/release/verify_pulled_bundle.py \
    --bundle-dir /tmp/bundle/ \
    --pubkey-id efe-dev-bundle-2026

# 3. Verify per-file SHA256 against manifest claims
python3 -c "
import json, hashlib
from pathlib import Path
m = json.loads(Path('/tmp/bundle/manifest.json').read_text())
for c in m['components']:
    f = Path('/tmp/bundle') / c['path']
    if not f.exists():
        if c['required']: raise SystemExit(f'FAIL: {c[\"path\"]} missing')
        continue
    actual = hashlib.sha256(f.read_bytes()).hexdigest()
    assert actual == c['sha256'], f'hash drift: {c[\"path\"]}'
print('VERIFY_OK')
"
```

## Use in EFE Helm chart

```bash
helm install efe charts/efe \
    --set config.embedder.backend=bundle \
    --set bundle.slug=all-MiniLM-L6-v2-en \
    --set bundle.version=1.0.0 \
    --set bundle.manifestSha256=7a0e393dc77186cf17f143067a9b29c08186e3a1f4a5a52c3ccec26346a2373e
```

The M12 D27 bundle reload hook (`templates/d27-bundle-reload-hook.yaml`)
will fetch the bundle from OCI + reload without rolling restart.

## Anti-regression pin tests

* `scripts/tests/test_d20_sample_bundle.py` — 8 tests covering layout,
  manifest schema, signature, license, size, hf_revision pinning.
