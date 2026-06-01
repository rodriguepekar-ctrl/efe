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
- **Manifest SHA-256** : `bc333c27706f3532dbda52f55d79e241787d7255baa20e66f9027542f94daa30`
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
