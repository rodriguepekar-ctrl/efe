# `bundles/all-MiniLM-L6-v2-en-1.0.0/` — sample bundle reference

> **Sprint D20 — DT-PC-1/2** — first real bundle shipped with the
> SDK quickstart. Without it, `efe-admin classify "hello"`
> end-to-end is broken post-install.
>
> **Schema version** : 1
> **Status** : DEV placeholder shipped in repo (32-byte `weights.bin`) ;
> CI release replaces with real INT8-quantized weights.

---

## 1. Bundle identity

| Field | Value |
|-------|-------|
| Slug | `all-MiniLM-L6-v2-en-1.0.0` |
| Schema version | 1 |
| Embedder family | `sentence-transformers` |
| Embedder model | `all-MiniLM-L6-v2` |
| Embedding dim | 384 |
| Quantization | `int8-dynamic` (CI release) ; placeholder zeros (dev) |
| Normalize | `l2` |
| Target size | ≤ 30 MiB on disk (release) |
| Pubkey id | `efe-release-2026` |

## 2. Files

The bundle directory contains exactly **3 artefacts** (post-CI-release) :

| File | Role |
|------|------|
| `weights.bin` | INT8-quantized model weights |
| `tokenizer.json` | HuggingFace tokenizer JSON |
| `manifest.json` | D16 schema v1 + per-file SHA-256 + total size |
| `manifest.sig` | Ed25519 detached signature over `manifest.json` (CI release only ; absent in dev mode) |

Dev mode (`--dev` flag) produces deterministic 32-byte placeholders
for `weights.bin` + a minimal valid JSON `tokenizer.json` so the
quickstart load path can be exercised without a 30 MiB pull.

## 3. Reproductibility (CI release)

```bash
# Build deterministically — same inputs → byte-identical manifest.json.
$ ./scripts/release/build_sample_bundle.py \
    --out bundles/all-MiniLM-L6-v2-en-1.0.0/ \
    --privkey-path /var/run/secrets/efe-release-key

BUNDLE_OK path=bundles/all-MiniLM-L6-v2-en-1.0.0 sha256_manifest=<HEX> ...

# Verify the manifest hash matches the published value.
$ sha256sum bundles/all-MiniLM-L6-v2-en-1.0.0/manifest.json
<HEX>  bundles/all-MiniLM-L6-v2-en-1.0.0/manifest.json
```

The manifest is canonical-JSON (sorted keys, 2-space indent,
trailing newline) so any two builds from the same inputs produce
**byte-identical** files — the auditor's `sha256sum` matches the
hash documented here in the GA release row of section 5.

## 4. Verifying the signature (post-load)

```bash
$ openssl pkeyutl -verify \
    -pubin -inkey /etc/efe/embedder_trust/efe-release-2026.pub \
    -rawin -in bundles/all-MiniLM-L6-v2-en-1.0.0/manifest.json \
    -sigfile <(jq -r .signature_base64 \
        bundles/all-MiniLM-L6-v2-en-1.0.0/manifest.sig | base64 -d)
Signature Verified Successfully
```

The SDK does this automatically on `LocalSTEmbedder.load_bundle()` ;
the manual command is for auditors + cross-platform CI verification.

## 5. Release table

| Version | Date | manifest.json SHA-256 | Mode | Notes |
|---------|------|-----------------------|------|-------|
| 1.0.0-dev0 | 2026-04-29 | `0d0c7d44e02360806092d448d64de60313649ac6ffefbc68c42d651da52a92e6` | DEV (placeholder) | First scaffold, 82-byte total. |
| 1.0.0    | TBD (CI release) | `<TO_BE_FILLED_BY_CI>` | RELEASE | INT8-quantized, ≤30 MiB. |

## 6. Quickstart — load this bundle

```python
from efe_tool import LocalSTEmbedder

# Loads + verifies the Ed25519 signature ; raises if mismatch.
embedder = LocalSTEmbedder.from_bundle(
    "bundles/all-MiniLM-L6-v2-en-1.0.0",
    trust_store="/etc/efe/embedder_trust.json",
)
print(embedder.dim)  # 384
```

## 7. Compliance pointer

The sample bundle is **deterministically reproducible** (DT-PC-2)
so SOC 2 / ISO 27001 auditors can independently verify the supply
chain :

1. The CI release pipeline checkout commit hash is recorded in
   `manifest.json:builder_version` + the git tag.
2. The upstream model weights (Hugging Face) are pinned to a SHA
   (recorded in the workflow run logs).
3. The Ed25519 signing key is held in an HSM (offline) ; only the
   CI release runner has access.
4. Every bundle artefact has its SHA-256 in `manifest.json` ; the
   manifest itself is signed.
5. Drift detection : any change to the bundle files without a
   corresponding `manifest.sig` regen → load-time signature
   verification fails → the SDK raises `EfeFacadeBundleError`
   (exit code 14, see `cli/errors.py`).

## 8. Pin tests

* `scripts/tests/test_d20_sample_bundle_present_and_signed.py`
* `scripts/tests/test_d20_sample_bundle_doc_present.py`
* `scripts/tests/test_d20_sample_bundle_reproducibility_check.py`

These tests execute on every PR ; a drift between this document
and the bundle on disk fails CI (anti-doc-drift).

## 9. References

* [`docs/EFE_PYTHON_SDK_EXTENSION_ARCHITECTURE.md`](../EFE_PYTHON_SDK_EXTENSION_ARCHITECTURE.md)
  — sprint D20 §13 (Sample bundle).
* [`scripts/release/build_sample_bundle.py`](../../scripts/release/build_sample_bundle.py)
  — the build script source.
* [`docs/bench/d16_embedder_bundle_architecture.md`](../bench/) —
  D16 baseline bundle format reference.
