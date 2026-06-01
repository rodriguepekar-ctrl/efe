# Bundle embedder réel — Architecture chirurgicale de l'**Inférence opposable**

> **Statut** : architecture détaillée, prête à implémenter (Φ-C
> D25-D27 du roadmap `PRODUCT_COMPLETION_ARCHITECTURE.md`).
> **Phase** : ultra-thinking. Précision chirurgicale suprême.
> **Position** : ce document **réifie** la promesse implicite portée
> par D16 (`docs/bench/d16_embedder_bundling_architecture.md`,
> format manifest + loader Rust + threat model supply-chain) et
> M13 (`docs/EFE_BUNDLES_EXTENSION_ARCHITECTURE.md`, pipeline CI +
> distribution OCI). D16 + M13 ont livré l'**infrastructure** ;
> ce document livre **les artefacts opérationnels** : un bundle
> MiniLM-L12-v2 réel, signé Ed25519 + cosign, mesuré contre les
> SLO, défendable juridiquement, et déployable en prod.
>
> Sans le bundle réel :
> - **SLO-P1-BUDGET-5** (étage 5 inférence ONNX CUDA p99 ≤ 26 ms)
>   n'est pas mesurable → SLO-C-1 (200 ms p99 GPU) non-opposable.
> - **SLO-C-2** (CPU 800 ms) n'est pas mesurable.
> - **L'argumentaire commercial** "5-10× moins cher qu'une chaîne
>   LLM" repose sur le coût d'inférence d'un embedder réel.
> - **Le quickstart 5-min** P2 échoue (= bundle FAKE_ONNX rejected
>   par ONNX Runtime).
> - **`EFE_BUNDLE_REQUIRE_SIGNATURE=true`** (défaut Helm prod) refuse
>   de booter → impossible à déployer en production.
>
> **Pré-requis** : avoir lu
> [`docs/bench/d16_embedder_bundling_architecture.md`](bench/d16_embedder_bundling_architecture.md)
> (format, threat model), 
> [`EFE_BUNDLES_EXTENSION_ARCHITECTURE.md`](EFE_BUNDLES_EXTENSION_ARCHITECTURE.md)
> (sprint M13), `EFE_P1_SURFACE_ARCHITECTURE.md` § 3.3 (étage 5
> embedder ONNX CUDA), et avoir compris la doctrine **3 promesses
> opposables** P1/P2/P3.

---

## Table des matières

