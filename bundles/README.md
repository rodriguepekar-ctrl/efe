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