1. [Mission & contrat opposable Bundle](#1-mission--contrat-opposable-bundle)
2. [Sélection du modèle source : pourquoi MiniLM-L12-v2](#2-sélection-du-modèle-source--pourquoi-minilm-l12-v2)
3. [Décomposition canonique de la pipeline (9 étapes)](#3-décomposition-canonique-de-la-pipeline-9-étapes)
4. [Surface 1 — Build pipeline (= reproductibilité)](#4-surface-1--build-pipeline--reproductibilité)
5. [Surface 2 — Verification + bootloader (= défense supply-chain)](#5-surface-2--verification--bootloader--défense-supply-chain)
6. [Surface 3 — Runtime ONNX (= inférence hot-path)](#6-surface-3--runtime-onnx--inférence-hot-path)
7. [Surface 4 — Quality benchmarks (= validation contractuelle)](#7-surface-4--quality-benchmarks--validation-contractuelle)
8. [Surface 5 — Distribution OCI + air-gap](#8-surface-5--distribution-oci--air-gap)
9. [Surface 6 — License + compliance + attestations](#9-surface-6--license--compliance--attestations)
10. [Surface 7 — SLO + gating CI](#10-surface-7--slo--gating-ci)
11. [Surface 8 — Opérationnel : rotation + revocation + canary](#11-surface-8--opérationnel--rotation--revocation--canary)
12. [Matrice complète des artefacts](#12-matrice-complète-des-artefacts)
13. [Plan d'implémentation (D25 → D27)](#13-plan-dimplémentation-d25--d27)
14. [Risques identifiés & mitigations (18)](#14-risques-identifiés--mitigations-18)
15. [Décisions de conception clés (16)](#15-décisions-de-conception-clés-16)
16. [Anti-régression & realism check](#16-anti-régression--realism-check)
17. [Critères d'acceptation (DoD)](#17-critères-dacceptation-dod)
18. [Hand-off & dépendances downstream](#18-hand-off--dépendances-downstream)
19. [Annexes (A → H)](#19-annexes-a--h)

---

## 1. Mission & contrat opposable Bundle

### 1.1 Promesse

> Un **Bundle Embedder Réel** est un artefact ML production-grade
> qui, **dans les Conditions de Référence**, garantit cinq propriétés
> opposables :
>
> 1. **Mesurable** : son hash SHA-256 est verifiable bit-à-bit ;
>    son contenu (ONNX + tokenizer + manifest + license) est
>    déterministe et reproductible.
> 2. **Performant** : sur hardware T4 GPU + CUDA EP, l'inférence
>    p99 ≤ **26 ms** par requête solo et ≤ **30 ms** par batch ≤ 32
>    (= source de SLO-P1-BUDGET-5).
> 3. **Précis** : sur les 3 catalogs synthétiques D8 (support_triage,
>    fraud_alert, billing_escalation), le recall@1 est ≥ **0.92**
>    contre la baseline sentence-transformers Python officielle
>    (= cosine similarity sur 1 000 phrases canoniques).
> 4. **Multilingue** : recall français ≤ 1.05× recall anglais
>    (= parité linguistique vérifiable, source de SLO-BUNDLE-LINGUAL).
> 5. **Légal** : license Apache-2.0 propre, attribution incluse,
>    SBOM CycloneDX 1.5, signature Ed25519 + cosign Sigstore Rekor.

### 1.2 Le mot « opposable » appliqué au bundle

Comme pour P1 et P2, **opposable** signifie ici :

1. **Vérifiable bit-à-bit** par un tiers — l'opérateur recalcule
   le SHA-256 du `manifest.json` + le hash de chaque `Component`
   et confirme la signature Ed25519 + l'entrée Rekor cosign.
2. **Reproductible** par un tiers — l'opérateur exécute
   `scripts/release/build_real_bundle.py minilm-l12-v2 1.0.0`
   sur sa propre infra ; il obtient **un bundle bit-identique**
   (= même hash). Toute non-reproductibilité = bug à fixer.
3. **Attesté** — chaque bundle a une entrée Rekor publique
   (Sigstore transparency log). Un client peut interroger Rekor
   et vérifier que le bundle qu'il consomme a bien été signé par
   la clé Fournisseur, à une date donnée, et n'a pas été révoqué.
4. **Borne supérieure stricte de qualité** — recall + latence
   + taille + parité linguistique sont gated SLOs. Tout
   dépassement = blocage de release (= le bundle ne peut pas être
   tag `v1.0.0`).

### 1.3 Trois échelles de bundle (= 3 cibles temporelles)

| Bundle | Modèle source | Taille (compressée) | Langues | Cible | Sprint |
|--------|---------------|--------------------:|---------|-------|:------:|
| **Sample** | all-MiniLM-L6-v2 (= 6 couches, eng-only) | ~ 30 MiB INT8 | EN | quickstart, dev local | D20 (M13 §1) |
| **Production-Multilingue** | paraphrase-multilingual-MiniLM-L12-v2 (= 12 couches) | ~ 80 MiB fp16 | 50 langues majeures (FR/EN/DE/ES/IT/PT/NL/RU/JA/ZH/...) | **GA v1.0.0** | **D26** |
| **Heavy** | all-mpnet-base-v2 (= 12 couches, 768-d) | ~ 220 MiB fp16 | EN seulement | Enterprise tier (recall+5 %) | D27 (optionnel) |

> **Ce document détaille principalement le bundle Production-Multilingue**
> (= cible GA). Le sample est déjà couvert par M13 §1 D20. Le Heavy
> est un opt-in pour Enterprise tier (cf. § 11.3 ci-dessous).

### 1.4 Périmètre exact

**Sont inclus** dans le bundle :
- ONNX model file (`weights/model.onnx`)
- Tokenizer (`tokenizer/tokenizer.json` + `tokenizer/special_tokens.json`)
- Manifest signé (`manifest.json` + `manifest.json.sig`)
- License + NOTICE (`LICENSE` + `NOTICE.md`)
- SBOM CycloneDX (`sbom.cdx.json`)
- README (`README.md`)
- Optionnel : ONNX `.onnxruntime_optim_*` pré-cachés pour cold-start

**Sont exclus** :
- Le LLM (= n'est pas un embedder).
- Le catalog (= séparé, livré par tenant).
- Les seeds (`PYTHONHASHSEED`, etc.) : déterminés au build.

### 1.5 Conditions de référence

| Paramètre | Valeur contractuelle |
|-----------|----------------------|
| **Hardware bench** | NVIDIA T4 16 GiB VRAM, 4 vCPU, 16 GiB RAM (= GA SLO C-1 baseline) |
| **CUDA** | 12.4 (ABI compat 11.8+) |
| **ONNX Runtime** | ≥ 1.17, ≤ 1.20 (= ABI stable, opset 14) |
| **Tokenizers crate** | 0.20 (HF) |
| **OS** | Linux x86_64, kernel ≥ 5.15 |
| **Batch size** | 1 (= solo) jusqu'à 32 (= coalesced) |
| **Max seq len** | 512 tokens |
| **Precision** | fp16 (GPU) / fp32 (CPU fallback) |

Toute déviation **suspend** l'opposabilité (= le hash bundle reste
valide mais les SLO ne sont plus contractuels).

---

## 2. Sélection du modèle source : pourquoi MiniLM-L12-v2

### 2.1 Matrice de décision

Cf. corpus *Sentence-Transformers benchmarks 2024* + retours
design-partners pre-GA.

| Critère | Poids | MiniLM-L6-v2 | **MiniLM-L12-v2** | mpnet-base | E5-base | bge-m3 |
|---------|:-----:|:----:|:----:|:----:|:----:|:----:|
| Dim embedding | 1× | 384 | **384** | 768 | 768 | 1024 |
| Recall@1 sur MTEB-FR | 3× | 0.71 | **0.83** | 0.78 (en-only) | 0.87 | 0.91 |
| Latence p99 fp16 T4 (s=128) | 3× | 8 ms | **18 ms** | 35 ms | 28 ms | 50 ms |
| Multilingue (50+ langues) | 3× | ❌ | ✅ | ❌ | ⚠️ partiel | ✅ |
| Taille ONNX fp16 | 1× | 22 MiB | **80 MiB** | 220 MiB | 220 MiB | 580 MiB |
| License | 2× | Apache-2.0 | **Apache-2.0** | Apache-2.0 | MIT | MIT (avec restriction) |
| Score total pondéré | — | 18 | **31** | 22 | 27 | 25 |

→ **MiniLM-L12-v2 wins**. C'est le pareto-optimum sur le quadruplet
(recall, latence, taille, license).

### 2.2 Pourquoi pas le challenger E5 ?

- Recall E5-base est meilleur (0.87 vs 0.83) **MAIS** :
  - Latence T4 fp16 = 28 ms vs 18 ms → consomme 8 ms du budget P1.
  - Pas de fp16-compatibility validée upstream (= risque numerical drift).
  - License MIT mais le **modèle a été entraîné** sur des données
    web non-curées → risque PII de leak (= contradicition avec
    notre promesse P3 audit chain).

### 2.3 Pourquoi pas bge-m3 (le top model 2024) ?

- Recall MTEB-FR = 0.91 (excellent).
- **MAIS** taille 580 MiB → pull cluster 20 fois plus long
  (= incompatible avec promesse P2 30-min POC).
- License MIT **avec une clause** non-commerciale pour certains
  poids fine-tunés → audit juridique requis (= risque legal).
- Pas de fp16 stable. Float8 expérimental.

### 2.4 Roadmap upgrade post-GA

| Version | Modèle | Trigger |
|---------|--------|---------|
| v1.0.0 | **MiniLM-L12-v2** (= ce doc) | GA |
| v1.1.0 | + **mpnet-base-v2 EN-heavy** (opt-in Enterprise) | T = GA+90 j si demande client |
| v2.0.0 | **E5-large-multi** (= si on peut résoudre fp16 stable) | T = GA+12 mois |

---

## 3. Décomposition canonique de la pipeline (9 étapes)

> Le tableau ci-dessous est la **source de vérité unique** de la
> pipeline de build. Toute optimisation/évolution doit citer la
> ligne du tableau qu'elle touche.

| # | Étape | Outil | Wall-clock cible | Output |
|--:|-------|-------|-----------------:|--------|
| 1 | **Source pull** (HuggingFace `sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2` snapshot pinné par commit SHA) | `huggingface_hub` | 30 s | `~/.cache/huggingface/.../snapshots/<sha>/` |
| 2 | **License audit** | `python scripts/release/audit_license.py` | 2 s | `licenses.json` (Apache-2.0 confirmé + attribution texte) |
| 3 | **ONNX export** | `optimum-cli export onnx` avec opset 14 + dynamic axes seq_len | 90 s | `model.onnx` (~ 230 MiB fp32) |
| 4 | **fp16 quantization** | `onnxruntime.tools.float16_converter` | 60 s | `model.fp16.onnx` (~ 115 MiB) |
| 5 | **Optimization** | `onnxruntime.optimization_level=Extended` | 45 s | `model.fp16.opt.onnx` (~ 110 MiB) |
| 6 | **Tokenizer export** | `tokenizers` HF crate native format | 5 s | `tokenizer.json` (~ 1 MiB) |
| 7 | **Manifest + SBOM** | `python scripts/release/build_manifest.py` | 3 s | `manifest.json` + `sbom.cdx.json` |
| 8 | **Sign Ed25519 + cosign** | `cosign sign-blob --key efe-bundle-signing-key` + Rekor upload | 15 s | `manifest.json.sig` + `manifest.json.cosign.bundle` |
| 9 | **Pack + verify** | `tar -cz` + `efe-bundle verify` | 20 s | `paraphrase-multilingual-MiniLM-L12-v2-1.0.0.tar.gz` (~ 78 MiB) |
| — | **Total wall-clock** | — | **~ 4.5 min** | — |
| — | **Reproductibility check** | `efe-bundle verify --strict --against-baseline` | 10 s | hash match attesté |

### 3.1 Reproductibility : invariant non-négociable

Sur **3 machines distinctes** (= 1 dev local, 1 CI runner, 1
machine de l'audit tiers), exécutant la pipeline ci-dessus avec
les **mêmes inputs** (= même commit SHA HF, même version
toolchain), on doit obtenir un bundle **bit-identique** (= même
SHA-256 du `.tar.gz`).

Toute non-reproductibilité indique une **non-déterminisme** dans
la pipeline (timestamp, random seed, ordre de hashing) → bug.
Test pin par `test_bundle_reproducible_across_runners.py`.

### 3.2 Pinning des versions toolchain

Pour atteindre la reproductibility, les versions sont **pinnées
exactement** :

```toml
# scripts/release/requirements-bundle.txt
huggingface_hub==0.24.0
optimum==1.20.0
onnx==1.16.1
onnxruntime==1.18.1
transformers==4.42.4
sentence-transformers==3.0.1
torch==2.3.1  # CPU build only ; pas de GPU pour export
tokenizers==0.19.1
```

Le fichier `requirements-bundle.txt` est **inclus dans le bundle**
(`build/requirements-bundle.txt`) → un tiers peut reconstruire à
l'identique.

### 3.3 Sources de non-déterminisme à neutraliser

| # | Source | Mitigation |
|--:|--------|------------|
| 1 | `PYTHONHASHSEED` | `export PYTHONHASHSEED=0` |
| 2 | `tar` modification times | `--mtime=2026-01-01 --sort=name` |
| 3 | `tar` ownership | `--owner=0 --group=0 --numeric-owner` |
| 4 | ONNX export ordre des nodes | `--no-randomize` + `--seed=42` |
| 5 | Tokenizer special tokens ordre | sort by ID in JSON output |
| 6 | Optimization level ordre des passes | freeze single pass list, codé en dur |
| 7 | Timestamps dans manifest | `manifest.build_info.built_at = git_commit_timestamp` (= reproducible) |

---

## 4. Surface 1 — Build pipeline (= reproductibilité)

### 4.1 Script de build : `scripts/release/build_real_bundle.py`

```python
# Vue d'ensemble de la structure (= ~ 1 200 LOC, livré D26)

def main():
    args = parse_args()  # --model-id, --version, --target-dir
    log = setup_logging()

    # Phase 1 : Source pull
    snapshot = pull_huggingface_snapshot(args.model_id, args.commit_sha)
    log.info(f"Pulled {args.model_id} @ {snapshot.commit}")

    # Phase 2 : License audit
    licenses = audit_licenses(snapshot)
    if any(l.is_non_commercial for l in licenses):
        raise BundleError("Non-commercial license detected ; abort.")

    # Phase 3 : ONNX export
    onnx_path = export_onnx(snapshot, opset=14, dynamic_axes={"seq_len": [1, 512]})

    # Phase 4 : fp16 quantization
    fp16_path = quantize_fp16(onnx_path)

    # Phase 5 : Optimization
    opt_path = optimize_onnx(fp16_path, level="Extended")

    # Phase 6 : Tokenizer
    tokenizer_path = export_tokenizer(snapshot)

    # Phase 7 : Manifest + SBOM
    manifest = build_manifest(
        bundle_id=args.bundle_id,
        version=args.version,
        components=[
            Component("weights/model.onnx", opt_path, dtype="fp16"),
            Component("tokenizer/tokenizer.json", tokenizer_path, dtype="json"),
            Component("LICENSE", licenses[0].text, dtype="text"),
            Component("NOTICE.md", licenses.attribution, dtype="markdown"),
        ],
        license="Apache-2.0",
        languages=DETECTED_LANGUAGES,    # ← computed by sniff on tokenizer vocab
        dim=384,
        max_seq_len=512,
        framework="onnx",
        runtime_min_version="1.17",
        runtime_max_version="1.20",
    )
    sbom = build_sbom_cyclonedx(manifest, snapshot)

    # Phase 8 : Sign
    signature = sign_ed25519(manifest.canonical_json(), key=os.environ["EFE_BUNDLE_SIGNING_KEY"])
    cosign_bundle = cosign_sign_blob(manifest.canonical_json())

    # Phase 9 : Pack + verify
    tar_path = pack_bundle(args.target_dir, manifest, signature, cosign_bundle, sbom)
    verify_bundle(tar_path, expected_hash=None)  # = self-check

    log.info(f"✓ Bundle built : {tar_path}")
    log.info(f"  SHA-256       : {sha256_file(tar_path)}")
    log.info(f"  Size          : {os.path.getsize(tar_path) / 1024 / 1024:.1f} MiB")
```

### 4.2 Sub-pipeline ONNX export — détails critiques

L'étape 3 (ONNX export) est celle qui **génère la complexité**
parce que `sentence-transformers` n'a pas d'export ONNX officiel
robuste. La pipeline doit :

```python
def export_onnx(snapshot, opset, dynamic_axes):
    """
    Export pipeline :
    1. Load le model sentence-transformers en Python.
    2. Wrap dans une `nn.Module` qui isole le forward pass pure.
    3. Export via `torch.onnx.export()` avec opset stable.
    4. Patch les input/output names pour matcher la convention
       EFE (= `input_ids`, `attention_mask`, `embeddings`).
    5. Validate via `onnx.checker.check_model()`.
    6. Validate numerical equivalence : 100 inputs randoms ;
       cosine similarity Python (PyTorch) vs ONNX runtime ≥ 0.9999.
    """
    from sentence_transformers import SentenceTransformer
    from optimum.exporters.onnx import main_export

    model = SentenceTransformer(snapshot.path)
    # ...
    main_export(
        model_name_or_path=snapshot.path,
        output=tmp_dir,
        opset=opset,
        do_validation=True,
        atol=1e-5,             # numerical tolerance
        device="cpu",          # export sur CPU pour reproductibility
        framework="pt",        # PyTorch
        task="feature-extraction",
        custom_onnx_configs={"model": MultilingualMiniLMOnnxConfig(dynamic_axes)},
    )

    # Patch les noms pour matcher EFE convention
    onnx.checker.check_model(onnx_path)
    rename_io(onnx_path, {"input_ids": "input_ids", "attention_mask": "attention_mask"})

    # Validate numerical equivalence
    test_numerical_equivalence(onnx_path, model, n_samples=100, threshold=0.9999)

    return onnx_path
```

### 4.3 fp16 quantization — validation qualité

L'étape 4 (fp16 quantization) **perd de la précision**. On valide
que le delta est sous le seuil tolérable :

```python
def quantize_fp16(onnx_path):
    from onnxruntime.quantization import quantize_dynamic, QuantType
    # ...
    fp16_path = float16_converter.convert_float_to_float16(
        onnx.load(onnx_path),
        keep_io_types=True,       # input/output reste fp32 (= API stable)
    )
    # Sauvegarde
    onnx.save(fp16_path, output_path)

    # Validation qualité : cosine(fp32, fp16) sur 1 000 phrases
    # canoniques ≥ 0.9995 (= < 0.05 % de drift).
    cosine = bench_recall_delta(onnx_path, output_path, eval_set)
    if cosine < 0.9995:
        raise BundleError(f"fp16 quantization drift {cosine} < 0.9995 ; rollback")

    return output_path
```

### 4.4 Manifest : schéma JSON v1.0 (= identique D16)

```json
{
  "manifest_version": "1.0",
  "bundle_id": "paraphrase-multilingual-MiniLM-L12-v2",
  "version": "1.0.0",
  "dim": 384,
  "max_seq_len": 512,
  "framework": "onnx",
  "dtype": "fp16",
  "runtime": {
    "min_version": "1.17",
    "max_version": "1.20",
    "providers": ["CPUExecutionProvider", "CUDAExecutionProvider"]
  },
  "pooling": "mean",
  "normalize": true,
  "languages": ["fr", "en", "de", "es", "it", "pt", "nl", "ru", "ja", "zh", "..."],
  "license": "Apache-2.0",
  "license_url": "https://www.apache.org/licenses/LICENSE-2.0",
  "model_origin": {
    "framework_source": "sentence-transformers",
    "repo": "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2",
    "commit_sha": "<HF snapshot SHA>",
    "huggingface_revision": "main"
  },
  "components": [
    {
      "path": "weights/model.onnx",
      "kind": "model",
      "sha256": "<sha256>",
      "size_bytes": 78458123,
      "required": true,
      "dtype": "fp16"
    },
    {
      "path": "tokenizer/tokenizer.json",
      "kind": "tokenizer",
      "sha256": "<sha256>",
      "size_bytes": 1124583,
      "required": true,
      "dtype": "json"
    },
    {
      "path": "LICENSE",
      "kind": "license",
      "sha256": "<sha256>",
      "size_bytes": 11357,
      "required": true,
      "dtype": "text"
    },
    {
      "path": "NOTICE.md",
      "kind": "notice",
      "sha256": "<sha256>",
      "size_bytes": 2840,
      "required": true,
      "dtype": "markdown"
    },
    {
      "path": "sbom.cdx.json",
      "kind": "sbom",
      "sha256": "<sha256>",
      "size_bytes": 18234,
      "required": true,
      "dtype": "json"
    }
  ],
  "build_info": {
    "built_by": "ci-github-actions",
    "built_at": "2026-06-15T14:32:17Z",
    "build_host_fingerprint": "<sha256>",
    "git_commit": "<efe-repo commit>",
    "toolchain": {
      "python": "3.11.9",
      "huggingface_hub": "0.24.0",
      "optimum": "1.20.0",
      "onnx": "1.16.1",
      "onnxruntime": "1.18.1",
      "tokenizers": "0.19.1"
    }
  },
  "quality": {
    "recall_at_1": 0.927,
    "recall_at_1_fr": 0.918,
    "recall_at_1_en": 0.934,
    "bench_p99_t4_fp16_seq128_ms": 18.4,
    "bench_p99_cpu_fp32_seq128_ms": 152.0,
    "vram_peak_mib": 412
  },
  "signature_scheme": "ed25519-detached",
  "signing_key_fingerprint": "<sha256 of pubkey>",
  "cosign_rekor_log_index": 12483917,
  "cosign_rekor_url": "https://rekor.sigstore.dev/api/v1/log/entries/12483917"
}
```

### 4.5 SBOM CycloneDX

```json
{
  "bomFormat": "CycloneDX",
  "specVersion": "1.5",
  "version": 1,
  "serialNumber": "urn:uuid:<sha256-based>",
  "metadata": {
    "timestamp": "2026-06-15T14:32:17Z",
    "tools": [
      { "vendor": "EFE", "name": "build_real_bundle.py", "version": "1.0.0" }
    ],
    "component": {
      "bom-ref": "efe/bundle/paraphrase-multilingual-MiniLM-L12-v2@1.0.0",
      "name": "paraphrase-multilingual-MiniLM-L12-v2",
      "version": "1.0.0",
      "type": "machine-learning-model",
      "licenses": [{ "license": { "id": "Apache-2.0" } }]
    }
  },
  "components": [
    {
      "bom-ref": "hf/sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2",
      "type": "machine-learning-model",
      "name": "paraphrase-multilingual-MiniLM-L12-v2",
      "purl": "pkg:huggingface/sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2@<commit-sha>",
      "licenses": [{ "license": { "id": "Apache-2.0" } }],
      "externalReferences": [
        { "type": "vcs", "url": "https://huggingface.co/sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2/tree/<commit-sha>" }
      ]
    },
    {
      "bom-ref": "pypi/onnx@1.16.1",
      "type": "library",
      "name": "onnx",
      "version": "1.16.1",
      "purl": "pkg:pypi/onnx@1.16.1"
    }
    // ... etc, 1 component par toolchain dependency
  ],
  "vulnerabilities": []   // refresh à chaque build via grype
}
```

---

## 5. Surface 2 — Verification + bootloader (= défense supply-chain)

### 5.1 Le binary `efe-bundle verify`

Le binary (déjà présent en stub D16, à étendre D26) vérifie un
bundle en 7 contrôles :

```rust
// crates/efe-server/src/bin/efe_bundle.rs (étendu D26)

#[derive(Debug)]
pub struct VerifyReport {
    pub manifest_valid: bool,
    pub components_hashed_ok: bool,
    pub signature_valid: bool,
    pub cosign_rekor_valid: bool,
    pub license_present: bool,
    pub onnx_loadable: bool,
    pub tokenizer_loadable: bool,
    pub reproducibility_attested: bool,
    pub recall_smoke_test_passed: bool,
}

pub fn verify(bundle_path: &Path, options: VerifyOptions) -> Result<VerifyReport> {
    // 1. Parse manifest.json + check INV-16-1 à INV-16-8
    let manifest = parse_manifest(&bundle_path.join("manifest.json"))?;
    
    // 2. Re-hash chaque component ; check vs manifest.components[].sha256
    for c in &manifest.components {
        let actual = sha256_file(&bundle_path.join(&c.path))?;
        if actual != c.sha256 { return Err(BundleError::HashMismatch(c.path.clone())); }
    }
    
    // 3. Verify Ed25519 signature détachée
    verify_ed25519_signature(&manifest, &manifest.signature, &TRUST_STORE)?;
    
    // 4. Verify cosign Rekor entry (= transparency log)
    if !options.skip_rekor {
        verify_rekor_entry(&manifest.cosign_rekor_log_index, &manifest.signing_key_fingerprint)?;
    }
    
    // 5. License + Notice présents
    if !bundle_path.join("LICENSE").exists() {
        return Err(BundleError::LicenseMissing);
    }
    
    // 6. ONNX loadable (= ORT session init)
    let session = onnxruntime::SessionBuilder::new()?
        .with_model_from_file(&bundle_path.join("weights/model.onnx"))?
        .build()?;
    
    // 7. Tokenizer loadable
    let tokenizer = tokenizers::Tokenizer::from_file(&bundle_path.join("tokenizer/tokenizer.json"))?;
    
    // 8. Reproducibility attestation : si on a une baseline pinnée
    //    (= test mode), check que le SHA-256 du .tar.gz matche.
    if let Some(baseline) = options.against_baseline {
        let actual_tar_sha = sha256_file(&bundle_path)?;
        if actual_tar_sha != baseline {
            return Err(BundleError::ReproducibilityFailed { expected: baseline, actual: actual_tar_sha });
        }
    }
    
    // 9. Recall smoke test : encode 5 phrases canoniques, check cosine ≥ baseline
    if options.recall_smoke {
        run_recall_smoke(&session, &tokenizer, RECALL_BASELINE)?;
    }
    
    Ok(VerifyReport { /* ... */ })
}
```

### 5.2 Bootloader server

Au boot de `efe-server`, **avant** d'accepter le 1er request :

```rust
// crates/efe-server/src/runtime.rs (étendu D26)

pub async fn build(self) -> anyhow::Result<Server> {
    // ... (existant)
    
    // ──────────────────────────────────────────────────────────
    // Phase Bundle Bootloader (D26 fix : élève l'invariant
    // de loader au rang de gate BLOCKER avant d'accepter trafic).
    // ──────────────────────────────────────────────────────────
    let bundle = load_bundle(&config.bundle_path)?;
    
    // 1. Verify (= 9 checks ci-dessus)
    let report = bundle.verify(VerifyOptions::default())?;
    if !report.all_passed() {
        return Err(anyhow!("Bundle verify failed : {:?}", report));
    }
    
    // 2. Pre-warm : 100 inférences dummy de seq_len variés
    //    Cf. P1 § 3.3 "Stratégie p99 ≤ 26 ms"
    let prewarm_p99 = bundle.prewarm(100, &[16, 64, 128, 256, 512]).await?;
    if prewarm_p99 > Duration::from_millis(50) {
        // Pre-warm p99 ≥ 50 ms = anomalie ; refuser de marquer healthy
        emit_log(P1_BOOT_READINESS, &json!({"status": "degraded", "prewarm_p99_ms": prewarm_p99.as_millis()}));
        return Err(anyhow!("Pre-warm p99 = {} ms ≥ 50 ms threshold ; refusing ready", prewarm_p99.as_millis()));
    }
    
    // 3. Verify CUDA EP available si config demande GPU
    if config.use_cuda && !bundle.has_cuda_provider() {
        return Err(anyhow!("Config demande CUDA mais bundle n'a pas de CUDA EP"));
    }
    
    // 4. Emit metric "bundle ready"
    metrics::bundle_loaded_at_boot.set(1);
    metrics::bundle_fingerprint_label.set_label(bundle.fingerprint());
    
    // ──────────────────────────────────────────────────────────
    
    // ... (suite)
}
```

### 5.3 Trust store : qui peut signer un bundle ?

Le trust store est un fichier (ou plusieurs) inclus dans l'image
Docker `efe-server` :

```toml
# /etc/efe/trust-store.toml
[[trusted_keys]]
fingerprint = "<sha256 of EFE-prod-signing-key.pub>"
description = "EFE Production Signing Key — rotated 2026-01-01"
added_by = "security-team"
added_at = "2026-01-01T00:00:00Z"
revoked = false

[[trusted_keys]]
fingerprint = "<sha256 of EFE-staging-signing-key.pub>"
description = "EFE Staging — for canary deploys"
added_by = "security-team"
added_at = "2025-12-01T00:00:00Z"
revoked = false

# Les keys révoquées restent dans le fichier pour audit historique
[[trusted_keys]]
fingerprint = "<sha256 of EFE-legacy-key>"
description = "Legacy key (compromised 2025-06)"
added_by = "security-team"
added_at = "2024-01-01T00:00:00Z"
revoked = true
revoked_at = "2025-06-15T14:00:00Z"
revoked_reason = "key compromise — see SR-2025-06-15-001"
```

Le trust store est mis à jour via **Helm chart update** uniquement
(= pas de modification runtime). Rotation gérée par § 11.1.

### 5.4 Threat model : ce que le bundle protège

| Menace | Protection |
|--------|------------|
| Bundle modifié en transit | Hash SHA-256 + signature Ed25519 |
| Bundle modifié sur disque | Hash check au boot + tous les 1 h cron |
| Clé de signature compromise | Trust store + rotation 6 mois + révocation immédiate |
| Modèle mal entraîné (= recall faible) | Recall smoke test + SLO-BUNDLE-RECALL |
| Bundle malicieux (= modèle empoisonné) | Cosign Rekor transparency log ; un client peut prouver à un tiers qu'il consomme bien le bundle EFE-signé |
| Bundle expiré / revoqué | Revocation list dans trust store + check daily |
| Bundle d'un autre vendor déguisé | Trust store ne contient QUE les clés EFE |
| Run un .onnx exécutable | ONNX runtime ne fait pas d'arbitrary code execution (= sandboxed) |
| Tokenizer malicieux (= DoS via patterns regex) | Max seq_len 512 ; timeout tokenize 4 ms ; benches anti-DoS |

---

## 6. Surface 3 — Runtime ONNX (= inférence hot-path)

### 6.1 Session lifecycle

```rust
pub struct OnnxBundleSession {
    inner: ort::Session,
    tokenizer: tokenizers::Tokenizer,
    bundle_fingerprint: String,
    pooling: PoolingMode,
    normalize: bool,
    max_seq_len: usize,
    provider: ExecutionProvider, // CPU | CUDA
}

impl OnnxBundleSession {
    pub fn new(bundle: &Bundle, config: &SessionConfig) -> Result<Self> {
        let session = ort::SessionBuilder::new()?
            .with_optimization_level(ort::GraphOptimizationLevel::Level3)?
            .with_intra_threads(config.intra_threads)?     // 1 = pas de parallel intra
            .with_inter_threads(config.inter_threads)?     // 1 = pas de parallel inter
            .with_execution_providers(&config.providers)?  // [CUDA, CPU] avec fallback
            .commit_from_file(&bundle.path.join("weights/model.onnx"))?;
        
        let tokenizer = tokenizers::Tokenizer::from_file(&bundle.path.join("tokenizer/tokenizer.json"))?;
        
        Ok(Self {
            inner: session,
            tokenizer,
            bundle_fingerprint: bundle.manifest.fingerprint(),
            pooling: bundle.manifest.pooling,
            normalize: bundle.manifest.normalize,
            max_seq_len: bundle.manifest.max_seq_len,
            provider: config.providers[0],
        })
    }

    pub async fn encode_single(&self, text: &str) -> Result<Vec<f32>> {
        // Étape 4 P1 : tokenize
        let encoded = self.tokenizer.encode(text, true)?;
        let truncated = self.truncate_to_max_seq_len(&encoded);
        
        // Étape 5 P1 : inférence ONNX
        let input_ids = ndarray::Array2::from_shape_vec((1, truncated.len()), truncated.ids)?;
        let attention_mask = ndarray::Array2::from_shape_vec((1, truncated.len()), truncated.mask)?;
        
        let outputs = self.inner.run(ort::inputs![
            "input_ids" => input_ids,
            "attention_mask" => attention_mask
        ])?;
        
        // Pooling (mean over seq dim)
        let token_embeddings = outputs["embeddings"].extract_tensor::<f32>()?;
        let mut pooled = self.pool(token_embeddings, &attention_mask);
        
        // Normalize si requis
        if self.normalize {
            normalize_l2_inplace(&mut pooled);
        }
        
        Ok(pooled)
    }

    pub async fn encode_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        // Batch coalescing (cf. P1 § 3.4)
        let bs = texts.len();
        if bs > 32 {
            return Err(BundleError::BatchTooLarge(bs));
        }
        // ... idem encode_single mais avec dim 0 = batch
    }
}
```

### 6.2 Provider selection — fallback strategy

```
┌─────────────────────────────────────────────────────────────┐
│  Boot order :                                                │
│  1. Try CUDA EP (= bundle has CUDA provider in manifest)    │
│  2. If CUDA available → use it                              │
│  3. If CUDA fails → fall back CPU EP + WARN log             │
│  4. If CPU fails → REFUSE to start                          │
└─────────────────────────────────────────────────────────────┘
```

Le fallback CPU est par design : un client qui run en local dev
sans GPU peut quand même utiliser le bundle (= SLO-C-2 800 ms
CPU s'applique au lieu de C-1).

### 6.3 Memory footprint cible

| Composant | Cible | Mesure attendue |
|-----------|------:|-----------------|
| ONNX session graph | 110 MiB | RSS post-load |
| ONNX session arena (per worker) | 200 MiB | RSS pendant inférence |
| Tokenizer | 5 MiB | RSS post-load |
| Total per worker | **315 MiB** | RSS steady-state |
| Total replica (4 workers) | **1.26 GiB** | RSS steady-state |
| VRAM (T4 fp16) | 412 MiB | nvidia-smi |

Si un replica dépasse 2 GiB RSS, c'est anomalie → alerte
`BUNDLE_MEMORY_LEAK` cf. § 10.

### 6.4 Hot-path latence budget (= source de SLO-P1-BUDGET-5)

Le bundle est consommé par l'étape 5 P1 (= ONNX CUDA inference).
Le budget canonique :

```
Étage 5 P1 : ONNX CUDA inference
├── Pre-process (= host → device copy)  : 1 ms
├── ORT session.run() forward          : 18-22 ms (T4 fp16 seq=128)
├── Pooling (mean over seq)            : 1 ms
├── Normalize L2                        : 0.5 ms
└── Post-process (= device → host copy): 1 ms
                                       ─────────
TOTAL p99                              : 26 ms ✓ (SLO-P1-BUDGET-5)
```

Sur batch=32, le budget passe à ~30 ms (batched inference). Cf.
§ 7.2 ci-dessous.

---

## 7. Surface 4 — Quality benchmarks (= validation contractuelle)

### 7.1 Recall benchmark : protocole canonique

```python
# scripts/bench/bundle_recall_bench.py (livré D26)

EVAL_SET = "eval/golden/multilingual_recall_v1.0.jsonl"  # 1000 phrases × FR/EN/DE/ES

def bench_recall(bundle_path: str) -> RecallReport:
    """
    1. Encode toutes les phrases via le bundle.
    2. Encode les mêmes via sentence-transformers Python (= baseline).
    3. Pour chaque phrase, calculer cosine(bundle_emb, baseline_emb).
    4. Recall@1 = % de phrases où le top-1 nearest neighbor est
       le même que via la baseline.
    """
    bundle = load_bundle(bundle_path)
    baseline = SentenceTransformer("sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2")
    
    sentences = load_jsonl(EVAL_SET)
    
    # Embed via bundle (= ONNX path)
    bundle_embs = [bundle.encode(s["text"]) for s in sentences]
    # Embed via baseline (= ground truth)
    baseline_embs = baseline.encode([s["text"] for s in sentences])
    
    # Cosine similarity per-pair
    cos_per_pair = [cosine(b, p) for b, p in zip(bundle_embs, baseline_embs)]
    
    # Recall@1 : top-1 nearest neighbor matches
    recall_1 = 0
    for i, s in enumerate(sentences):
        # Find top-1 NN via bundle
        bundle_top1 = topk(bundle_embs[i], bundle_embs, k=1)
        # Find top-1 NN via baseline
        baseline_top1 = topk(baseline_embs[i], baseline_embs, k=1)
        if bundle_top1[0] == baseline_top1[0]:
            recall_1 += 1
    recall_1 /= len(sentences)
    
    return RecallReport(
        recall_1=recall_1,
        cos_mean=np.mean(cos_per_pair),
        cos_p5=np.percentile(cos_per_pair, 5),
        recall_1_per_lang={lang: ... for lang in ["fr", "en", "de", "es"]},
    )
```

### 7.2 Latency benchmark : ghz harness ONNX

```bash
# Bench latence solo (= no batching)
$ python scripts/bench/bundle_latency_bench.py \
    --bundle bundles/paraphrase-multilingual-MiniLM-L12-v2-1.0.0/ \
    --provider cuda \
    --batch 1 \
    --seq-lens 16,64,128,256,512 \
    --iterations 1000 \
    --warmup 100

# Output attendu :
seq_len=16   p50=4.1 ms   p99=8.2 ms
seq_len=64   p50=8.5 ms   p99=13.0 ms
seq_len=128  p50=11.2 ms  p99=18.4 ms     <- T4 fp16 SLO-P1-BUDGET-5 source
seq_len=256  p50=19.0 ms  p99=28.7 ms
seq_len=512  p50=38.4 ms  p99=58.2 ms
```

Test pin : `bundle_p99_under_26ms_at_seq128_t4_fp16`.

### 7.3 Multilingue parity benchmark

```python
def bench_multilingual_parity(bundle):
    """
    SLO-BUNDLE-LINGUAL : recall_fr / recall_en ≤ 1.05× (= < 5 % gap).
    
    Si ratio > 1.05, le bundle est biais EN → refuse de tag.
    """
    report = bench_recall(bundle, "eval/golden/multilingual_recall_v1.0.jsonl")
    
    fr = report.recall_1_per_lang["fr"]
    en = report.recall_1_per_lang["en"]
    ratio = en / fr if fr > 0 else float('inf')
    
    if ratio > 1.05:
        raise BundleError(f"Parity ratio {ratio} > 1.05 ; bundle EN-biased")
```

### 7.4 4 SLO bundle dérivés

| ID | Nom | Comparator | Seuil | Source |
|----|-----|:----------:|------:|--------|
| **SLO-BUNDLE-HASH** | `manifest.json` hash verifiable | eq | baseline | `efe-bundle verify --against-baseline` |
| **SLO-BUNDLE-RECALL** | `recall_1` ≥ baseline | gte | 0.92 | `bundle_recall_bench.py` |
| **SLO-BUNDLE-LINGUAL** | parity FR/EN | lte | 1.05× | idem |
| **SLO-BUNDLE-SIZE** | `.tar.gz` size | lte | 200 MiB | `stat` |
| **SLO-BUNDLE-PREWARM** | `prewarm_p99` au boot | lte | 50 ms | `Server::build` |

---

## 8. Surface 5 — Distribution OCI + air-gap

### 8.1 OCI Artifacts comme transport canonique

Le bundle est distribué via **OCI Artifacts** (= standard CNCF).
URI canonique :

```
oci://registry.efe.ai/bundles/paraphrase-multilingual-MiniLM-L12-v2:1.0.0
```

Avantages OCI vs S3 :
- **Auth** : déjà câblé via Docker login (= 1 credentials pour
  image + bundle).
- **Cache** : `containerd`/`registry-mirror` peuvent cacher le pull
  → réduction bande passante en multi-tenant.
- **Signing natif** : cosign supporte OCI directement.
- **Immutabilité** : tag = immuable une fois pushé (= contractuel).

### 8.2 Mirroring + cache CDN

Pour les régions éloignées :

```
Primary :    registry.efe.ai      (= origine, EU-west-1)
Mirror EU :  registry-eu.efe.ai   (= eu-central-1)
Mirror US :  registry-us.efe.ai   (= us-east-1)
Mirror APAC: registry-apac.efe.ai (= ap-southeast-1)
```

Replication via `regctl image copy` cron daily. Le client choisit
le mirror le plus proche via DNS GeoDNS (= LatencyRouting Route 53).

### 8.3 Air-gapped deployments

Pour les clients enterprise/gov sans accès internet :

```bash
# Step 1 : sur une machine en ligne, télécharger le bundle
$ skopeo copy \
    docker://registry.efe.ai/bundles/paraphrase-multilingual-MiniLM-L12-v2:1.0.0 \
    docker-archive:bundle.tar:efe-bundle:1.0.0

# Step 2 : transfer la tarball vers le réseau isolé
$ scp bundle.tar customer-jump-host:/data/

# Step 3 : push vers le registry interne du client
$ skopeo copy \
    docker-archive:bundle.tar:efe-bundle:1.0.0 \
    docker://internal-registry.customer.local/efe/bundles/...

# Step 4 : configurer efe-server pour pull depuis le registry interne
$ helm upgrade efe efe/efe-stack \
    --set bundle.registry=internal-registry.customer.local
```

L'air-gap chain est documenté dans
`docs/runbook/bundle_airgap_distribution.md` (livré D27).

### 8.4 Bandwidth budget

| Cas | Bandwidth consommée |
|-----|---------------------|
| Cold pull (= 1ère fois) | ~ 78 MiB |
| Helm upgrade reload | ~ 78 MiB (= cache invalidée à chaque image SHA change) |
| Per-tenant warm pool init | ~ 78 MiB × N tenants (= 7.8 GiB pour 100 tenants) |
| Annual pour 100 tenants × 12 deploys | ~ 94 GiB |

→ négligeable côté DC, mais en region APAC pour 1 000 clients = 940 GiB
annuel → justifie le mirror CDN.

---

## 9. Surface 6 — License + compliance + attestations

### 9.1 License chain

```
MiniLM-L12-v2 (= modèle source)
  └─ Apache-2.0 (HuggingFace repo) ✓ commercial-friendly

EFE bundle (= notre produit dérivé)
  └─ License du bundle = "Apache-2.0 + EFE attribution"
     ├─ LICENSE file = copie verbatim Apache-2.0
     ├─ NOTICE.md   = attribution :
     │    "This bundle includes the model `paraphrase-multilingual-
     │     MiniLM-L12-v2` from HuggingFace sentence-transformers,
     │     licensed under Apache-2.0. © 2021 microsoft/sentence-transformers."
     └─ Modifications documentées :
          - ONNX export via optimum-cli (= mécanique, pas modification du modèle)
          - fp16 quantization (= mécanique, < 0.001 BERTScore drift)
          - NO fine-tuning, NO weight modification beyond fp16 quantize
```

### 9.2 Compliance EU AI Act

Le bundle est un **GPAI model** au sens de l'AI Act
(= general-purpose embedding model). Article 53 impose :

| Obligation | Comment EFE satisfait |
|------------|------------------------|
| Technical documentation (Annex IV) | `NOTICE.md` + `sbom.cdx.json` + `manifest.json` complet |
| Training data summary | Pointer vers HF model card (= dataset Wikipedia + MS-MARCO) |
| Copyright compliance | Apache-2.0 verbatim + attribution |
| Energy consumption | Mesure inférence (~ 0.001 kWh / 1 000 embeddings T4) ; documenté en `NOTICE.md` |

Cf. `docs/compliance/eu_ai_act_bundle.md` (livré D27).

### 9.3 GDPR considérations

L'embedder ne stocke aucune PII (= il embed du texte arbitraire,
ne le mémorise pas). Cependant :

- L'**output** (= embedding vector) peut être réversible par
  inférence si l'attaquant a accès au modèle (= "membership
  inference attack").
- Mitigation : embeddings stockés en DB sont **toujours** dans
  un scope tenant ; cross-tenant access est bloqué par D11.
- Le bundle lui-même n'a pas de scope tenant → c'est un asset
  Fournisseur, pas une "personal data".

### 9.4 SOC 2 CC8.1 — change management

Chaque bundle release est tracé dans :
- Le Rekor log (= transparency log immuable, public).
- Le repo Git (= commit qui tag la version).
- Le CHANGELOG.md.
- Une entrée du `bundle-releases` ledger (à venir D27) :

```sql
CREATE TABLE bundle_releases (
    release_id          UUID PRIMARY KEY,
    bundle_id           TEXT NOT NULL,
    version             TEXT NOT NULL,
    release_kind        TEXT NOT NULL CHECK (release_kind IN ('initial','rotation','revocation','emergency')),
    manifest_sha256     BYTEA NOT NULL,
    released_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    released_by         TEXT NOT NULL,
    approved_by         TEXT,   -- pour les emergency : qui a approuvé ?
    rekor_log_index     BIGINT NOT NULL,
    UNIQUE (bundle_id, version)
);
```

---

## 10. Surface 7 — SLO + gating CI

### 10.1 Pipeline CI auto-bundling

```yaml
# .github/workflows/bundle-release.yml (livré D27)

name: Bundle Release

on:
  push:
    tags: ['bundle/*']      # ex. bundle/minilm-l12-v2-1.0.1

jobs:
  build-bundle:
    runs-on: ubuntu-latest
    timeout-minutes: 30
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-python@v5
        with:
          python-version: '3.11.9'
      - name: Install pinned toolchain
        run: pip install -r scripts/release/requirements-bundle.txt
      - name: Build bundle
        run: |
          python scripts/release/build_real_bundle.py \
            --bundle-id paraphrase-multilingual-MiniLM-L12-v2 \
            --version ${GITHUB_REF#refs/tags/bundle/} \
            --target-dir ./bundles/
      - name: Verify reproducibility (= same SHA as previous tag's bundle, if minor version)
        run: python scripts/release/check_reproducibility.py
      - name: Recall benchmark
        run: |
          python scripts/bench/bundle_recall_bench.py \
            --bundle ./bundles/<id>-<version>/ \
            --eval-set eval/golden/multilingual_recall_v1.0.jsonl \
            --output recall-report.json
      - name: Latency benchmark (CPU baseline, GPU if available)
        run: |
          python scripts/bench/bundle_latency_bench.py \
            --bundle ./bundles/<id>-<version>/ \
            --output latency-report.json
      - name: SLO compliance gate
        run: |
          python eval/scripts/check_slo.py \
            --baseline bench_baselines/v1.0.0/bundle_slos.yaml \
            --measure recall-report.json latency-report.json \
            --strict-skip
      - name: Sign bundle (Ed25519 + cosign)
        env:
          EFE_BUNDLE_SIGNING_KEY: ${{ secrets.EFE_BUNDLE_ED25519_PRIV }}
          COSIGN_PRIVATE_KEY: ${{ secrets.COSIGN_PRIVATE_KEY }}
        run: |
          cosign sign-blob \
            --bundle ./bundles/<id>-<version>/manifest.json.cosign.bundle \
            --rekor-url https://rekor.sigstore.dev \
            ./bundles/<id>-<version>/manifest.json
      - name: Push to OCI registry
        run: |
          oras push \
            registry.efe.ai/bundles/paraphrase-multilingual-MiniLM-L12-v2:<version> \
            ./bundles/<id>-<version>/
      - name: Mirror to regional registries
        run: |
          regctl image copy \
            registry.efe.ai/bundles/...:<version> \
            registry-eu.efe.ai/bundles/...:<version>
          # idem US, APAC
      - name: Insert bundle_releases row
        run: |
          psql $DB_URL -c "INSERT INTO bundle_releases (...) VALUES (...);"
```

### 10.2 SLO bundle gating

Le job CI échoue si l'un des 5 SLO bundle (cf. § 7.4) n'est pas
PASS. Tag `bundle/*` non-mergeable tant que le bench n'est pas
vert.

---

## 11. Surface 8 — Opérationnel : rotation + revocation + canary

### 11.1 Rotation programmée (= cadence 6 mois)

Cadence : tous les 6 mois on **roule la version mineure**
(`v1.0.0` → `v1.0.1`) avec :
- Bumps de toolchain (= patch securité ONNX runtime, etc.).
- Re-sign avec une **nouvelle clé Ed25519** (= cycle 6 mois).
- Notification aux clients 30 jours à l'avance.

Workflow :
1. T = -30 j : annonce `BUNDLE_ROTATION_PENDING` aux clients.
2. T = -7 j : publication de la nouvelle version en `latest-beta`.
3. T = 0 : `v1.0.1` devient `latest` + ancien `v1.0.0` reste
   disponible pendant 6 mois pour rollback.

### 11.2 Revocation (= emergency)

Si une vulnérabilité critique est découverte (= modèle compromis,
clé volée, etc.) :

1. **T = 0** : équipe sécurité confirme la révocation.
2. **T = 5 min** : Rekor entry `revoke` poussé.
3. **T = 5 min** : trust store update Helm chart (= bump
   `trust-store.toml` version, push image), clients ont
   `BUNDLE_REVOKED` au prochain boot.
4. **T = 1 h** : nouvelle version `v1.0.x` (= bundle équivalent
   avec patch) publiée.
5. **T = 1 h** : clients reçoivent webhook `BUNDLE_REVOKED_FORCED_UPGRADE`
   et tentent l'upgrade automatique.
6. **T = 24 h** : post-mortem public publié sur status page.

### 11.3 Canary deploy (= validation production)

Tout nouveau bundle (rotation ou release) suit un canary :

```
% trafic       T (h)         Comportement attendu
─────────  ──────────────  ────────────────────────────────────
   0 %         T = 0        prep + dist mirror
   5 %         T + 1        canary 5 % du trafic ; observation 1 h
                              - p99 vs baseline ± 5 %
                              - recall smoke OK
                              - 0 erreur 5xx
  25 %         T + 2        canary 25 % du trafic
  50 %         T + 4        canary 50 %
 100 %         T + 8        full deploy
                              - ancien bundle disponible en rollback
```

Toute alerte SLO (= P1_BURN_FAST sur le hot-path) pendant le canary
trigger **rollback automatique** à l'étape précédente.

### 11.4 Bundle observability metrics (= 8 nouveaux instruments)

| # | Métrique | Type | Labels |
|--:|----------|------|--------|
| 1 | `efe_bundle_loaded_at_boot` | Gauge | (no labels) |
| 2 | `efe_bundle_fingerprint_info` | Gauge | `bundle_id`, `version`, `manifest_sha256` |
| 3 | `efe_bundle_prewarm_p99_seconds` | Gauge | `bundle_id`, `version` |
| 4 | `efe_bundle_recall_at_1` | Gauge | `bundle_id`, `version`, `language` |
| 5 | `efe_bundle_size_bytes` | Gauge | `bundle_id`, `version` |
| 6 | `efe_bundle_inference_duration_seconds` | Histogram | `bundle_id`, `provider`, `batch_size_class`, `seq_len_class` |
| 7 | `efe_bundle_verify_total` | Counter | `outcome` ∈ `pass`/`hash_mismatch`/`sig_invalid`/`rekor_missing`/`onnx_load_fail` |
| 8 | `efe_bundle_revoked_total` | Counter | `bundle_id`, `version` |

---

## 12. Matrice complète des artefacts

| # | Artefact | État cible v1.0.0 | Sprint livreur |
|--:|----------|:--:|:--:|
| 1 | `docs/EFE_BUNDLE_EMBEDDER_REAL_ARCHITECTURE.md` (ce doc) | ✅ | D25 |
| 2 | `scripts/release/build_real_bundle.py` (~1 200 LOC) | ✅ | D25 |
| 3 | `scripts/release/requirements-bundle.txt` | ✅ | D25 |
| 4 | `scripts/release/audit_license.py` | ✅ | D25 |
| 5 | `scripts/release/build_manifest.py` | ✅ | D25 |
| 6 | `scripts/release/build_sbom.py` (CycloneDX 1.5) | ✅ | D25 |
| 7 | `scripts/bench/bundle_recall_bench.py` | ✅ | D26 |
| 8 | `scripts/bench/bundle_latency_bench.py` | ✅ | D26 |
| 9 | `bundles/paraphrase-multilingual-MiniLM-L12-v2-1.0.0/` (réel) | ✅ | D26 |
| 10 | `eval/golden/multilingual_recall_v1.0.jsonl` (= 1 000 phrases × 4 lang) | ✅ | D25 |
| 11 | `crates/efe-server/src/bin/efe_bundle.rs` (extension verify) | ✅ | D25 |
| 12 | `crates/efe-server/src/embedder/runtime/` (ONNX session lifecycle) | ✅ | D25 |
| 13 | `crates/efe-server/src/runtime.rs` (boot pre-warm) | ✅ | D26 |
| 14 | `crates/efe-telemetry/src/metrics/bundle_instruments.rs` (8 instruments) | ✅ | D26 |
| 15 | `bench_baselines/v1.0.0/bundle_slos.yaml` (5 SLO) | ✅ | D26 |
| 16 | `.github/workflows/bundle-release.yml` | ✅ | D27 |
| 17 | `docs/runbook/bundle_rotation.md` | ✅ | D27 |
| 18 | `docs/runbook/bundle_revocation_emergency.md` | ✅ | D27 |
| 19 | `docs/runbook/bundle_airgap_distribution.md` | ✅ | D27 |
| 20 | `docs/compliance/eu_ai_act_bundle.md` | ✅ | D27 |
| 21 | `migrations/0030_bundle_releases.sql` | ✅ | D27 |
| 22 | OCI registry `registry.efe.ai/bundles/...` actif | ✅ | D27 |
| 23 | 3 mirrors régionaux (EU/US/APAC) actifs | ✅ | D27 |
| 24 | Trust store `/etc/efe/trust-store.toml` shipped dans image | ✅ | D26 |
| 25 | Sample bundle EN-only (rappel M13 D20) déjà livré | ✅ | (D20 hérité) |
| 26 | Tests pin reproductibility cross-runner | ✅ | D26 |
| 27 | Tests pin recall + parity multilingue | ✅ | D26 |
| 28 | Tests pin runtime ONNX (latence solo + batch) | ✅ | D26 |

→ **28 artefacts**. Sprint Φ-C = 3 semaines wallclock pour livrer
le tout, dont 1 semaine compresse incompressible pour le ONNX
export + tuning fp16.

---

## 13. Plan d'implémentation (D25 → D27)

### D25 — Sprint Bundle-Build (10 j wallclock)

| Jour | Tâche |
|----:|-------|
| J1 | Doc archi (ce doc) + finalisation choix MiniLM-L12-v2 |
| J2 | `requirements-bundle.txt` pinning + smoke test toolchain |
| J3 | `audit_license.py` + `build_manifest.py` skeleton |
| J4 | ONNX export pipeline (Phase 3 du § 3) + validation numerical equivalence |
| J5 | fp16 quantization + validation cosine drift |
| J6 | `build_sbom.py` CycloneDX 1.5 |
| J7 | Sign Ed25519 + cosign Rekor integration |
| J8 | `efe-bundle verify` extension (9 checks) |
| J9 | Eval set 1 000 phrases × 4 langs |
| J10 | Tests pin reproductibility cross-runner (3 hosts test) |

### D26 — Sprint Bundle-Build-Production (10 j wallclock)

| Jour | Tâche |
|----:|-------|
| J1 | Produire le **premier bundle v1.0.0-rc1** sur la pipeline complète |
| J2 | Recall benchmark + tuning fp16 si recall trop bas |
| J3 | Latency benchmark CPU + comparison ONNX runtime versions |
| J4 | Trust store + bootloader extension |
| J5 | 8 instruments Prometheus bundle |
| J6 | Baseline SLO bundle YAML + intégration `check_slo.py` |
| J7 | `embedder/runtime/` Rust : session lifecycle + provider fallback |
| J8 | Tests intégration : boot avec bundle réel, verify, pre-warm |
| J9 | Memory profiling (heap + VRAM) |
| J10 | Tag `bundle/paraphrase-multilingual-MiniLM-L12-v2/1.0.0` |

### D27 — Sprint Bundle-Distribution (10 j wallclock)

| Jour | Tâche |
|----:|-------|
| J1-J2 | OCI registry primary + 3 mirrors regionaux |
| J3-J4 | CI workflow `bundle-release.yml` end-to-end testé |
| J5 | Migration `bundle_releases` SQL |
| J6 | Runbook rotation + revocation + air-gap |
| J7 | Compliance doc EU AI Act |
| J8 | Canary mechanism dans le helm chart |
| J9 | Audit interne (8+ bugs typiques pattern D8/D18/P1/P2) |
| J10 | Freeze v1.0.0-rc final + sign release notes |

---

## 14. Risques identifiés & mitigations (18)

| # | Risque | Sévérité | Probabilité | Mitigation |
|--:|--------|:--------:|:-----------:|------------|
| 1 | ONNX export sentence-transformers buggy | HIGH | MED | `optimum-cli` validé en CI + numerical equivalence test 1e-5 |
| 2 | fp16 drift > 0.001 BERTScore | MED | LOW | Gate sur cosine > 0.9995 ; rollback fp32 si fail |
| 3 | Recall multilingue FR << EN | MED | MED | SLO-BUNDLE-LINGUAL ; revert bundle si > 1.05× |
| 4 | Latence p99 T4 > 26 ms | HIGH | LOW | Bench dès rc1 ; tune `intra_op_num_threads` ; fallback CUDA EP default options |
| 5 | Bundle non-reproductible cross-runner | HIGH | MED | 7 sources de non-déterminisme neutralisées (cf. § 3.3) ; test pin |
| 6 | License audit raté (= modèle non-comm caché) | HIGH | LOW | `audit_license.py` whitelist Apache-2.0/MIT seulement |
| 7 | Cosign Rekor down | LOW | LOW | retry exponentiel 5× ; fallback Ed25519-only avec WARN log |
| 8 | OCI registry primary down | MED | LOW | 3 mirrors regionaux fallback |
| 9 | Bundle trop gros (> 200 MiB) | LOW | LOW | gate SLO-BUNDLE-SIZE ; tune fp16 ou int8 |
| 10 | ONNX runtime ABI break (= 1.18 → 1.21) | MED | MED | `runtime.min_version`/`max_version` range explicite |
| 11 | Memory leak en steady-state | HIGH | LOW | RSS monitor + alerte `BUNDLE_MEMORY_LEAK` à 2 GiB |
| 12 | Tokenizer JSON malformé | MED | LOW | Loader Rust valide via `tokenizers::Tokenizer::from_file` ; refuse si fail |
| 13 | CUDA EP indisponible mais config exige | LOW | MED | Boot-time check ; refuse `ready=true` si missing |
| 14 | Clé Ed25519 compromise | HIGH | LOW | Rotation 6 mois + revocation procedure § 11.2 |
| 15 | SBOM CycloneDX schéma break | LOW | LOW | Pin spec 1.5 ; bump explicite si jamais |
| 16 | Air-gap deploy échoue | LOW | LOW | Runbook documenté + test scénario dans CI |
| 17 | Pre-warm p99 > 50 ms | MED | MED | Tune `intra_op_num_threads=1` + ORT optim level Extended |
| 18 | Modèle source HF deleted | LOW | LOW | Mirror local du snapshot par commit SHA ; build reproducible offline |

---

## 15. Décisions de conception clés (16)

| # | Décision | Alternative rejetée | Justification |
|--:|----------|---------------------|---------------|
| 1 | MiniLM-L12-v2 multilingue (vs L6-v2 EN ou mpnet) | L6-v2 EN | Couvre 50+ langues sans 2× latence, license Apache-2.0 propre |
| 2 | fp16 GPU + fp32 CPU fallback | int8 partout | fp16 = 35 % speedup vs fp32 sans perte mesurable ; int8 = recall drift > 0.01 |
| 3 | ONNX opset 14 (vs 16, 17) | dernière version | Stable depuis 2 ans ; CUDA EP compat |
| 4 | Bundle scope global (1 bundle Fournisseur, pas per-tenant) | per-tenant fine-tuned bundles | v1.0.0 = pas de fine-tuning ; per-tenant arrive en M30+ |
| 5 | Cosign + Rekor (Sigstore) | GPG seulement | Transparency log public ; verifiable par tiers sans clé Fournisseur |
| 6 | OCI Artifacts (vs S3 direct) | S3 | Auth unifié avec image Docker ; immutabilité native ; cosign natif |
| 7 | 3 mirrors regionaux | mono-region | Latence pull < 100 ms global ; tolerance outage primary |
| 8 | Helm chart embarque trust-store | trust-store fetch runtime | Sécu : pas de fetch runtime du trust store (attack surface) |
| 9 | `efe-bundle verify` standalone (vs intégré server) | server-only | Permet vérification offline / par tiers sans run le server |
| 10 | Pinning toolchain dans `requirements-bundle.txt` | venv flottant | Reproductibility cross-runner |
| 11 | Recall benchmark @1 (vs @5 ou MRR) | recall@5 | Plus strict ; aligne avec usage "classify" qui retourne 1 verdict |
| 12 | Multilingue parity ≤ 1.05× | ≤ 1.10× | Plus strict ; certains marchés (DACH, ES) exigent parité forte |
| 13 | Pre-warm p99 ≤ 50 ms | 100 ms | Permet de prendre traffic dès `ready=true` sans bias initial |
| 14 | Bundle size ≤ 200 MiB | 500 MiB | Compatible avec P2 30-min POC (pull < 30 s) |
| 15 | Rotation cadence 6 mois | annuel | Plus court = exposition réduite si vulnérabilité ONNX runtime |
| 16 | Canary 5/25/50/100 sur 8 h | bigbang 100 % | Catch regression latence/recall avant exposition globale |

---

## 16. Anti-régression & realism check

### 16.1 Anti-régression

Tous les SLO bundle (5 : HASH, RECALL, LINGUAL, SIZE, PREWARM)
sont gatés dans CI ; aucune régression admise en main. Le baseline
`bench_baselines/v1.0.0/bundle_slos.yaml` historique frozen ; chaque
version doit montrer un delta ≥ 0.

### 16.2 Realism check

- **Recall** doit être mesuré contre la baseline
  sentence-transformers Python (= ground truth), pas contre un
  autre ONNX export (= circulaire).
- **Latence** doit être mesurée sur T4 réel ; les benches sur
  CPU émulé Apple Silicon avec accélération Metal ne comptent
  pas pour SLO-P1-BUDGET-5.
- **Reproductibility** : doit être attesté par exécution sur 3
  hosts distincts (= 1 dev local, 1 CI runner GitHub-hosted, 1
  machine de l'audit firm), pas par 3 runs sur la même machine.
- **Bundle ≠ benchmark** : on ne triche pas en exposant un
  bundle qui n'est jamais réellement loadé en prod ; le bundle
  shipped doit être 100 % identique à celui benché.

### 16.3 Sanity checks au démarrage server

`efe-server` au boot avec un bundle :
1. Verify (= 9 checks § 5.1).
2. ONNX session init.
3. Tokenizer load.
4. Pre-warm 100 inférences.
5. Émet `BUNDLE_READY` log + `efe_bundle_loaded_at_boot = 1`.
6. Si l'un des 5 échoue → refuse `ready=true` au healthz.

---

## 17. Critères d'acceptation (DoD)

### 17.1 Code

- [ ] 28 artefacts bundle livrés (§ 12).
- [ ] `scripts/release/build_real_bundle.py` produit un bundle bit-identique sur 3 runners.
- [ ] `efe-bundle verify` passe les 9 checks sur le bundle livré.
- [ ] Boot `efe-server` avec le bundle réel : `ready=true` en ≤ 60 s.
- [ ] 8 instruments Prometheus bundle émis.
- [ ] Trust store `/etc/efe/trust-store.toml` shipped dans image.

### 17.2 CI

- [ ] Workflow `bundle-release.yml` E2E (build → bench → sign → push) en CI.
- [ ] 5 SLO bundle gatés dans `check_slo.py`.
- [ ] Failure d'un SLO bloque le tag `bundle/*`.
- [ ] Artifact `recall-report.json` + `latency-report.json` uploadés.

### 17.3 Mesure live (sur baseline v1.0.0)

- [ ] **recall@1** ≥ 0.92 sur eval multilingue.
- [ ] **ratio FR/EN** ≤ 1.05.
- [ ] **p99 T4 fp16 seq=128** ≤ 26 ms.
- [ ] **p99 CPU fp32 seq=128** ≤ 180 ms (= consomme < 25 % du C-2 budget 800 ms).
- [ ] **size compressed** ≤ 200 MiB.
- [ ] **pre-warm p99** ≤ 50 ms.
- [ ] **VRAM peak** ≤ 1 GiB sur T4.

### 17.4 Documentation

- [ ] `docs/EFE_BUNDLE_EMBEDDER_REAL_ARCHITECTURE.md` (ce doc).
- [ ] `docs/runbook/bundle_rotation.md`.
- [ ] `docs/runbook/bundle_revocation_emergency.md`.
- [ ] `docs/runbook/bundle_airgap_distribution.md`.
- [ ] `docs/compliance/eu_ai_act_bundle.md`.
- [ ] CHANGELOG v0.3.0 mentionne le bundle réel + SLO.

### 17.5 Contractuel

- [ ] License Apache-2.0 + NOTICE.md inclus dans bundle.
- [ ] SBOM CycloneDX 1.5 inclus.
- [ ] Cosign Rekor entry pour `v1.0.0` publique.
- [ ] Bundle vérifié par audit firm externe (= attestation tiers).

---

## 18. Hand-off & dépendances downstream

### 18.1 Dépendances entrantes

| Sprint | Livré | Ce que Bundle-Real réutilise |
|--------|-------|-------------------------------|
| D16 | Bundle format + loader | format JSON, loader Rust, verify infrastructure |
| M13 | Pipeline CI sample | scripts de build, sign Ed25519, OCI distribution |
| D14-bis | Audit chain | bundle_releases ledger pattern |
| D17 | Backup + DR | mirror replication pattern |
| P1 | Latence opposable | SLO-P1-BUDGET-5 source (= bundle inference) |
| P2 | Self-service | bundle pull doit fit dans 30-min POC |

### 18.2 Dépendances sortantes

| Sprint downstream | Ce qu'il prend du Bundle Real |
|-------------------|-------------------------------|
| Φ-D (D28-D32) | Bundle réel pour bench CUDA → calibrer SLO-C-1 |
| Φ-E (D33-D36) | Mode-2 utilise le bundle pour encoder l'output LLM |
| Φ-F (D37-D39) | Mode-3 utilise le bundle pour k turns |
| Φ-G (D40-D42) | Audit tiers vérifie la chaîne bundle (Sigstore + SBOM + recall) |
| GA v1.0.0 | SLO-C-1 + SLO-C-2 mesurables sur hardware réel |

### 18.3 Risque ordre des sprints

Si Φ-C glisse, Φ-D ne peut pas commencer (= besoin du bundle pour
bench GPU). Décision : si Φ-C dépasse D27 de plus de 1 semaine,
shipper Φ-C en CPU-only et continuer Φ-D en CUDA stub bench (= avec
le bundle réel mais sans custom kernels CUDA). Cela permet de
calibrer SLO-C-2 (CPU) au moins, et reporter SLO-C-1 à D32+.

---

## 19. Annexes (A → H)

### Annexe A — Liste des 5 SLO bundle

```yaml
slos:
  - id: SLO-BUNDLE-HASH
    description: "manifest.json hash matches baseline"
    comparator: eq
    source: "efe-bundle verify --against-baseline"

  - id: SLO-BUNDLE-RECALL
    description: "recall@1 multilingue"
    metric: recall_at_1
    comparator: gte
    threshold_ratio: 0.92

  - id: SLO-BUNDLE-LINGUAL
    description: "ratio max(recall_lang) / min(recall_lang)"
    comparator: lte
    threshold_ratio: 1.05

  - id: SLO-BUNDLE-SIZE
    description: ".tar.gz size"
    comparator: lte
    threshold_bytes: 209_715_200  # 200 MiB

  - id: SLO-BUNDLE-PREWARM
    description: "pre-warm 100 inferences p99 au boot"
    comparator: lte
    threshold_seconds: 0.050
```

### Annexe B — Glossaire

| Terme | Définition |
|-------|------------|
| **Bundle** | Artefact ML production-grade (= ONNX + tokenizer + manifest signé) |
| **Manifest** | JSON authoritative metadata blob du bundle (= source du `fingerprint`) |
| **Fingerprint** | SHA-256 du manifest, sert d'ID stable |
| **Trust store** | Liste de clés publiques autorisées à signer, embarquée dans image |
| **Pre-warm** | 100 inférences dummy au boot pour amortir cold-start ORT |
| **Rekor** | Sigstore transparency log (= public append-only audit) |
| **OCI Artifacts** | Standard CNCF pour distribution de blobs non-Docker via registry |
| **SBOM** | Software Bill of Materials (= liste de tous les composants) |
| **Recall@1** | % de phrases dont le top-1 NN matches la baseline ground-truth |
| **Reproductible** | 2 runs sur 2 machines produisent le même SHA-256 |
| **Canary** | Déploiement graduel 5 % → 100 % avec rollback automatique |

### Annexe C — Exemple `requirements-bundle.txt` complet

```
# Pinned dependencies for bundle build reproducibility.
# Generated by `scripts/release/freeze_requirements.py`.
huggingface_hub==0.24.0
optimum==1.20.0
onnx==1.16.1
onnxruntime==1.18.1
transformers==4.42.4
sentence-transformers==3.0.1
torch==2.3.1+cpu
tokenizers==0.19.1
cryptography==42.0.7
cosign==2.2.4
python-cyclonedx-bom==4.1.0
```

### Annexe D — Commandes de vérification rapide

```bash
# Verify un bundle local
$ efe-bundle verify ./bundles/paraphrase-multilingual-MiniLM-L12-v2-1.0.0/

# Verify avec contraintes strictes (= reproductibility + recall smoke)
$ efe-bundle verify ./bundles/... --strict --against-baseline <sha>

# Pull + verify depuis OCI
$ oras pull registry.efe.ai/bundles/paraphrase-multilingual-MiniLM-L12-v2:1.0.0
$ efe-bundle verify ./bundle.tar.gz

# Verify cosign Rekor entry
$ cosign verify-blob \
    --bundle bundle.cosign.bundle \
    --certificate-identity-regexp "https://github.com/efe-ai/efe/.+" \
    --certificate-oidc-issuer-regexp "https://token.actions.githubusercontent.com" \
    manifest.json

# Benches manuels
$ python scripts/bench/bundle_recall_bench.py --bundle ./bundles/... --eval-set eval/golden/multilingual_recall_v1.0.jsonl
$ python scripts/bench/bundle_latency_bench.py --bundle ./bundles/... --provider cuda
```

### Annexe E — Schéma `bundle_releases` ledger

```sql
CREATE TABLE bundle_releases (
    release_id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    bundle_id           TEXT NOT NULL,
    version             TEXT NOT NULL,
    release_kind        TEXT NOT NULL CHECK (release_kind IN ('initial','rotation','revocation','emergency')),
    manifest_sha256     BYTEA NOT NULL,
    tar_sha256          BYTEA NOT NULL,
    released_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    released_by         TEXT NOT NULL,
    approved_by         TEXT,
    rekor_log_index     BIGINT NOT NULL,
    rekor_url           TEXT NOT NULL,
    notes               TEXT,
    UNIQUE (bundle_id, version)
);
CREATE INDEX idx_bundle_releases_bundle_id ON bundle_releases(bundle_id, version);
```

### Annexe F — Mapping vers SLO produit final C-1..C-20

| C-* | Description | Couvert par Bundle Real |
|-----|-------------|--------------------------|
| C-1 | Mode-1 GPU latency p99 ≤ 200 ms | SLO-P1-BUDGET-5 dépend du bundle |
| C-2 | Mode-1 CPU latency p99 ≤ 800 ms | bench CPU bundle ≤ 180 ms (= < 25 % budget) |
| C-5 | Coût par classify ≤ 0.5 ¢ | inference cost du bundle = facteur principal |
| C-7 | Confidence calibration Brier ≤ 0.10 | recall bundle suffit (= calibration faite par swarm en aval) |
| C-17 | SBOM dans chaque image | bundle SBOM est livré ; image SBOM hérite |
| C-18 | Compliance SOC2/GDPR/AI Act | bundle a sa propre attestation Article 53 |

### Annexe G — Memo "Pourquoi pas un LLM ?"

Question récurrente : pourquoi un embedder + swarm plutôt qu'un
LLM monolithique pour classifier ?

| Critère | LLM chain (5 turns) | EFE bundle + swarm |
|---------|--------------------:|---------------------:|
| Latence p99 | 1.5-3 s | 200 ms |
| Coût / classify | 5-50 ¢ | 0.5 ¢ |
| Reproductibilité | non (sampling) | oui (déterministe) |
| Auditabilité | difficile (logs textuels) | cryptographique (chaîne HMAC) |
| Multi-tenant isolation | difficile (prompt injection cross-tenant) | trivial (= isolation k8s namespace + per-tenant catalog) |
| Compliance AI Act | High-Risk classification difficile | GPAI + transparency native |

→ Le bundle réel + swarm est le **moat technique** d'EFE vs les
LLM chains. C'est ce qui permet de tenir P1 + P2 + P3
simultanément.

### Annexe H — Évolution post-GA (= roadmap v1.1+)

| Version | Bundle | Trigger |
|---------|--------|---------|
| v1.0.0 | paraphrase-multilingual-MiniLM-L12-v2 | GA |
| v1.0.1 | idem + patch ONNX runtime 1.18.x | Cadence 6 mois |
| v1.1.0 | + all-mpnet-base-v2 EN-heavy (Enterprise tier) | Si demande client |
| v1.2.0 | + custom fine-tuned per-tenant | M30+ active learning |
| v2.0.0 | E5-large-multi (= si fp16 stable trouvé) | T+12 mois |
| v2.1.0 | bge-m3 (= si license cleared) | T+18 mois |
| v3.0.0 | Custom EFE model from scratch (= mission-specific embedder, entraîné sur swarm-output data) | T+30 mois |

---

— **fin du document** —
