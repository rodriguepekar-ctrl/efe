# Sprint M13 — Bundles modèles : architecture détaillée du contenu, signing opérationnel, et CI auto-bundling

> **Statut** : architecture détaillée, prête à implémenter.
> **Phase** : ultra-thinking. Précision chirurgicale suprême.
> **Position** : suit M12 (Helm chart extension D21+D22+D27+D40, livré
> + audit pass 1 — 8 défauts corrigés, 43 pin tests GREEN). M13 est
> l'**opérationnalisation** de D16 (`docs/bench/d16_embedder_bundling_architecture.md`,
> 2 213 lignes) qui définissait le **format** du bundle (manifest
> JSON Schema v1.0, Ed25519 signing scheme, OCI Artifacts distribution
> mechanism, loader Rust, threat model supply-chain ML), mais n'a
> **livré aucun bundle réel** — le dossier `bundles/` contient
> uniquement un placeholder dev de 32 octets (`weights.bin` rempli
> de zéros) + un manifest dev-unsigned.
>
> **Pré-requis** : avoir lu
> [`d16_embedder_bundling_architecture.md`](bench/d16_embedder_bundling_architecture.md)
> dans son intégralité (format spec, threat model, signing scheme,
> OCI distribution, loader Rust, catalog binding) — M13 ne ré-explique
> PAS la conception, il en livre les artefacts.
> Avoir également une compréhension de :
> [`EFE_HELM_CHART_EXTENSION_ARCHITECTURE.md`](EFE_HELM_CHART_EXTENSION_ARCHITECTURE.md)
> § 11 (Helm post-upgrade hook D27 bundle reload — consomme les
> bundles M13 via `bundle.manifestSha256`),
> [`d14bis_hmac_audit_chain_architecture.md`](bench/d14bis_hmac_audit_chain_architecture.md)
> (le pattern de signature détachée HMAC-chain dont D16 s'inspire),
> [`d17_backup_dr_architecture.md`](bench/d17_backup_dr_architecture.md)
> § 8 (Object Lock COMPLIANCE 7-year retention — pattern repris pour
> les bundles immutables), et le `scripts/release/build_sample_bundle.py`
> existant (post-M8 audit pass 1, fixes A1+A2+A3 livrés) qui couvre
> Phase 1-5 du build (pull / quantize / manifest / sign / summary).
>
> **Mission M13** : transformer le dossier `bundles/` (vide, dev-only
> placeholder) en une **distribution complète, signée, reproductible
> et auto-construite par CI** de bundles modèles ML production-grade.
> Trois sprints :
>
> | Sprint | Phase | Jour | Livrable |
> |---|:---:|:---:|---|
> | **D20** | Φ-A | J55  | Sample bundle `bundles/all-MiniLM-L6-v2-en-1.0.0/` (~30 MiB INT8 quantizé) signé Ed25519 |
> | **D26** | Φ-B | J120 | Bundle réel `bundles/paraphrase-multilingual-MiniLM-L12-v2-1.0.0/` (~80 MiB) signé + SBOM CycloneDX 1.5 |
> | **D27** | Φ-C | J140 | Pipeline CI auto-bundling (`.github/workflows/bundle-release.yml`) + endpoint OCI Artifacts `oci://registry.efe.ai/bundles/<id>:<version>` |
>
> **Pain points opérationnels résolus** :
>
> 1. **Quickstart impossible aujourd'hui** : un développeur qui
>    clone le repo + lance `efe-admin classify "hello"` se heurte
>    au placeholder `weights.bin` de 32 zéros. Le SDK Python load
>    échoue (rejected by ONNX Runtime). Pas de path E2E utilisable.
> 2. **Production deployment impossible** : sans bundle réel signé,
>    `EFE_BUNDLE_REQUIRE_SIGNATURE=true` (défaut Helm prod, M12 §14)
>    refuse de booter. La chart M12 est livrée mais inutilisable.
> 3. **Aucune reproductibilité actuelle** : le placeholder dev a
>    `is_dev_unsigned=true`, donc le pipeline OCI publish refuse
>    de le distribuer. Le bundle ne peut jamais quitter dev.
> 4. **CI manuelle = drift** : sans pipeline auto-bundling, chaque
>    nouvelle version du modèle nécessite un opérateur humain qui
>    pull + quantize + sign + push. Risque d'erreur (mauvaise
>    privkey, mauvaise version, mauvais hash) ; pas de
>    reproducibility check automatique.
> 5. **OCI registry pas wiré** : la chart M12 référence
>    `oci://registry.efe.ai/bundles/<id>` dans la doc opérationnelle
>    mais aucun bundle n'y est actuellement publié.
> 6. **SBOM manquant** : compliance SOC 2 CC8.1 (change management)
>    + SLSA Level 3 + ML-BOM (CycloneDX 1.5) exigent un SBOM par
>    artefact ML déployé. Aujourd'hui : 0 SBOM produit.
> 7. **License attribution invisible** : Apache-2.0, MIT, OpenRAIL-M
>    requièrent que la license soit incluse dans la distribution
>    redistribuée. Sans bundle ship-ready, pas de trace légale.
> 8. **Multi-langue absente** : le sample est `en`-only. Pour les
>    déploiements EU/multi-tenant FR+ES+DE, il faut un bundle
>    multilingual (D26).
>
> **Note importante sur le périmètre** : ce sprint ne change PAS
> le format manifest (D16 v1.0), le loader Rust (D16 §8), ni le
> trust-store mechanism (D16 §5.2). M13 livre **les artefacts**,
> **le pipeline qui les construit**, et **l'infrastructure OCI**
> qui les distribue. Toute évolution du format est gated par un
> bump `manifest_version` (interdit en M13, refusé par les pin
> tests anti-régression).

---

## Table des matières

1. [Pain points + objectifs M13 (mesurables)](#1-pain-points--objectifs-m13-mesurables)
2. [Threat model M13 — supply-chain auto-build CI](#2-threat-model-m13--supply-chain-auto-build-ci)
3. [Bundle inventory canonique M13](#3-bundle-inventory-canonique-m13)
4. [D20 — Sample bundle `all-MiniLM-L6-v2-en-1.0.0`](#4-d20--sample-bundle-all-minilm-l6-v2-en-100)
5. [D26 — Bundle production `paraphrase-multilingual-MiniLM-L12-v2-1.0.0`](#5-d26--bundle-production-paraphrase-multilingual-minilm-l12-v2-100)
6. [D27 — Pipeline CI auto-bundling](#6-d27--pipeline-ci-auto-bundling)
7. [OCI Artifacts publication mechanism](#7-oci-artifacts-publication-mechanism)
8. [Manifest schema instances concrètes](#8-manifest-schema-instances-concrètes)
9. [Signing & verification operational flow](#9-signing--verification-operational-flow)
10. [Reproducibility bit-for-bit (SOURCE_DATE_EPOCH)](#10-reproducibility-bit-for-bit-source_date_epoch)
11. [SBOM CycloneDX 1.5 (ML-BOM)](#11-sbom-cyclonedx-15-ml-bom)
12. [License compliance & redistribution](#12-license-compliance--redistribution)
13. [Helm integration (post-M12)](#13-helm-integration-post-m12)
14. [CI workflow stages détaillés](#14-ci-workflow-stages-détaillés)
15. [Configuration surface (env + Helm values + flags CLI)](#15-configuration-surface-env--helm-values--flags-cli)
16. [Test plan M13 (35+ pin tests)](#16-test-plan-m13-35-pin-tests)
17. [Operational runbook](#17-operational-runbook)
18. [SLOs M13-BND-*](#18-slos-m13-bnd)
19. [Migration path D16-stub → M13](#19-migration-path-d16-stub--m13)
20. [Risk register & open questions](#20-risk-register--open-questions)
21. [Appendix A — alternatives considered](#21-appendix-a--alternatives-considered)
22. [Appendix B — glossaire](#22-appendix-b--glossaire)
23. [Appendix C — `bundle-release.yml` full example](#23-appendix-c--bundle-releaseyml-full-example)
24. [Appendix D — `build_real_bundle.py` blueprint](#24-appendix-d--build_real_bundlepy-blueprint)
25. [Appendix E — pin tests inventory complète](#25-appendix-e--pin-tests-inventory-complète)

---

## 1. Pain points + objectifs M13 (mesurables)

### 1.1 Statut quo `bundles/` post-D18 + M12

```
bundles/
└── all-MiniLM-L6-v2-en-1.0.0/
    ├── manifest.json     # 27 lignes, is_dev_unsigned=true
    ├── tokenizer.json    # 50 octets, _dev_placeholder=true
    └── weights.bin       # 32 octets de zéros (placeholder)
```

`manifest.sig` : **absent**. `SBOM.json` : **absent**.
`README.md` / `LICENSE` : **absent**.

**Diagnostic** : aucun bundle production-ready. Le placeholder existe
uniquement pour faire passer les pin tests de présence
(`test_d20_sample_bundle_present_and_signed` accepte le mode dev).

### 1.2 Pourquoi M13 alors que D16 existe ?

D16 a livré la **conception** :

- Format manifest JSON Schema v1.0 (autoritative)
- Signing scheme Ed25519 détaché (`manifest.json.sig`)
- OCI Artifacts distribution mechanism (`oras push/pull`)
- Loader Rust (`crates/efe-server/src/embedder/bundle.rs`)
- Trust-store `pubkeys.json` mounted via K8s Secret
- Catalog ↔ bundle fingerprint binding (D14 schema extension)

D16 n'a **pas livré** :

- Aucun bundle réel (placeholder dev only)
- Aucun pipeline CI auto-bundling (le `release.yml` existant
  ne publie que l'image Docker `efe-server`)
- Aucune publication OCI (registry vide)
- Aucun SBOM (placeholder n'a pas de dépendances réelles)
- Aucune procédure de rotation de signing key opérationnelle
- Aucun multilingual bundle (gap pour EU/multi-tenant)

M13 = livraison opérationnelle. **Triplet sprint** :

| Sprint | Quoi | Pourquoi |
|---|---|---|
| **D20** | Sample bundle minimal signé | Quickstart fonctionnel + smoke test E2E |
| **D26** | Bundle production multilingual | Déploiements EU + clients FR/DE/ES |
| **D27** | Pipeline CI + OCI registry | Élimine la dérive humaine + supply-chain attestée |

### 1.3 Objectifs D20 (Sprint Φ-A)

| ID | Objectif | Métrique mesurable |
|---|---|---|
| **O-D20-1** | Bundle sample byte-exact reproductible | 2 builds CI back-to-back produisent SHA256 manifest identique |
| **O-D20-2** | Signature Ed25519 valide | `cosign verify-blob --signature manifest.sig --key pubkey.b64 manifest.json` retourne exit 0 |
| **O-D20-3** | Volumétrie ≤ 30 MiB on-disk | `du -sb bundles/all-MiniLM-L6-v2-en-1.0.0/` ≤ 31_457_280 octets |
| **O-D20-4** | INT8 dynamic quantization | `manifest.json::quantization == "int8-dynamic"` + `model.onnx` quantized verifiable via Netron inspection |
| **O-D20-5** | Quickstart E2E fonctionnel | `efe-admin classify "hello world"` retourne un label valide en < 5 s sur cold-start |
| **O-D20-6** | Fingerprint catalogue stable | `sha256(manifest.json) == d20_fingerprint_pinned` (épinglé dans `test_d20_*`) |

### 1.4 Objectifs D26 (Sprint Φ-B)

| ID | Objectif | Métrique mesurable |
|---|---|---|
| **O-D26-1** | Bundle production multilingual | 100+ langues supportées (paraphrase-multilingual-MiniLM-L12-v2 default) |
| **O-D26-2** | Volumétrie ≤ 80 MiB on-disk | `du -sb` ≤ 83_886_080 octets |
| **O-D26-3** | SBOM CycloneDX 1.5 valide | `cyclonedx-cli validate --input-file SBOM.json --schema-version 1.5` retourne exit 0 |
| **O-D26-4** | Inventaire ML complet dans SBOM | SBOM contient `components[*].type=="machine-learning-model"` pour le modèle + `data` pour le tokenizer |
| **O-D26-5** | License chain attestation | SPDX `Apache-2.0` documenté + `LICENSE` file inclus + `redistribution_allowed=true` |
| **O-D26-6** | Cosine similarity gate | Embeddings produits matchent ground truth dataset (~99.5% des paires hors-distribution > 0.5) |
| **O-D26-7** | Cross-lingual transfer | Score F1 ≥ 0.80 sur la batterie de tests `tests/fixtures/multilingual_eval.jsonl` (FR/DE/ES/IT/PT) |

### 1.5 Objectifs D27 (Sprint Φ-C)

| ID | Objectif | Métrique mesurable |
|---|---|---|
| **O-D27-1** | Pipeline auto-bundling déclenché par tag | `git tag bundle-v1.0.0 && git push` produit un push OCI complet sans intervention humaine |
| **O-D27-2** | Reproducibility check obligatoire | Le pipeline build le bundle DEUX fois (ubuntu-22.04 + ubuntu-22.04 même runner) et compare les SHA256 ; mismatch = échec |
| **O-D27-3** | OCI publish success rate | ≥ 99.5 % sur 30 derniers jours (Prometheus alert `D27ObsBundlePublishFailureRate`) |
| **O-D27-4** | Post-publish verify | Le pipeline pull le bundle juste publié, vérifie sa signature, hash chaque fichier, fail si mismatch |
| **O-D27-5** | Signing key dans secret KMS | Le pipeline lit la privkey depuis `secrets.EFE_BUNDLE_PRIVKEY_PEM` (sealed in GitHub repo) ou `EFE_BUNDLE_KMS_KEY_REF` (KMS path) |
| **O-D27-6** | Polaris + ct lint sur le pipeline | `actionlint .github/workflows/bundle-release.yml` retourne exit 0 |
| **O-D27-7** | OCI artifact retention | Tags publiés au registre privé pendant les phases pré-GA ; promotion vers public au D40 GA gate |

---

## 2. Threat model M13 — supply-chain auto-build CI

### 2.1 Récap D16 threat model

D16 §2 a couvert 10 menaces (T-16-1 à T-16-10) — modèle backdooré,
modèle remplacé en transit, bundle altéré on-disk, drift silencieux,
tokenizer ≠ poids, bundle plus récent que catalogue, insider
threat (CLI dev signed), ONNX Custom Op exfil, ré-utilisation
inter-tenant, cache loader corrompu.

### 2.2 Nouvelles menaces M13 (auto-build CI)

| ID | Menace | Vecteur | Conséquence | Mitigation M13 |
|---|---|---|---|---|
| **T-M13-1** | Compromise du runner GitHub Actions | RCE via PR malicieuse sur un fork | Privkey signing exfiltrée | Privkey en secret encrypted-at-rest GitHub + scope `bundle-release.yml` only via `permissions: { id-token: write, contents: read }` ; jamais en `pull_request_target` ; `if: github.repository == 'efe-project/efe'` strict |
| **T-M13-2** | Push OCI vers mauvais registry | Typosquat `registry.efe.ai` → `registery.efe.ai` | Distribution d'un bundle backdooré | Registry URL hard-coded en const + pin test ; `actionlint` vérifie ; `cosign push --tlog-upload` produit Rekor entry queryable |
| **T-M13-3** | Reproducibility bypass | CI rebuild différent → SHA différent | Loss of bit-exact attestation | Build deux fois dans le même job, échec si SHA mismatch ; `SOURCE_DATE_EPOCH` injecté depuis `git log -1 --format=%ct` |
| **T-M13-4** | OCI artifact écrasé après publish | Mauvais ACL registry | Bundle remplacé silencieusement | Tag immutable enforced côté registry (GHCR `package_settings.delete_branch_protection_rule`); `cosign verify` lifetime-pinned via Rekor |
| **T-M13-5** | SBOM divergent du bundle | SBOM générée d'un build, bundle d'un autre | Compliance audit fail | SBOM générée DANS le même job que le bundle, includes `manifest.sha256` comme provenance attestation |
| **T-M13-6** | Privkey leak via logs CI | `set -x` ou debug print de l'env | Privkey en clair dans GH Actions logs publiques | `echo "::add-mask::${PRIVKEY}"` immédiatement après load + `set +x` strict + GitHub secret scanning enabled |
| **T-M13-7** | Modèle upstream corrompu silencieusement | HF push surprise sur `main` | Bundle CI build = modèle backdooré | `huggingface_hub.snapshot_download(revision=PINNED_SHA)` obligatoire ; sha pinné dans `manifest.model_origin.huggingface_revision` ; refuse `revision=main` |
| **T-M13-8** | License non-redistribuable embarquée | Modèle upstream change de license en mid-flight | Violation légale en redistribuant le bundle | Pre-publish gate : `manifest.license.spdx_id ∈ ALLOWLIST` (Apache-2.0, MIT, BSD-3-Clause, CC-BY-4.0, OpenRAIL-M with ack) ; refus sinon |
| **T-M13-9** | OCI manifest annotation manipulée | Registry-side malicieux change `org.opencontainers.image.created` | Attestation date faussée | Manifest interne (notre `manifest.json`) reste source de vérité ; loader IGNORE les annotations OCI ; pin test |
| **T-M13-10** | DoS sur le job auto-bundling | Tag spam | Coût CI explosé | Concurrency `cancel-in-progress: false` + `if: startsWith(github.ref, 'refs/tags/bundle-v')` strict ; max 1 active runs |

### 2.3 Trust boundaries M13

```
                    ┌──────────────────────────────────────────┐
                    │   GitHub Actions runner (M13 CI)         │  ← Trust root
                    │   - PINNED_SHA upstream HF model         │
                    │   - SOURCE_DATE_EPOCH from git tag       │
                    │   - Privkey loaded from gh secret        │
                    │   - Build → Sign → Verify → Publish      │
                    └──────────┬───────────────────────────────┘
                               │
                               │ ORAS push (oci://registry.efe.ai/bundles/<id>:<v>)
                               │
                               │ (1) Bundle layer: manifest.json + manifest.sig + weights/ + tokenizer/ + SBOM.json
                               │ (2) OCI manifest with `org.opencontainers.artifact.type` = "application/vnd.efe.bundle.v1+json"
                               │
                               ▼
                    ┌──────────────────────────────────────────┐
                    │   OCI Registry (GHCR | self-hosted)      │  ← Semi-trusted
                    │   - Stores layers (immutable tags)       │
                    │   - Returns layers on pull               │
                    │   - Logs all push events to Rekor        │
                    └──────────┬───────────────────────────────┘
                               │ ORAS pull
                               ▼
                    ┌──────────────────────────────────────────┐
                    │   K8s pod efe-server (M12 D27 init)      │  ← Zero-trust verifier
                    │   - Verifies manifest.sig (Ed25519)      │
                    │   - Verifies per-file SHA256             │
                    │   - Verifies SBOM integrity              │
                    │   - Refuses if license not in allowlist  │
                    │   - Loads model into ONNX Runtime        │
                    └──────────────────────────────────────────┘
```

Trust roots :

- **CI root** : la privkey Ed25519 sealed dans GH secrets + le KMS HSM-exported.
- **OCI root** : Rekor transparency log entry (immutable + auditable).
- **Verifier root** : `pubkeys.json` mounted dans le pod via K8s Secret.

Compromise de l'un n'implique PAS compromise des autres (defense in depth).

---

## 3. Bundle inventory canonique M13

### 3.1 Bundles livrés

| Slug | Sprint | Modèle upstream | HF revision | Size on-disk | Quantization | Framework | Dim | License |
|---|:---:|---|---|:---:|:---:|:---:|:---:|:---:|
| `all-MiniLM-L6-v2-en-1.0.0` | D20 | `sentence-transformers/all-MiniLM-L6-v2` | `e4ce9877abf3edfe10b0d82785e83bdcb973e22e` | 30 MiB | int8-dynamic | onnx | 384 | Apache-2.0 |
| `paraphrase-multilingual-MiniLM-L12-v2-1.0.0` | D26 | `sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2` | `bf3bf13ab40c3157080a7ab344c831b9ad18b5eb` | 80 MiB | fp16 | onnx | 384 | Apache-2.0 |

### 3.2 Bundle ID canonique (D16 §3.2)

```
{family}/{name}@{version}/{framework}/{dim}
```

D20 :
```
sentence-transformers/all-MiniLM-L6-v2@1.0.0/onnx/384
```

D26 :
```
sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2@1.0.0/onnx/384
```

### 3.3 OCI tag canonique

```
oci://registry.efe.ai/bundles/<slug>:<version>
```

D20 :
```
oci://registry.efe.ai/bundles/all-MiniLM-L6-v2-en:1.0.0
```

D26 :
```
oci://registry.efe.ai/bundles/paraphrase-multilingual-MiniLM-L12-v2:1.0.0
```

Pre-GA (Φ-A → Φ-B) : registre **privé** `ghcr.io/efe-project/bundles-private/...`.
GA (post-D40) : registre **public** `registry.efe.ai/bundles/...`.

### 3.4 Layout filesystem post-M13

```
bundles/
├── all-MiniLM-L6-v2-en-1.0.0/                # D20
│   ├── README.md                              # Provenance + checksums
│   ├── LICENSE                                # Apache-2.0 verbatim
│   ├── SBOM.json                              # CycloneDX 1.5 (deferred D20→D26 si time-boxed)
│   ├── manifest.json                          # D16 v1.0 schema
│   ├── manifest.json.sig                      # Ed25519 detached signature
│   ├── runtime/
│   │   └── runtime_config.json                # max_seq_len=256, pooling=mean, normalize=true, dtype=float32
│   ├── tokenizer/
│   │   ├── tokenizer.json                     # HF fast-tokenizer JSON
│   │   ├── tokenizer_config.json
│   │   ├── special_tokens_map.json
│   │   └── vocab.txt
│   └── weights/
│       └── model.onnx                         # ONNX INT8 dynamic quantized, ~28 MiB
├── paraphrase-multilingual-MiniLM-L12-v2-1.0.0/  # D26
│   ├── README.md
│   ├── LICENSE
│   ├── SBOM.json                              # OBLIGATOIRE pour D26
│   ├── manifest.json
│   ├── manifest.json.sig
│   ├── runtime/
│   │   └── runtime_config.json                # max_seq_len=128, pooling=mean, normalize=true, dtype=float16
│   ├── tokenizer/
│   │   ├── tokenizer.json                     # XLM-R based multilingual tokenizer
│   │   ├── tokenizer_config.json
│   │   ├── special_tokens_map.json
│   │   └── sentencepiece.bpe.model            # XLM-R subword model
│   └── weights/
│       └── model.onnx                         # ONNX FP16, ~78 MiB
└── README.md                                   # Dossier-level index
```

### 3.5 Pourquoi ces deux modèles précisément ?

**`all-MiniLM-L6-v2`** (D20) :
- Modèle "hello world" du semantic embeddings : 6 layers Transformer
  encoder, 384 dim, ~22M params.
- Distillé de `microsoft/MiniLM-L12-H384-uncased` puis fine-tuné
  sur 1B+ paires (`sentence-transformers` README).
- Latence < 5 ms encode/sentence sur CPU x86_64.
- License Apache-2.0 (clean redistribution).
- Quantizable INT8 sans perte significative (~99.3% cosine
  similarity vs fp32 baseline).
- Taille post-quant ~30 MiB → ship-shipped quickstart.

**`paraphrase-multilingual-MiniLM-L12-v2`** (D26) :
- 12 layers Transformer (vs 6 pour D20), 384 dim, ~118M params.
- Multilingual training set : 50+ langues directement représentées,
  100+ via cross-lingual transfer (XLM-R-style).
- License Apache-2.0.
- Format ONNX disponible directement upstream
  (`sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2`
  → `onnx/model.onnx`).
- FP16 quantization stable → ~80 MiB post-quant.

Pourquoi pas D26 avec un modèle plus gros (BGE-large, 1024 dim) ?

- **Volumétrie** : >300 MiB, hors budget Helm Secret (max ~1 MiB)
  → exigerait un PVC ou OCI pull-at-boot, hors scope M13.
- **Latence** : 30 ms+ par encode → viole `M11-RPC-3` SLO.
- **Choix réservé pour D40+** (production tier-3 / GPU pods).

### 3.6 Versions futures

Le pattern `slug-Major.Minor.Patch` permet plusieurs versions
co-existantes :

```
bundles/
├── all-MiniLM-L6-v2-en-1.0.0/      # M13 / D20 baseline
├── all-MiniLM-L6-v2-en-1.0.1/      # patch tokenizer config (futur D40+)
├── all-MiniLM-L6-v2-en-1.1.0/      # int4 quantization (futur D45+)
└── all-MiniLM-L6-v2-en-2.0.0/      # breaking : new bundle_id format (M14+)
```

Le `manifest_version` reste `"1.0"` pour TOUTES ces versions tant
que le **schema** ne change pas. Le `bundle_id`'s semver track
change CONTENT, pas SCHEMA.

---

## 4. D20 — Sample bundle `all-MiniLM-L6-v2-en-1.0.0`

### 4.1 Sub-sprint inventory (D20 = J55, ~1 semaine d'effort)

| Sub-sprint | Phase | Output | Deps |
|---|---|---|---|
| **D20.1** | Pull modèle upstream HF (CI mode) | `weights.onnx` ~88 MiB FP32 + `tokenizer.json` | HF access (no auth) |
| **D20.2** | Quantization INT8 dynamic | `model.onnx` ~28 MiB | `onnxruntime>=1.16` quantize tool |
| **D20.3** | Manifest write canonique | `manifest.json` | `build_sample_bundle.py` post-A1+A2+A3 |
| **D20.4** | Ed25519 signature | `manifest.json.sig` | `cryptography>=42` + privkey |
| **D20.5** | LICENSE + README copy | `LICENSE`, `README.md` | upstream HF + manuel |
| **D20.6** | Smoke test E2E quickstart | `efe-admin classify "hello"` retourne label | M12 chart deployable |
| **D20.7** | Pin tests anti-régression | 8 tests `test_d20_*` | pytest |

### 4.2 D20.1 — Pull modèle upstream

**Source** :
```
huggingface.co/sentence-transformers/all-MiniLM-L6-v2
revision: e4ce9877abf3edfe10b0d82785e83bdcb973e22e (SHA pinned)
```

**Commande CI** (`build_sample_bundle.py` enrichi) :

```python
def phase_pull_model(out_dir: Path, *, dev: bool, hf_revision: str | None) -> None:
    """Pull upstream model from HF Hub (CI mode) or use placeholder (dev).

    Phase D20.1 — pinned revision MANDATORY in CI mode (avoids T-M13-7).

    In CI mode:
      1. huggingface_hub.snapshot_download with revision=PINNED_SHA
      2. Verify ONNX export available (else fall back to PyTorch + onnx-export)
      3. Stage to out_dir/weights/model.onnx + out_dir/tokenizer/

    In dev mode: skip (placeholder weights.bin + tokenizer.json
                  already shipped post-A1+A2+A3 fixes).

    Pin tested by `test_d20_pull_uses_pinned_revision_in_ci_mode`.
    """
    if dev:
        return  # legacy path A1 fix
    if hf_revision is None:
        raise RuntimeError(
            "Audit fix M13-D20-A1 — CI mode requires --hf-revision "
            "(no `revision=main` resolution allowed) ; "
            "see threat T-M13-7"
        )
    from huggingface_hub import snapshot_download
    snapshot_download(
        repo_id="sentence-transformers/all-MiniLM-L6-v2",
        revision=hf_revision,
        local_dir=str(out_dir / "_upstream"),
        local_dir_use_symlinks=False,
        allow_patterns=[
            "tokenizer.json",
            "tokenizer_config.json",
            "special_tokens_map.json",
            "vocab.txt",
            "onnx/model.onnx",
            "config.json",
        ],
    )
    # Re-organize to canonical bundle layout
    shutil.move(out_dir / "_upstream/onnx/model.onnx",
                out_dir / "weights/model.onnx")
    for f in ("tokenizer.json", "tokenizer_config.json",
              "special_tokens_map.json", "vocab.txt"):
        shutil.move(out_dir / "_upstream" / f,
                    out_dir / "tokenizer" / f)
    shutil.rmtree(out_dir / "_upstream")
```

### 4.3 D20.2 — Quantization INT8 dynamic

**Algorithme** :

```python
from onnxruntime.quantization import quantize_dynamic, QuantType

quantize_dynamic(
    model_input=str(out_dir / "weights/model.onnx"),
    model_output=str(out_dir / "weights/model.onnx.int8"),
    weight_type=QuantType.QInt8,            # signed int8 (-128..127)
    op_types_to_quantize=[
        "MatMul",                            # main accelerator gain
        "Gather",                            # embedding lookup
    ],
    per_channel=False,                       # dynamic = per-tensor (smaller, faster)
    reduce_range=False,                      # full int8 range
    extra_options={
        "WeightSymmetric": True,             # symmetric quant for int8
        "ActivationSymmetric": False,        # asymmetric for activations
    },
)
# Replace original
shutil.move(out_dir / "weights/model.onnx.int8",
            out_dir / "weights/model.onnx")
```

**Validation post-quant** :

```python
import onnx
model = onnx.load(out_dir / "weights/model.onnx")
onnx.checker.check_model(model)              # raises if malformed
# Inspect quantization markers
quantized_ops = [
    n for n in model.graph.node
    if n.op_type in ("MatMulInteger", "QGemm", "DynamicQuantizeLinear")
]
assert len(quantized_ops) > 0, "INT8 quantization didn't produce QInt-typed ops"
```

**Cosine similarity gate** (post-quant accuracy check) :

```python
# Run a few sentences through both fp32 + int8, ensure cosine ≥ 0.99
import onnxruntime as ort
sess_fp32 = ort.InferenceSession("upstream/model.onnx")
sess_int8 = ort.InferenceSession(out_dir / "weights/model.onnx")
sentences = ["Hello world", "The quick brown fox", "Semantic similarity"]
for s in sentences:
    e_fp32 = encode(sess_fp32, s)
    e_int8 = encode(sess_int8, s)
    cos = np.dot(e_fp32, e_int8) / (np.linalg.norm(e_fp32) * np.linalg.norm(e_int8))
    assert cos > 0.99, f"INT8 quant degraded similarity to {cos:.4f}"
```

### 4.4 D20.3 — Manifest D16 v1.0 (full schema)

```json
{
  "manifest_version": "1.0",
  "bundle_id": "sentence-transformers/all-MiniLM-L6-v2@1.0.0/onnx/384",
  "framework": "onnx",
  "dim": 384,
  "components": [
    {
      "path": "weights/model.onnx",
      "sha256": "<filled by build script>",
      "size_bytes": 29360128,
      "required": true
    },
    {
      "path": "tokenizer/tokenizer.json",
      "sha256": "<filled>",
      "size_bytes": 712431,
      "required": true
    },
    {
      "path": "tokenizer/tokenizer_config.json",
      "sha256": "<filled>",
      "size_bytes": 1235,
      "required": true
    },
    {
      "path": "tokenizer/special_tokens_map.json",
      "sha256": "<filled>",
      "size_bytes": 695,
      "required": true
    },
    {
      "path": "tokenizer/vocab.txt",
      "sha256": "<filled>",
      "size_bytes": 231508,
      "required": true
    },
    {
      "path": "runtime/runtime_config.json",
      "sha256": "<filled>",
      "size_bytes": 312,
      "required": true
    },
    {
      "path": "LICENSE",
      "sha256": "<filled>",
      "size_bytes": 11357,
      "required": false
    },
    {
      "path": "README.md",
      "sha256": "<filled>",
      "size_bytes": 2456,
      "required": false
    }
  ],
  "runtime": {
    "max_seq_len": 256,
    "pooling": "mean",
    "normalize": true,
    "dtype": "float32"
  },
  "build": {
    "builder": "https://github.com/efe-project/efe/actions/runs/<RUN_ID>",
    "built_at_unix_ms": 1745875200000,
    "git_sha": "<40-char SHA>",
    "source_date_epoch": 1745875200
  },
  "license": {
    "spdx_id": "Apache-2.0",
    "url": "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/blob/main/LICENSE",
    "redistribution_allowed": true
  },
  "model_origin": {
    "huggingface_repo": "sentence-transformers/all-MiniLM-L6-v2",
    "huggingface_revision": "e4ce9877abf3edfe10b0d82785e83bdcb973e22e",
    "model_card_url": "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2"
  },
  "sbom_path": "SBOM.json"
}
```

### 4.5 D20.4 — Ed25519 signature

```python
def phase_sign(manifest_path: Path, *, privkey_path: Path) -> Path:
    """Phase D20.4 — Ed25519 detached signature.

    Pin tested by `test_d20_signature_valid_against_pinned_pubkey`.
    """
    privkey = _load_ed25519_privkey(privkey_path)
    manifest_bytes = manifest_path.read_bytes()
    signature = privkey.sign(manifest_bytes)
    sig_path = manifest_path.parent / "manifest.json.sig"
    sig_path.write_bytes(signature)            # 64 raw bytes
    return sig_path
```

**Verification end-to-end** (post-build CI step) :

```bash
python3 -c "
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
import base64

# Public key from CI artefact (base64-armored 32 bytes)
pubkey_b64 = '<filled by CI>'
pubkey = Ed25519PublicKey.from_public_bytes(base64.b64decode(pubkey_b64))

# Verify
manifest = open('bundles/all-MiniLM-L6-v2-en-1.0.0/manifest.json', 'rb').read()
signature = open('bundles/all-MiniLM-L6-v2-en-1.0.0/manifest.json.sig', 'rb').read()
pubkey.verify(signature, manifest)  # raises InvalidSignature if mismatch
print('VERIFY_OK')
"
```

### 4.6 D20.5 — LICENSE + README

**LICENSE** : copie verbatim de
`huggingface.co/sentence-transformers/all-MiniLM-L6-v2/blob/main/LICENSE`
(Apache 2.0 standard).

**README.md** :

```markdown
# all-MiniLM-L6-v2-en bundle (1.0.0)

EFE-packaged bundle of `sentence-transformers/all-MiniLM-L6-v2`,
pinned at HF revision `e4ce9877abf3edfe10b0d82785e83bdcb973e22e`,
quantized to INT8 dynamic for production EFE deployments.

## Provenance

- Source: https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2
- License: Apache-2.0
- Built by: GitHub Actions run <URL>
- Build SHA (manifest): `<filled>`

## Usage

```bash
helm install efe charts/efe \
    --set config.embedder.backend=bundle \
    --set config.embedder.bundle.existingSecretName=efe-bundle-mini \
    --set bundle.slug=all-MiniLM-L6-v2-en \
    --set bundle.version=1.0.0 \
    --set bundle.manifestSha256=<filled>
```

## Verification

```bash
oras pull oci://registry.efe.ai/bundles/all-MiniLM-L6-v2-en:1.0.0 \
    -o /tmp/bundle/
cosign verify-blob \
    --signature /tmp/bundle/manifest.json.sig \
    --key /etc/efe/pubkeys/efe-prod-bundle-2026q1.pub \
    /tmp/bundle/manifest.json
```
```

### 4.7 D20.7 — Pin tests (~8 tests)

```python
# scripts/tests/test_d20_sample_bundle.py

def test_d20_bundle_directory_present() -> None:
    """The canonical sample bundle directory must exist."""
    assert (REPO / "bundles/all-MiniLM-L6-v2-en-1.0.0").is_dir()

def test_d20_bundle_required_files_present() -> None:
    """All D16 v1.0 required components must be on disk."""
    bundle = REPO / "bundles/all-MiniLM-L6-v2-en-1.0.0"
    for relpath in (
        "manifest.json",
        "manifest.json.sig",
        "weights/model.onnx",
        "tokenizer/tokenizer.json",
        "tokenizer/tokenizer_config.json",
        "tokenizer/special_tokens_map.json",
        "tokenizer/vocab.txt",
        "runtime/runtime_config.json",
        "LICENSE",
        "README.md",
    ):
        assert (bundle / relpath).is_file(), f"missing : {relpath}"

def test_d20_manifest_schema_v1_0() -> None:
    manifest = json.loads((BUNDLE / "manifest.json").read_text())
    assert manifest["manifest_version"] == "1.0"
    assert manifest["framework"] == "onnx"
    assert manifest["dim"] == 384

def test_d20_manifest_per_file_sha256_matches_disk() -> None:
    manifest = json.loads((BUNDLE / "manifest.json").read_text())
    for comp in manifest["components"]:
        if comp["required"]:
            actual = sha256_file(BUNDLE / comp["path"])
            assert actual == comp["sha256"], (
                f"hash drift on {comp['path']}: "
                f"manifest={comp['sha256']}, disk={actual}"
            )

def test_d20_signature_verifies_against_pinned_pubkey() -> None:
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
    import base64
    pubkey = Ed25519PublicKey.from_public_bytes(
        base64.b64decode(PINNED_PUBKEY_B64)
    )
    pubkey.verify(
        (BUNDLE / "manifest.json.sig").read_bytes(),
        (BUNDLE / "manifest.json").read_bytes(),
    )  # raises InvalidSignature on failure

def test_d20_bundle_total_size_under_30mib() -> None:
    total = sum(f.stat().st_size for f in BUNDLE.rglob("*") if f.is_file())
    assert total <= 30 * (1 << 20), (
        f"bundle size {total:,} > 30 MiB target"
    )

def test_d20_license_spdx_apache_2_0() -> None:
    manifest = json.loads((BUNDLE / "manifest.json").read_text())
    assert manifest["license"]["spdx_id"] == "Apache-2.0"
    assert manifest["license"]["redistribution_allowed"] is True

def test_d20_hf_revision_pinned_not_main() -> None:
    """Threat T-M13-7 anti-regression."""
    manifest = json.loads((BUNDLE / "manifest.json").read_text())
    rev = manifest["model_origin"]["huggingface_revision"]
    assert rev != "main"
    assert len(rev) == 40  # full SHA, not abbreviated
    assert all(c in "0123456789abcdef" for c in rev)
```

---

## 5. D26 — Bundle production `paraphrase-multilingual-MiniLM-L12-v2-1.0.0`

### 5.1 Sub-sprint inventory (D26 = J120, ~2 semaines d'effort)

| Sub-sprint | Phase | Output | Deps |
|---|---|---|---|
| **D26.1** | Pull modèle multilingual upstream | ONNX 250 MiB FP32 + tokenizer XLM-R + sentencepiece | HF access |
| **D26.2** | FP16 quantization (vs INT8 D20) | `model.onnx` ~78 MiB FP16 | onnxruntime |
| **D26.3** | Tokenizer config multilingual | sentencepiece.bpe.model + tokenizer_config | upstream |
| **D26.4** | Manifest write D16 v1.0 | `manifest.json` | `build_real_bundle.py` (NEW) |
| **D26.5** | SBOM CycloneDX 1.5 generation | `SBOM.json` | `cyclonedx-bom>=4.0` |
| **D26.6** | License compliance gate | LICENSE Apache-2.0 + ack | manuel |
| **D26.7** | Cross-lingual evaluation | `tests/fixtures/multilingual_eval.jsonl` | F1 ≥ 0.80 |
| **D26.8** | Ed25519 signature | `manifest.json.sig` | privkey |
| **D26.9** | Pin tests (~10 tests) | tests `test_d26_*` | pytest |

### 5.2 D26.1 — Pull multilingual model

```python
HF_REPO = "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2"
HF_REVISION = "bf3bf13ab40c3157080a7ab344c831b9ad18b5eb"  # PINNED

snapshot_download(
    repo_id=HF_REPO,
    revision=HF_REVISION,
    local_dir=staging_dir,
    allow_patterns=[
        "tokenizer.json",
        "tokenizer_config.json",
        "special_tokens_map.json",
        "sentencepiece.bpe.model",   # XLM-R subword
        "onnx/model.onnx",
        "config.json",
    ],
)
```

### 5.3 D26.2 — FP16 quantization (vs D20 INT8)

**Pourquoi FP16 et pas INT8 pour D26 ?**

- Le modèle multilingual a un vocab 250k+ (vs 30k pour D20). INT8
  quantization du `Embedding` layer dégrade fortement les
  similarities cross-lingual.
- FP16 produit ~50% reduction (250 MiB → 80 MiB) avec ~99.9%
  cosine similarity vs FP32 baseline.
- Trade-off accepté : +50 MiB on-disk vs D20, mais qualité
  multilingual préservée.

```python
from onnx import helper, numpy_helper
import numpy as np

def fp16_quantize_model(input_path: Path, output_path: Path) -> None:
    model = onnx.load(str(input_path))
    for init in model.graph.initializer:
        if init.data_type == onnx.TensorProto.FLOAT:
            arr = numpy_helper.to_array(init)
            arr_fp16 = arr.astype(np.float16)
            new_init = numpy_helper.from_array(arr_fp16, init.name)
            new_init.data_type = onnx.TensorProto.FLOAT16
            init.CopyFrom(new_init)
    # Update ValueInfo + IO types
    for io in list(model.graph.input) + list(model.graph.output):
        if io.type.tensor_type.elem_type == onnx.TensorProto.FLOAT:
            io.type.tensor_type.elem_type = onnx.TensorProto.FLOAT16
    onnx.save(model, str(output_path))
```

**Cosine similarity gate** (post-FP16) :

```python
sentences = [
    ("Hello world", "Bonjour le monde"),       # en/fr translation
    ("How are you?", "¿Cómo estás?"),          # en/es translation
    ("Good morning", "Guten Morgen"),          # en/de translation
    ("Thank you", "Grazie"),                   # en/it translation
    ("See you", "Até logo"),                   # en/pt translation
]
for en, other in sentences:
    e_fp32_en = encode(sess_fp32, en)
    e_fp16_en = encode(sess_fp16, en)
    e_fp32_other = encode(sess_fp32, other)
    e_fp16_other = encode(sess_fp16, other)

    # FP32 baseline cross-lingual
    cos_fp32 = cosine(e_fp32_en, e_fp32_other)
    # FP16 cross-lingual
    cos_fp16 = cosine(e_fp16_en, e_fp16_other)

    # FP16 must preserve cross-lingual similarity within 0.02
    assert abs(cos_fp32 - cos_fp16) < 0.02, (
        f"FP16 degraded {en}/{other}: fp32={cos_fp32:.4f}, fp16={cos_fp16:.4f}"
    )
```

### 5.4 D26.5 — SBOM CycloneDX 1.5 generation

**Schema target** : CycloneDX 1.5 + ML-BOM extension.

```python
import cyclonedx_python_lib as cd
from cyclonedx_python_lib.model.component import Component, ComponentType
from cyclonedx_python_lib.model.bom import Bom
from cyclonedx_python_lib.output.json import JsonV1Dot5

bom = Bom()

# Main ML model component
ml_model = Component(
    type=ComponentType.MACHINE_LEARNING_MODEL,
    bom_ref="sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2@1.0.0",
    name="paraphrase-multilingual-MiniLM-L12-v2",
    version="1.0.0",
    publisher="sentence-transformers (UKPLab)",
    licenses=[License(spdx_license_id="Apache-2.0")],
    purl="pkg:huggingface/sentence-transformers/"
         "paraphrase-multilingual-MiniLM-L12-v2"
         "@bf3bf13ab40c3157080a7ab344c831b9ad18b5eb",
    cpe=None,
    description=(
        "Sentence transformer for multilingual paraphrase similarity. "
        "Distilled from XLM-R, 12-layer MiniLM architecture, 384-dim output. "
        "Supports 50+ languages via direct training, 100+ via cross-lingual transfer."
    ),
    properties=[
        Property("ml.model.architecture", "MiniLM-L12"),
        Property("ml.model.parameters", "~118M"),
        Property("ml.model.training_data",
                 "ParallelCorpora 50+ languages — see model card"),
        Property("ml.model.framework", "onnx"),
        Property("ml.model.quantization", "fp16"),
        Property("efe.bundle.id",
                 "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2"
                 "@1.0.0/onnx/384"),
    ],
)
bom.components.add(ml_model)

# Tokenizer data component
tokenizer = Component(
    type=ComponentType.DATA,
    bom_ref="paraphrase-multilingual-MiniLM-L12-v2/tokenizer",
    name="paraphrase-multilingual-MiniLM-L12-v2-tokenizer",
    version="1.0.0",
    publisher="sentence-transformers (UKPLab)",
    description="XLM-R-based sentencepiece tokenizer, vocab 250002.",
    licenses=[License(spdx_license_id="Apache-2.0")],
    properties=[
        Property("nlp.tokenizer.algorithm", "sentencepiece-bpe"),
        Property("nlp.tokenizer.vocab_size", "250002"),
        Property("nlp.tokenizer.subword_model_path",
                 "tokenizer/sentencepiece.bpe.model"),
    ],
)
bom.components.add(tokenizer)

# Build dependencies (the runtime deps, not the model deps)
build_deps = [
    ("onnxruntime", "1.16.3"),
    ("onnx", "1.15.0"),
    ("transformers", "4.35.2"),
    ("sentence-transformers", "2.2.2"),
    ("numpy", "1.26.0"),
    ("torch", "2.1.0"),
]
for name, version in build_deps:
    bom.components.add(Component(
        type=ComponentType.LIBRARY,
        bom_ref=f"build/{name}@{version}",
        name=name,
        version=version,
        purl=f"pkg:pypi/{name}@{version}",
    ))

# Provenance (build metadata)
bom.metadata.tools.add(Tool(
    vendor="EFE Project",
    name="build_real_bundle.py",
    version="1.0.0",
    hashes=[Hash(alg="SHA-256", content="<sha256 of build script>")],
))
bom.metadata.timestamp = datetime.fromtimestamp(
    SOURCE_DATE_EPOCH, tz=timezone.utc
)
bom.metadata.component = Component(
    type=ComponentType.APPLICATION,
    bom_ref="bundle/paraphrase-multilingual-MiniLM-L12-v2@1.0.0",
    name="paraphrase-multilingual-MiniLM-L12-v2",
    version="1.0.0",
)

# Output
output = JsonV1Dot5(bom)
output.output_to_file(out_dir / "SBOM.json", indent=2)
```

**Validation** :

```bash
cyclonedx-cli validate \
    --input-file bundles/paraphrase-multilingual-MiniLM-L12-v2-1.0.0/SBOM.json \
    --schema-version 1.5 \
    --fail-on-errors
```

### 5.5 D26.7 — Cross-lingual evaluation gate

**Test fixture** : `tests/fixtures/multilingual_eval.jsonl` :

```jsonl
{"lang_pair":"en-fr","src":"Hello world","tgt":"Bonjour le monde","cos_min":0.85}
{"lang_pair":"en-de","src":"Hello world","tgt":"Hallo Welt","cos_min":0.85}
{"lang_pair":"en-es","src":"Hello world","tgt":"Hola mundo","cos_min":0.85}
{"lang_pair":"en-it","src":"Hello world","tgt":"Ciao mondo","cos_min":0.85}
{"lang_pair":"en-pt","src":"Hello world","tgt":"Olá mundo","cos_min":0.85}
{"lang_pair":"en-fr","src":"How are you?","tgt":"Comment allez-vous?","cos_min":0.80}
... (100 paires couvrant 5 langues × 20 sentences chacune)
{"lang_pair":"unrelated","src":"Hello world","tgt":"The mitochondria is the powerhouse of the cell","cos_max":0.40}
... (50 contre-exemples)
```

**Test** :

```python
def test_d26_cross_lingual_f1_above_080() -> None:
    fixtures = list(jsonl_load("tests/fixtures/multilingual_eval.jsonl"))
    sess = ort.InferenceSession(BUNDLE / "weights/model.onnx")
    tokenizer = load_tokenizer(BUNDLE / "tokenizer")

    correct = 0
    total = 0
    for fix in fixtures:
        e_src = encode(sess, tokenizer, fix["src"])
        e_tgt = encode(sess, tokenizer, fix["tgt"])
        cos = cosine_sim(e_src, e_tgt)
        if "cos_min" in fix:
            if cos >= fix["cos_min"]:
                correct += 1
        elif "cos_max" in fix:
            if cos <= fix["cos_max"]:
                correct += 1
        total += 1
    f1 = correct / total
    assert f1 >= 0.80, f"D26 cross-lingual F1 {f1:.4f} < 0.80 threshold"
```

### 5.6 D26 manifest concrete

```json
{
  "manifest_version": "1.0",
  "bundle_id": "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2@1.0.0/onnx/384",
  "framework": "onnx",
  "dim": 384,
  "components": [
    { "path": "weights/model.onnx",
      "sha256": "<filled>", "size_bytes": 81788928, "required": true },
    { "path": "tokenizer/tokenizer.json",
      "sha256": "<filled>", "size_bytes": 17082921, "required": true },
    { "path": "tokenizer/sentencepiece.bpe.model",
      "sha256": "<filled>", "size_bytes": 5069051, "required": true },
    { "path": "tokenizer/tokenizer_config.json",
      "sha256": "<filled>", "size_bytes": 1235, "required": true },
    { "path": "tokenizer/special_tokens_map.json",
      "sha256": "<filled>", "size_bytes": 695, "required": true },
    { "path": "runtime/runtime_config.json",
      "sha256": "<filled>", "size_bytes": 312, "required": true },
    { "path": "SBOM.json",
      "sha256": "<filled>", "size_bytes": 4521, "required": true },
    { "path": "LICENSE",
      "sha256": "<filled>", "size_bytes": 11357, "required": false },
    { "path": "README.md",
      "sha256": "<filled>", "size_bytes": 3120, "required": false }
  ],
  "runtime": {
    "max_seq_len": 128,
    "pooling": "mean",
    "normalize": true,
    "dtype": "float16"
  },
  "build": {
    "builder": "https://github.com/efe-project/efe/actions/runs/<RUN>",
    "built_at_unix_ms": <SDE_MS>,
    "git_sha": "<40 chars>",
    "source_date_epoch": <SDE>
  },
  "license": {
    "spdx_id": "Apache-2.0",
    "url": "https://huggingface.co/sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2/blob/main/LICENSE",
    "redistribution_allowed": true
  },
  "model_origin": {
    "huggingface_repo": "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2",
    "huggingface_revision": "bf3bf13ab40c3157080a7ab344c831b9ad18b5eb",
    "model_card_url": "https://huggingface.co/sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2"
  },
  "sbom_path": "SBOM.json"
}
```

---

## 6. D27 — Pipeline CI auto-bundling

### 6.1 Sub-sprint inventory (D27 = J140, ~1 semaine d'effort)

| Sub-sprint | Phase | Output |
|---|---|---|
| **D27.1** | Workflow file `.github/workflows/bundle-release.yml` | YAML 250+ LOC |
| **D27.2** | Trigger : tag `bundle-v*.*.*` + workflow_dispatch | event filter |
| **D27.3** | Job 1 : Setup + checkout + tooling | Python + ORAS + cosign |
| **D27.4** | Job 2 : Build + reproducibility check | bundle artefact |
| **D27.5** | Job 3 : Sign Ed25519 + verify | manifest.json.sig |
| **D27.6** | Job 4 : SBOM CycloneDX generation | SBOM.json |
| **D27.7** | Job 5 : License gate + scan | exit 1 si license non-allowlisted |
| **D27.8** | Job 6 : OCI publish via ORAS | OCI artifact pushed |
| **D27.9** | Job 7 : Post-publish verify (pull + verify) | smoke test |
| **D27.10** | Job 8 : Notify (Slack + alerting) | webhook |

### 6.2 D27.2 — Triggers

```yaml
on:
  push:
    tags:
      - "bundle-v*.*.*"           # e.g., bundle-v1.0.0
  workflow_dispatch:
    inputs:
      bundle_slug:
        description: "Bundle slug (e.g., all-MiniLM-L6-v2-en-1.0.0)"
        required: true
        type: string
      bundle_version:
        description: "Semver version (e.g., 1.0.0)"
        required: true
        type: string
      mode:
        description: "private | public (private until D40 GA)"
        required: false
        default: "private"
        type: choice
        options: [private, public]
      hf_revision:
        description: "Pinned HF revision SHA (40 chars)"
        required: true
        type: string
```

### 6.3 D27.5 — Signing key management

**Sources de privkey, par ordre de priorité** :

1. **GitHub Repository Secret** `EFE_BUNDLE_PRIVKEY_PEM` (preferred for self-hosted GHCR)
   - PEM-encoded Ed25519 private key
   - Set via `gh secret set EFE_BUNDLE_PRIVKEY_PEM < privkey.pem`
2. **GitHub Environment Secret** (per-environment isolation, e.g. `production`)
   - Same key but scoped via `environment: production` in the job
3. **Cloud KMS reference** `EFE_BUNDLE_KMS_KEY_REF` (pour orgs pouvant utiliser AWS KMS / GCP KMS / HashiCorp Vault)
   - Format : `awskms:///arn:aws:kms:us-east-1:123:key/abc-...` ou `gcpkms://projects/<P>/locations/<L>/keyRings/<KR>/cryptoKeys/<K>/cryptoKeyVersions/<V>`
   - Le pipeline appelle `cosign sign-blob --key <KMS_REF>` qui délègue le signing au KMS

**Rotation procedure** :

```
T=0       T=+30j         T=+90j        T=+120j
│         │              │              │
old key   ─── verify ────────── verify ─┤
                          │              ── (expired)
              new key  ── verify ───────────────────── verify ──
              new key  ── sign  ───────────────────── sign  ──
```

1. Generate new keypair locally (HSM-backed or `cosign generate-key-pair`)
2. Upload pubkey to `pubkeys.json` trust-store with `valid_from=T+30j`
3. After 30 days, set new privkey as `active_for_signing=true` in CI
4. After 90 more days, retire old key (set `active_for_signing=false`)
5. After 120 more days, remove old pubkey from trust-store

### 6.4 D27.4 — Reproducibility check

**Algorithme** :

```yaml
- name: Build (run 1)
  run: |
    export SOURCE_DATE_EPOCH=$(git log -1 --format=%ct HEAD)
    python3 scripts/release/build_sample_bundle.py \
        --out _build1/${{ inputs.bundle_slug }} \
        --hf-revision ${{ inputs.hf_revision }} \
        --privkey-path /dev/null     # skip signing in run 1
    sha256sum _build1/${{ inputs.bundle_slug }}/manifest.json > _build1.sha

- name: Build (run 2 — independent)
  run: |
    export SOURCE_DATE_EPOCH=$(git log -1 --format=%ct HEAD)
    python3 scripts/release/build_sample_bundle.py \
        --out _build2/${{ inputs.bundle_slug }} \
        --hf-revision ${{ inputs.hf_revision }} \
        --privkey-path /dev/null
    sha256sum _build2/${{ inputs.bundle_slug }}/manifest.json > _build2.sha

- name: Verify reproducibility
  run: |
    if ! diff _build1.sha _build2.sha; then
      echo "::error::REPRODUCIBILITY_FAIL — manifest SHA256 differs across runs"
      diff -ru _build1/ _build2/ | head -200
      exit 3
    fi
    echo "::notice::REPRODUCIBILITY_OK — SHA256 matches across runs"
```

### 6.5 D27.8 — OCI publish via ORAS

**Pourquoi ORAS** ?

- Standard OCI 1.1 Distribution + Artifacts spec
- Compatible GHCR, Docker Hub, AWS ECR, Harbor, registry self-hosted
- CLI mature (`oras-project/oras` v1.0+)
- Supporte les annotations OCI custom (`org.opencontainers.artifact.type`)

**Layout OCI** :

```
manifest:
  schemaVersion: 2
  mediaType: application/vnd.oci.image.manifest.v1+json
  artifactType: application/vnd.efe.bundle.v1+json
  config:
    mediaType: application/vnd.efe.bundle.config.v1+json
    digest: sha256:<config blob digest>
  layers:
    - mediaType: application/vnd.efe.bundle.manifest.v1+json
      digest: sha256:<manifest.json sha256>
      annotations:
        org.opencontainers.image.title: manifest.json
    - mediaType: application/octet-stream
      digest: sha256:<manifest.sig sha256>
      annotations:
        org.opencontainers.image.title: manifest.json.sig
    - mediaType: application/vnd.onnx.model.v1
      digest: sha256:<model.onnx sha256>
      annotations:
        org.opencontainers.image.title: weights/model.onnx
    - mediaType: application/vnd.huggingface.tokenizer.v1+json
      digest: sha256:<tokenizer.json sha256>
      annotations:
        org.opencontainers.image.title: tokenizer/tokenizer.json
    - mediaType: application/vnd.cyclonedx+json
      digest: sha256:<sbom.json sha256>
      annotations:
        org.opencontainers.image.title: SBOM.json
    ... (autres layers)
  annotations:
    org.opencontainers.artifact.created: "<ISO8601>"
    org.opencontainers.image.source: "https://github.com/efe-project/efe"
    org.opencontainers.image.revision: "<git_sha>"
    efe.bundle.id: "sentence-transformers/all-MiniLM-L6-v2@1.0.0/onnx/384"
    efe.bundle.manifest_sha256: "<hex>"
    efe.bundle.signature_algo: "ed25519"
    efe.bundle.signing_key_id: "efe-prod-bundle-2026q1"
```

**Commande ORAS push** :

```bash
oras push \
    oci://${REGISTRY}/bundles/${BUNDLE_SLUG}:${BUNDLE_VERSION} \
    --artifact-type "application/vnd.efe.bundle.v1+json" \
    --annotation "efe.bundle.id=${BUNDLE_ID}" \
    --annotation "efe.bundle.manifest_sha256=${MANIFEST_SHA}" \
    --annotation "efe.bundle.signature_algo=ed25519" \
    --annotation "efe.bundle.signing_key_id=${SIGNING_KEY_ID}" \
    --annotation "org.opencontainers.image.source=https://github.com/${GITHUB_REPOSITORY}" \
    --annotation "org.opencontainers.image.revision=${GIT_SHA}" \
    "manifest.json:application/vnd.efe.bundle.manifest.v1+json" \
    "manifest.json.sig:application/octet-stream" \
    "weights/model.onnx:application/vnd.onnx.model.v1" \
    "tokenizer/tokenizer.json:application/vnd.huggingface.tokenizer.v1+json" \
    "tokenizer/tokenizer_config.json:application/json" \
    "tokenizer/special_tokens_map.json:application/json" \
    "tokenizer/vocab.txt:text/plain" \
    "runtime/runtime_config.json:application/json" \
    "SBOM.json:application/vnd.cyclonedx+json" \
    "LICENSE:text/plain" \
    "README.md:text/markdown"
```

### 6.6 D27.9 — Post-publish verify

```yaml
- name: Pull artifact + verify integrity
  run: |
    mkdir _verify/
    oras pull oci://${REGISTRY}/bundles/${BUNDLE_SLUG}:${BUNDLE_VERSION} \
        -o _verify/

    # 1. Verify manifest sig
    cosign verify-blob \
        --signature _verify/manifest.json.sig \
        --key /tmp/pubkey-${SIGNING_KEY_ID}.pem \
        _verify/manifest.json

    # 2. Verify per-file SHA256 against manifest claims
    python3 - <<'PY'
    import json, hashlib
    from pathlib import Path
    m = json.loads(Path("_verify/manifest.json").read_text())
    for c in m["components"]:
        f = Path("_verify") / c["path"]
        if not f.exists():
            if c["required"]:
                raise SystemExit(f"FAIL: required {c['path']} missing")
            continue
        actual = hashlib.sha256(f.read_bytes()).hexdigest()
        if actual != c["sha256"]:
            raise SystemExit(
                f"FAIL: {c['path']} hash drift "
                f"manifest={c['sha256']}, disk={actual}"
            )
    print("INTEGRITY_OK")
    PY
```

---

## 7. OCI Artifacts publication mechanism

### 7.1 Pourquoi OCI Artifacts vs alternatives

| Option | Pros | Cons | Décision |
|---|---|---|---|
| **OCI Artifacts (ORAS)** | Standard ouvert, immutable tags, multi-registry, signing native (cosign) | Tooling encore jeune (ORAS 1.0+) | ✅ **CHOSEN** |
| GitHub Releases | Simple, gratuit | 2 GB max par fichier, pas immutable, no signing native | ❌ |
| S3 + signed URL | Scalable | Pas de versioning natif, no immutability sans Object Lock, no signing chain | ❌ |
| HuggingFace Hub repo (private) | Familier ML | Lock-in, no SLAs, license/redistribution complexe | ❌ |
| Self-hosted Nexus / Artifactory | Enterprise-friendly | Coût licence, ops overhead | Future M14+ option |

### 7.2 Registry endpoints

**Pre-GA (M13 livraison)** :
```
ghcr.io/efe-project/bundles-private/<slug>:<version>
```
- Authentication : GH PAT scope `read:packages` + `write:packages`
- Visibility : private
- Retention : 365 jours par défaut (GHCR)

**GA (post-D40)** :
```
registry.efe.ai/bundles/<slug>:<version>
```
- Authentication : OIDC token via `EFE_REGISTRY_TOKEN`
- Visibility : public
- Retention : permanent (Object Lock COMPLIANCE)

### 7.3 Tag immutability

Configuration GHCR pour M13 (M13-A1 anti-régression) :

```yaml
# .github/settings.yml
packages:
  - name: bundles-private
    visibility: private
    delete_branch_protection_rule: false  # immutable tags
    restrict_pushes_to_protected_branches: true
```

Self-hosted alternative (Harbor) :
```yaml
project:
  name: bundles
  immutable_tags:
    - .*  # all tags immutable
  retention_policy:
    rules:
      - action: retain
        params: { latestPushedK: 100 }
```

### 7.4 OCI annotations canoniques

```
org.opencontainers.image.created          ISO 8601 timestamp
org.opencontainers.image.source           GitHub repo URL
org.opencontainers.image.revision         Git SHA
org.opencontainers.image.title            Bundle slug
org.opencontainers.image.description      Brief description
org.opencontainers.image.licenses         Apache-2.0
org.opencontainers.image.documentation    README.md URL
efe.bundle.id                             D16 §3.2 canonical bundle ID
efe.bundle.manifest_sha256                bundle_fingerprint
efe.bundle.signature_algo                 ed25519
efe.bundle.signing_key_id                 efe-prod-bundle-2026q1
efe.bundle.sbom_present                   true | false
efe.bundle.framework                      onnx | safetensors
efe.bundle.dim                            384 | 768 | ...
efe.bundle.dtype                          float32 | float16 | ...
```

### 7.5 Cosign signature on OCI artifact

En complément du `manifest.json.sig` interne au bundle, le pipeline
peut signer **l'OCI artifact lui-même** avec cosign :

```bash
cosign sign --key env://COSIGN_PRIVATE_KEY \
    oci://${REGISTRY}/bundles/${BUNDLE_SLUG}:${BUNDLE_VERSION}
```

Cela produit une signature détachée qui vit dans le registry sous
le tag `:<version>.sig`. Verify côté consumer :

```bash
cosign verify --key /etc/efe/cosign.pub \
    oci://registry.efe.ai/bundles/${BUNDLE_SLUG}:${BUNDLE_VERSION}
```

**Double signature** = defense in depth : compromise du manifest
sig + cosign sig requiert deux compromises distincts.

---

## 8. Manifest schema instances concrètes

(Voir §4.4 et §5.6 pour les exemples D20 et D26 complets.)

### 8.1 Différences D20 vs D26

| Champ | D20 | D26 |
|---|---|---|
| `bundle_id` | `sentence-transformers/all-MiniLM-L6-v2@1.0.0/onnx/384` | `sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2@1.0.0/onnx/384` |
| `runtime.dtype` | `float32` (post INT8 dequant) | `float16` (FP16 quantized) |
| `runtime.max_seq_len` | 256 | 128 |
| `quantization` indicator (custom) | int8-dynamic | fp16 |
| `components[].size_bytes` total | ~30 MiB | ~80 MiB |
| `sbom_path` | `SBOM.json` (deferred D26) | `SBOM.json` (mandatory) |
| `model_origin.huggingface_revision` | `e4ce9877...` | `bf3bf13a...` |

### 8.2 Validation contre JSON Schema

```python
import jsonschema
schema = json.loads((REPO / "crates/efe-server/schemas/d16_manifest.schema.json").read_text())
manifest = json.loads((BUNDLE / "manifest.json").read_text())
jsonschema.validate(instance=manifest, schema=schema)
```

Pin tested by `test_d20_manifest_validates_against_d16_schema`
+ `test_d26_manifest_validates_against_d16_schema`.

---

## 9. Signing & verification operational flow

### 9.1 Flow complet build → publish → verify

```
┌─────────────────────────────────────────────────────────────────┐
│  1. CI Build phase (GH Actions)                                 │
│     - Pull HF model (PINNED revision)                           │
│     - Quantize ONNX                                             │
│     - Compute per-file SHA256                                   │
│     - Write canonical manifest.json                             │
│     - Reproducibility check (build twice)                       │
└──────────────────┬──────────────────────────────────────────────┘
                   │
                   ▼
┌─────────────────────────────────────────────────────────────────┐
│  2. CI Sign phase                                               │
│     - Load privkey from GH Secret (mask immediately)            │
│     - Ed25519.sign(canonical manifest bytes) → manifest.sig    │
│     - cosign sign-blob (Rekor transparency log entry)           │
│     - Verify locally before push                                │
└──────────────────┬──────────────────────────────────────────────┘
                   │
                   ▼
┌─────────────────────────────────────────────────────────────────┐
│  3. CI Publish phase                                            │
│     - oras push to ghcr.io / registry.efe.ai                    │
│     - cosign sign on OCI manifest                               │
│     - Get digest back (immutable reference)                     │
└──────────────────┬──────────────────────────────────────────────┘
                   │
                   ▼
┌─────────────────────────────────────────────────────────────────┐
│  4. CI Verify phase                                             │
│     - Pull artifact (oras pull)                                 │
│     - Verify manifest.sig with pubkey                           │
│     - Verify per-file SHA256                                    │
│     - Verify cosign sig on OCI manifest                         │
│     - Verify Rekor transparency log entry                       │
└──────────────────┬──────────────────────────────────────────────┘
                   │
                   ▼
┌─────────────────────────────────────────────────────────────────┐
│  5. Production verify (efe-server pod boot, M12 D27 hook)       │
│     - Pull bundle from OCI to PVC / Secret                      │
│     - Verify manifest.sig                                       │
│     - Verify per-file SHA256                                    │
│     - Verify license SPDX in allowlist                          │
│     - Verify model.onnx via onnx.checker                        │
│     - Load into ONNX Runtime                                    │
│     - Refuse boot if any check fails                            │
└─────────────────────────────────────────────────────────────────┘
```

### 9.2 Signing key lifecycle

| Stage | Action | Operator | Audit trail |
|---|---|---|---|
| Generation | `cosign generate-key-pair --kms aws-kms://...` | Security team | KMS audit log |
| Activation | Set `active_for_signing=true` in `pubkeys.json` | DevOps lead | Git commit |
| Use | CI signs all bundles with active key | Automated | GitHub Actions log + Rekor |
| Rotation announce | Generate new key, add to trust-store with `valid_from=T+30j` | DevOps lead | Git commit |
| Cutover | New key becomes `active_for_signing` in CI | DevOps lead | CI config commit |
| Grace | Old key still verifies for 90 days | Automated | trust-store retention |
| Retirement | Remove from trust-store | DevOps lead | Git commit |

---

## 10. Reproducibility bit-for-bit (SOURCE_DATE_EPOCH)

### 10.1 Sources de non-déterminisme et mitigations

| Source | Cause | Mitigation |
|---|---|---|
| Timestamp `built_at` | `datetime.now()` | `SOURCE_DATE_EPOCH` env (Reproducible Builds spec) |
| File enumeration order | OS-dependent `os.listdir()` | `sorted(os.listdir())` partout |
| JSON serialization | Random key order | `json.dumps(..., sort_keys=True)` |
| Float representation | NumPy default | Pinned NumPy version via `requirements-build.txt` |
| ONNX optimizer pass | Non-deterministic op fusion | `--no-optimization` flag in ONNX Runtime quantize |
| Tokenizer compile | sentencepiece may shuffle | Pin sentencepiece version + use upstream pre-compiled |
| SBOM timestamp | CycloneDX library defaults | Override `bom.metadata.timestamp` to SOURCE_DATE_EPOCH |
| Tar/zip ordering | mtime-sensitive | N/A — bundle is a directory, not an archive |

### 10.2 Verification CI step

```yaml
- name: Reproducibility verify
  run: |
    # Build N=3 times, ensure all produce identical SHA256
    for i in 1 2 3; do
      rm -rf "_build${i}/"
      SOURCE_DATE_EPOCH=$(git log -1 --format=%ct) \
          python3 scripts/release/build_sample_bundle.py \
              --out "_build${i}/${SLUG}" \
              --hf-revision ${HF_REV}
      sha256sum "_build${i}/${SLUG}/manifest.json" \
          | awk '{print $1}' > "_build${i}.sha"
    done
    # All 3 must match
    sha1=$(cat _build1.sha)
    sha2=$(cat _build2.sha)
    sha3=$(cat _build3.sha)
    if [ "$sha1" != "$sha2" ] || [ "$sha2" != "$sha3" ]; then
      echo "::error::REPRODUCIBILITY_FAIL"
      echo "  build1: $sha1"
      echo "  build2: $sha2"
      echo "  build3: $sha3"
      exit 3
    fi
    echo "::notice::REPRODUCIBILITY_OK :: $sha1"
```

---

## 11. SBOM CycloneDX 1.5 (ML-BOM)

### 11.1 Pourquoi CycloneDX 1.5 et pas SPDX

| Critère | CycloneDX 1.5 | SPDX 2.3 |
|---|---|---|
| ML-Bom support natif | ✅ `type=machine-learning-model` | ❌ via extensions only |
| Vulnerability tracking | ✅ via `vulnerabilities[]` | ✅ via SPDX-2.3 |
| Adoption ML/AI ecosystem | ✅ référence pour Hugging Face Hub, OpenSSF | Mid-tier |
| Tooling | `cyclonedx-bom`, `cyclonedx-cli` mature | spdx-tools moins mature |
| Default format | JSON natif | tag-value puis JSON |

**Décision** : CycloneDX 1.5 pour M13 ; option SPDX exportable via
`cyclonedx-cli convert` pour les clients régulés exigeant SPDX.

### 11.2 Composants obligatoires dans le SBOM

| Component type | Bom-ref pattern | Champs obligatoires |
|---|---|---|
| `machine-learning-model` | `<family>/<name>@<version>` | name, version, publisher, license, purl, properties (architecture, params, training_data) |
| `data` (tokenizer) | `<bundle>/tokenizer` | name, version, license, properties (algo, vocab_size) |
| `library` (build deps) | `build/<pkg>@<v>` | name, version, purl |
| `application` (root component) | `bundle/<slug>@<v>` | name, version, type=application |

### 11.3 SBOM verification

```bash
# Validate schema
cyclonedx-cli validate \
    --input-file SBOM.json \
    --schema-version 1.5 \
    --fail-on-errors

# Sanity check ML-bom presence
jq -e '.components[] | select(.type == "machine-learning-model")' SBOM.json
```

---

## 12. License compliance & redistribution

### 12.1 Allowlist licenses M13

```python
M13_LICENSE_ALLOWLIST = {
    "Apache-2.0",       # default for sentence-transformers
    "MIT",
    "BSD-3-Clause",
    "BSD-2-Clause",
    "CC-BY-4.0",
    "CC0-1.0",
    "Apache-2.0-with-LLVM-exception",
}

M13_LICENSE_RESTRICTED = {
    # Requires operator ack via EFE_BUNDLE_ACK_NONREDIST=true
    "OpenRAIL-M",
    "OpenRAIL-S",
    "CC-BY-NC-4.0",
    "CC-BY-SA-4.0",
}

M13_LICENSE_FORBIDDEN = {
    # Refused even with ack
    "GPL-3.0-only",
    "AGPL-3.0-only",
    "Proprietary",
    "UNKNOWN",
}
```

### 12.2 Pre-publish license gate

```python
def license_gate(manifest: dict) -> None:
    spdx_id = manifest.get("license", {}).get("spdx_id")
    if not spdx_id:
        raise SystemExit("license.spdx_id missing in manifest")
    if spdx_id in M13_LICENSE_FORBIDDEN:
        raise SystemExit(
            f"License {spdx_id} is FORBIDDEN by M13 policy. "
            f"Refusing to publish."
        )
    if spdx_id in M13_LICENSE_RESTRICTED:
        ack = os.environ.get("EFE_BUNDLE_ACK_NONREDIST")
        if ack != "true":
            raise SystemExit(
                f"License {spdx_id} requires explicit ack "
                f"(EFE_BUNDLE_ACK_NONREDIST=true)"
            )
        manifest["license"]["redistribution_allowed"] = False
        return
    if spdx_id not in M13_LICENSE_ALLOWLIST:
        raise SystemExit(
            f"License {spdx_id} not in M13 allowlist. "
            f"Add it explicitly to ALLOWLIST or use --license-override."
        )
    manifest["license"]["redistribution_allowed"] = True
```

### 12.3 LICENSE file requirement

Tout bundle DOIT inclure le texte verbatim de sa license SPDX :

| SPDX | Source canonique |
|---|---|
| Apache-2.0 | https://www.apache.org/licenses/LICENSE-2.0.txt |
| MIT | https://opensource.org/licenses/MIT |
| BSD-3-Clause | https://opensource.org/licenses/BSD-3-Clause |

Pin tested by `test_d20_license_file_matches_spdx`
+ `test_d26_license_file_matches_spdx`.

---

## 13. Helm integration (post-M12)

### 13.1 Wirage avec M12 D27 bundle reload

M12 D27 a livré le `helm.sh/hook: post-upgrade` Job qui appelle
`/admin/bundle-reload` quand `.Values.bundle.manifestSha256` change.
M13 fournit les vraies valeurs de `manifestSha256`.

```yaml
# values.prod.yaml (post-M13)
config:
  embedder:
    backend: bundle
    bundle:
      path: /etc/efe/bundles/active
      requireSignature: required
      trustStorePath: /etc/efe/bundles/pubkeys.json
      requireSbom: true
      acknowledgeNonRedistributable: false
      loadTimeoutS: 60
      encodeTimeoutMs: 500
      existingSecretName: "efe-bundle-paraphrase-multilingual"
      trustStoreSecretName: "efe-bundle-pubkeys"

bundle:
  slug: paraphrase-multilingual-MiniLM-L12-v2
  version: "1.0.0"
  manifestSha256: "<filled at M13 D26 release>"

bundleReload:
  enabled: true
  hookTiming: post-upgrade
  image:
    repository: bitnami/kubectl
    tag: "1.30"
    pullPolicy: IfNotPresent
  timeout: 300
  retries: 3
  adminEndpoint:
    service: ""
    port: 8080
    path: /admin/bundle-reload
    bundleInfoPath: /admin/bundle-info
```

### 13.2 Bundle distribution to K8s pods

**Option A : OCI pull init container** (default M13)

```yaml
initContainers:
  - name: bundle-pull
    image: oras-project/oras:v1.0.0
    command:
      - /bin/sh
      - -c
      - |
        oras pull oci://registry.efe.ai/bundles/${BUNDLE_SLUG}:${BUNDLE_VERSION} \
            -o /etc/efe/bundles/active/
    volumeMounts:
      - name: bundle-active
        mountPath: /etc/efe/bundles/active
volumes:
  - name: bundle-active
    emptyDir: {}
```

**Option B : K8s Secret mount** (legacy D16, retained for compatibility)

```yaml
volumes:
  - name: bundle
    secret:
      secretName: efe-bundle-paraphrase-multilingual
      defaultMode: 0444
```

**Option C : ReadOnlyMany PVC** (pour déploiements multi-réplica scaling)

```yaml
volumes:
  - name: bundle
    persistentVolumeClaim:
      claimName: efe-bundle-shared
      readOnly: true
```

### 13.3 Helm values.m13-bundle.yaml fragment

```yaml
# deploy/helm/efe/values.m13-bundle.yaml — fragment opt-in
bundles:
  enabled: false  # opt-in M13 ; default uses M12 D27 bundle reload only
  # OCI pull config
  ociPull:
    enabled: false
    registry: "registry.efe.ai/bundles"
    image:
      repository: oras-project/oras
      tag: "v1.0.0"
      pullPolicy: IfNotPresent
    pullSecret: ""  # for private registries
  # Bundles list (1+)
  list:
    - slug: all-MiniLM-L6-v2-en
      version: "1.0.0"
      manifestSha256: "<filled at D20 release>"
      mountPath: /etc/efe/bundles/all-MiniLM-L6-v2-en
    - slug: paraphrase-multilingual-MiniLM-L12-v2
      version: "1.0.0"
      manifestSha256: "<filled at D26 release>"
      mountPath: /etc/efe/bundles/paraphrase-multilingual-MiniLM-L12-v2
  # Default active bundle (per tenant or global)
  defaultActive: "all-MiniLM-L6-v2-en"
```

---

## 14. CI workflow stages détaillés

### 14.1 Job graph

```
┌─────────────┐
│  setup      │ ← env + checkout + python + oras + cosign
└──────┬──────┘
       │
       ▼
┌─────────────┐    ┌─────────────┐
│  build (1)  │    │  build (2)  │ ← reproducibility (parallel)
└──────┬──────┘    └──────┬──────┘
       │                  │
       └─────┬────────────┘
             ▼
       ┌──────────────┐
       │ verify-repro │ ← compare SHA256
       └──────┬───────┘
              │
              ▼
       ┌──────────────┐
       │ sign         │ ← Ed25519 + cosign
       └──────┬───────┘
              │
              ▼
       ┌──────────────┐
       │ sbom         │ ← CycloneDX 1.5
       └──────┬───────┘
              │
              ▼
       ┌──────────────┐
       │ license-gate │ ← allowlist check
       └──────┬───────┘
              │
              ▼
       ┌──────────────┐
       │ publish      │ ← oras push
       └──────┬───────┘
              │
              ▼
       ┌──────────────┐
       │ verify-pull  │ ← oras pull + verify sig + hash
       └──────┬───────┘
              │
              ▼
       ┌──────────────┐
       │ notify       │ ← Slack + email
       └──────────────┘
```

### 14.2 Permissions matrix

```yaml
permissions:
  contents: read           # checkout + git log
  packages: write          # oras push to GHCR
  id-token: write          # OIDC for cosign keyless (future)
  attestations: write      # GitHub artifact attestations API
```

### 14.3 Concurrency

```yaml
concurrency:
  group: bundle-release-${{ github.event.inputs.bundle_slug || github.ref }}
  cancel-in-progress: false  # never cancel a running bundle build
```

### 14.4 Runner pinning

```yaml
runs-on: ubuntu-22.04   # NOT ubuntu-latest (reproducibility T-M13-3)
```

---

## 15. Configuration surface (env + Helm values + flags CLI)

### 15.1 Build script flags (`build_sample_bundle.py` / `build_real_bundle.py`)

| Flag | Type | Default | Description |
|---|---|---|---|
| `--out` | path | (required) | Bundle output directory |
| `--slug` | str | derived from `--out` | Override bundle_slug |
| `--hf-revision` | str | None (CI), N/A (dev) | Pinned HF revision SHA |
| `--privkey-path` | path | None | Ed25519 private key file (raw 32B or PEM) |
| `--dev` | flag | False | Dev mode (no signing, placeholder weights) |
| `--quantization` | enum | int8-dynamic (D20), fp16 (D26) | int8-dynamic / int8-static / fp16 / none |
| `--license-override` | str | None | Force a license SPDX ID |
| `--ack-nonredist` | flag | False | Acknowledge non-redistributable license |
| `--source-date-epoch` | int | env or git | Override SOURCE_DATE_EPOCH |
| `--max-size-mib` | int | 30 (D20), 80 (D26) | Refuse if total > limit |
| `--sbom` | flag | False (D20), True (D26) | Generate SBOM.json |

### 15.2 CI workflow inputs (workflow_dispatch)

| Input | Required | Default |
|---|---|---|
| `bundle_slug` | yes | (none) |
| `bundle_version` | yes | (none) |
| `hf_revision` | yes | (none) |
| `mode` | no | private |

### 15.3 Helm values M13

Voir §13.3 pour le fragment `values.m13-bundle.yaml`.

### 15.4 Env vars consumed by efe-server (loader)

(Hérité de D16 §14, repris ici pour référence opérationnelle.)

| Env var | Type | Default | Description |
|---|---|---|---|
| `EFE_EMBEDDER_BACKEND` | enum | `legacy` | `legacy` / `bundle` |
| `EFE_BUNDLE_PATH` | path | `/etc/efe/bundles/active` | Bundle directory mount |
| `EFE_BUNDLE_REQUIRE_SIGNATURE` | bool | `true` (prod) | Refuse unsigned |
| `EFE_BUNDLE_TRUST_STORE` | path | `/etc/efe/bundles/pubkeys.json` | Trust store |
| `EFE_BUNDLE_REQUIRE_SBOM` | bool | `true` (prod) | Refuse missing SBOM |
| `EFE_BUNDLE_ACK_NONREDIST` | bool | `false` | Operator ack for restricted licenses |
| `EFE_BUNDLE_LOAD_TIMEOUT_S` | int | 60 | Boot load timeout |
| `EFE_BUNDLE_ENCODE_TIMEOUT_MS` | int | 500 | Per-encode timeout |

---

## 16. Test plan M13 (35+ pin tests)

### 16.1 Test inventory by sprint

| Sprint | Test file | Tests count |
|---|---|---|
| **D20** | `scripts/tests/test_d20_sample_bundle.py` | 8 |
| **D26** | `scripts/tests/test_d26_real_bundle.py` | 10 |
| **D27** | `scripts/tests/test_d27_bundle_release_workflow.py` | 12 |
| **M13** | `scripts/tests/test_m13_bundles_extension.py` | 8 |
| **Total** | | **38** |

### 16.2 D20 pin tests

```python
def test_d20_bundle_directory_present(): ...
def test_d20_bundle_required_files_present(): ...
def test_d20_manifest_schema_v1_0(): ...
def test_d20_manifest_per_file_sha256_matches_disk(): ...
def test_d20_signature_verifies_against_pinned_pubkey(): ...
def test_d20_bundle_total_size_under_30mib(): ...
def test_d20_license_spdx_apache_2_0(): ...
def test_d20_hf_revision_pinned_not_main(): ...
```

### 16.3 D26 pin tests

```python
def test_d26_bundle_directory_present(): ...
def test_d26_bundle_required_files_present(): ...
def test_d26_manifest_schema_v1_0(): ...
def test_d26_sbom_cyclonedx_1_5(): ...
def test_d26_sbom_includes_machine_learning_model(): ...
def test_d26_sbom_includes_tokenizer_data_component(): ...
def test_d26_signature_verifies(): ...
def test_d26_total_size_under_80mib(): ...
def test_d26_license_apache_2_0(): ...
def test_d26_cross_lingual_f1_above_080(): ...
```

### 16.4 D27 pin tests

```python
def test_d27_workflow_file_present(): ...
def test_d27_workflow_triggers_on_bundle_tag(): ...
def test_d27_workflow_pins_runner_to_ubuntu_22_04(): ...
def test_d27_workflow_has_reproducibility_check_step(): ...
def test_d27_workflow_uses_oras_push(): ...
def test_d27_workflow_post_publish_verify_step(): ...
def test_d27_workflow_uses_pinned_action_versions(): ...
def test_d27_workflow_no_unsigned_publish(): ...
def test_d27_workflow_concurrency_does_not_cancel_in_progress(): ...
def test_d27_workflow_permissions_least_privilege(): ...
def test_d27_workflow_loads_privkey_from_secret(): ...
def test_d27_workflow_masks_privkey_in_logs(): ...
```

### 16.5 M13-extension pin tests

```python
def test_m13_bundles_directory_inventory(): ...
def test_m13_bundles_index_readme_present(): ...
def test_m13_all_bundles_have_manifest_and_sig(): ...
def test_m13_all_bundles_pass_d16_schema(): ...
def test_m13_no_bundle_uses_revision_main(): ...
def test_m13_no_bundle_in_forbidden_license(): ...
def test_m13_workflow_oras_version_pinned(): ...
def test_m13_workflow_cosign_version_pinned(): ...
```

---

## 17. Operational runbook

### 17.1 Add a new bundle

1. **Choose model + version** : pick HF model + freeze a revision SHA.
2. **Local dry-run** :
   ```bash
   python3 scripts/release/build_real_bundle.py \
       --out /tmp/bundle-test \
       --slug new-model-en-1.0.0 \
       --hf-revision <40-char SHA> \
       --dev
   ```
3. **Validate manifest** :
   ```bash
   python3 -c "
   import json, jsonschema
   m = json.loads(open('/tmp/bundle-test/manifest.json').read())
   s = json.loads(open('crates/efe-server/schemas/d16_manifest.schema.json').read())
   jsonschema.validate(instance=m, schema=s)
   "
   ```
4. **Local smoke test** :
   ```bash
   helm install efe-test charts/efe \
       --set config.embedder.bundle.path=/tmp/bundle-test
   helm test efe-test
   ```
5. **Tag + push** :
   ```bash
   git tag bundle-vX.Y.Z
   git push origin bundle-vX.Y.Z
   ```
6. **Wait for CI** : ~10-15 min for D20 / ~25-30 min for D26.
7. **Verify OCI publish** :
   ```bash
   oras manifest fetch oci://registry.efe.ai/bundles/<slug>:<version>
   ```

### 17.2 Rotate signing key

1. Generate new keypair :
   ```bash
   cosign generate-key-pair --kms aws-kms://...
   # Or local for testing :
   cosign generate-key-pair
   ```
2. Add pubkey to `pubkeys.json` trust store with `valid_from=T+30j` :
   ```bash
   git checkout -b feat/rotate-bundle-key-2027q1
   # Edit deploy/helm/efe/files/pubkeys.json
   git commit -m "feat(bundle): add 2027q1 signing pubkey to trust store"
   ```
3. Wait 30 days (grace period for trust store deployment).
4. Update GH secret :
   ```bash
   gh secret set EFE_BUNDLE_PRIVKEY_PEM < new-privkey.pem
   ```
5. Update CI to use new key ID :
   ```bash
   # .github/workflows/bundle-release.yml
   env:
     SIGNING_KEY_ID: efe-prod-bundle-2027q1   # was 2026q1
   ```
6. Republish all active bundles with new key (M13 §17.3 procedure).
7. After 90 days grace, retire old pubkey from trust store.

### 17.3 Republish a bundle (e.g., after key rotation)

1. Create a new tag with the same version (impossible if immutable!) → use `1.0.1` patch.
2. Re-trigger CI with `workflow_dispatch` :
   ```bash
   gh workflow run bundle-release.yml \
       -f bundle_slug=all-MiniLM-L6-v2-en \
       -f bundle_version=1.0.1 \
       -f hf_revision=e4ce9877abf3edfe10b0d82785e83bdcb973e22e
   ```
3. Update `manifest.sha256` references in chart values.

### 17.4 Recover from corrupt OCI artifact

1. Detect : alert `D27ObsBundlePublishFailureRate` fires.
2. Inspect manifest digest at registry :
   ```bash
   oras manifest fetch --output - oci://...:<v> | jq
   ```
3. Compare with build-time digest in CI logs (artifact `manifest.json.sha`).
4. If mismatch :
   - Mark tag as compromised (`oras tag delete`).
   - Republish under `<v+1>` patch version.
   - Notify SOC + post-incident review (`runbooks/M13-D27-corrupt-oci.md`).

---

## 18. SLOs M13-BND-*

| SLO ID | Metric | Target | Window | Source |
|---|---|---|---|---|
| **M13-BND-1** | Bundle release pipeline duration | P99 < 1800s (30 min) | rolling 7d | GH Actions duration |
| **M13-BND-2** | OCI publish success rate | ≥ 99.5% | rolling 30d | `oras push` exit code |
| **M13-BND-3** | Reproducibility check pass rate | ≥ 99.9% | rolling 7d | Compare SHA in CI |
| **M13-BND-4** | Signature verify time (boot) | P99 < 100ms | rolling 1h | `efe_bundle_verify_duration_seconds` |
| **M13-BND-5** | Bundle pull from OCI to pod | P95 < 30s | rolling 1h | `efe_bundle_pull_duration_seconds` |
| **M13-BND-6** | License gate refusal rate | < 1% | rolling 30d | CI exit code |
| **M13-BND-7** | SBOM validation pass rate | 100% | every release | cyclonedx-cli validate |

---

## 19. Migration path D16-stub → M13

### 19.1 Existing chart M12 → M13 D20 release path

Phase 1 (J55, sprint D20 ship) :
- New build of `bundles/all-MiniLM-L6-v2-en-1.0.0/` via CI tag `bundle-v1.0.0`.
- Push to `ghcr.io/efe-project/bundles-private/all-MiniLM-L6-v2-en:1.0.0`.
- Update chart `values.prod.yaml` :
  ```yaml
  bundle:
    slug: all-MiniLM-L6-v2-en
    version: "1.0.0"
    manifestSha256: "<filled>"
  ```
- Roll out via `helm upgrade` → M12 D27 hook auto-reloads.

Phase 2 (J120, sprint D26 ship) :
- Multilingual bundle published.
- Operators can switch via `--set bundle.slug=paraphrase-multilingual-...`.

Phase 3 (D40 GA, post-M13) :
- Promote registry `private → public`.
- Update default values to point to public registry.

### 19.2 Backward compat with D16 stub

The D16 stub (`is_dev_unsigned=true` placeholder) MUST continue to
work for dev clusters until the first real M13 D20 release ships.
Pin test `test_d16_stub_still_loads_in_dev_mode` enforces this.

After D20 ships, the dev stub is **deprecated** but kept for one
release cycle, then removed in M14.

---

## 20. Risk register & open questions

### 20.1 Risk register

| Risk ID | Description | Probability | Impact | Mitigation |
|---|---|:---:|:---:|---|
| **R-M13-1** | HF model upstream pulled / withdrawn | LOW | HIGH | Mirror to internal artifact storage at first publish |
| **R-M13-2** | License changes upstream (Apache → restricted) | LOW | HIGH | License gate in CI, fail-closed |
| **R-M13-3** | OCI registry outage at release time | MED | MED | Retry logic + fallback registry (GHCR backup) |
| **R-M13-4** | Reproducibility break due to onnxruntime version drift | MED | HIGH | Pin onnxruntime in `requirements-build.txt` |
| **R-M13-5** | Signing key compromise | LOW | CRITICAL | KMS HSM-backed + Rekor transparency log |
| **R-M13-6** | Bundle size grows beyond Helm Secret 1MB limit | MED | MED | OCI pull init container (default M13) avoids Secret limit |
| **R-M13-7** | Cross-lingual quality regression on D26 | LOW | MED | F1 ≥ 0.80 gate + monthly eval against fixed test set |
| **R-M13-8** | SBOM tooling drift | LOW | LOW | Pin `cyclonedx-bom>=4.0,<5.0` |

### 20.2 Open questions

| ID | Question | Owner | Resolution target |
|---|---|---|---|
| **Q-M13-1** | Should D26 ship FP16 or BF16 quantization? | ML Lead | Before D26 sprint kickoff |
| **Q-M13-2** | Public registry FQDN : `registry.efe.ai` or `bundles.efe.ai`? | DevOps Lead | Before D40 GA |
| **Q-M13-3** | Cosign keyless via GH OIDC (vs static key)? | Security Lead | Decide post-D27 |
| **Q-M13-4** | KMS provider : AWS KMS, GCP KMS, HashiCorp Vault, or self-hosted? | Security Lead | Decide post-D27 |
| **Q-M13-5** | SBOM signed separately or covered by manifest sig? | Security Lead | Resolved : covered by manifest (since `SBOM.json` is in `components[]`) |
| **Q-M13-6** | M14 multi-bundle per-tenant routing — same registry or per-tenant scoped? | Product Lead | M14 kickoff |

---

## 21. Appendix A — alternatives considered

### 21.1 Storage : OCI vs S3 vs GitHub Releases

(Voir §7.1.) **OCI Artifacts via ORAS** chosen.

### 21.2 Signing : raw Ed25519 vs cosign vs Sigstore Fulcio

| Option | Pros | Cons | Decision |
|---|---|---|---|
| Raw Ed25519 | Simple, deterministic, Rust crate `ed25519-dalek` | Pas de transparency log natif | M13 base layer |
| cosign with static key | Adds Rekor entry | Requires Sigstore client | M13 enrichment layer |
| Sigstore Fulcio (keyless) | No private keys to manage, OIDC-bound | Online dependency, cert chain complexity | Post-M14 evaluation |

**Décision M13** : raw Ed25519 (sur le manifest) + cosign sign on
OCI artifact. **Pas de Fulcio en M13** (online dependency, defer).

### 21.3 Quantization : INT8 dynamic vs INT8 static vs FP16

| Option | Compression | Accuracy | D20 | D26 |
|---|---|---|---|---|
| INT8 dynamic | 4× | ~99.3% | ✅ | ❌ (vocab too large) |
| INT8 static (calibrated) | 4× | ~99.7% | (deferred D40+) | (deferred) |
| FP16 | 2× | ~99.9% | ❌ (size overshoots) | ✅ |
| BF16 | 2× | ~99.9% (training-friendly) | N/A | considered Q-M13-1 |

### 21.4 Container distribution : separate registry vs same as efe-server

| Option | Pros | Cons | Decision |
|---|---|---|---|
| Same registry as efe-server image | Simpler ops | Mixed concerns (image vs ML) | ❌ |
| Separate `bundles/` namespace | Clear separation, distinct retention/access policies | Two registries to manage | ✅ |

---

## 22. Appendix B — glossaire

| Term | Definition |
|---|---|
| **Bundle** | A directory containing a model's weights + tokenizer + manifest + signature, packaged for distribution. |
| **manifest.json** | Authoritative metadata file per D16 v1.0 schema, lists all components + their SHA256 hashes. |
| **manifest.json.sig** | Detached Ed25519 signature over the canonical manifest.json bytes. |
| **bundle_id** | Canonical identifier `{family}/{name}@{version}/{framework}/{dim}`. |
| **bundle_slug** | Filesystem-safe identifier `{name}-{lang}-{version}` used as directory name + OCI tag. |
| **bundle_fingerprint** | SHA256 of the canonical manifest.json bytes. Used as catalog binding. |
| **OCI Artifact** | Generic blob storage in an OCI-compliant registry with custom artifact type. |
| **ORAS** | OCI Registry As Storage — CLI/library for pushing arbitrary content as OCI artifacts. |
| **Rekor** | Sigstore transparency log for signing events (immutable, queryable). |
| **SBOM** | Software Bill of Materials — inventory of components in a software/ML artifact. |
| **CycloneDX** | OWASP-curated SBOM format, version 1.5 supports ML-BOM extension. |
| **ML-BOM** | Machine Learning Bill of Materials — CycloneDX 1.5 components with `type=machine-learning-model`. |
| **SOURCE_DATE_EPOCH** | Reproducible Builds env var that fixes "now" to a deterministic value. |
| **SPDX** | Software Package Data Exchange — standardized license identifiers (Apache-2.0, MIT, etc.). |
| **Trust store** | `pubkeys.json` mounted into the pod listing accepted Ed25519 signing keys. |

---

## 23. Appendix C — `bundle-release.yml` full example

```yaml
# .github/workflows/bundle-release.yml
#
# Sprint M13 D27 (Φ-C) — Bundle release auto-bundling pipeline.
#
# Cf. docs/EFE_BUNDLES_EXTENSION_ARCHITECTURE.md §6 + §14.

name: bundle-release

on:
  push:
    tags:
      - "bundle-v*.*.*"
  workflow_dispatch:
    inputs:
      bundle_slug:
        description: "Bundle slug"
        required: true
        type: string
      bundle_version:
        description: "Semver version"
        required: true
        type: string
      hf_revision:
        description: "Pinned HF revision SHA (40 chars)"
        required: true
        type: string
      mode:
        description: "private | public"
        required: false
        default: "private"
        type: choice
        options: [private, public]

env:
  REGISTRY_PRIVATE: "ghcr.io/efe-project/bundles-private"
  REGISTRY_PUBLIC: "registry.efe.ai/bundles"
  ORAS_VERSION: "1.0.0"
  COSIGN_VERSION: "v2.2.0"
  PYTHON_VERSION: "3.12"
  ONNXRUNTIME_VERSION: "1.16.3"
  CYCLONEDX_VERSION: "4.0.0"

concurrency:
  group: bundle-release-${{ github.event.inputs.bundle_slug || github.ref }}
  cancel-in-progress: false

permissions:
  contents: read
  packages: write
  id-token: write
  attestations: write

jobs:
  setup:
    name: Setup + parse inputs
    runs-on: ubuntu-22.04
    timeout-minutes: 5
    outputs:
      bundle_slug: ${{ steps.parse.outputs.bundle_slug }}
      bundle_version: ${{ steps.parse.outputs.bundle_version }}
      hf_revision: ${{ steps.parse.outputs.hf_revision }}
      registry: ${{ steps.parse.outputs.registry }}
      git_sha: ${{ steps.parse.outputs.git_sha }}
      source_date_epoch: ${{ steps.parse.outputs.source_date_epoch }}
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0

      - name: Parse inputs
        id: parse
        run: |
          # Tag-driven path: parse from tag bundle-vX.Y.Z
          if [[ "${GITHUB_REF}" =~ refs/tags/bundle-v([0-9]+)\.([0-9]+)\.([0-9]+)$ ]]; then
            BUNDLE_VERSION="${BASH_REMATCH[1]}.${BASH_REMATCH[2]}.${BASH_REMATCH[3]}"
            # Slug + revision come from a YAML registry file or workflow input
            BUNDLE_SLUG="${{ inputs.bundle_slug || 'all-MiniLM-L6-v2-en' }}"
            HF_REVISION="${{ inputs.hf_revision }}"
          else
            BUNDLE_SLUG="${{ inputs.bundle_slug }}"
            BUNDLE_VERSION="${{ inputs.bundle_version }}"
            HF_REVISION="${{ inputs.hf_revision }}"
          fi
          MODE="${{ inputs.mode || 'private' }}"
          if [[ "${MODE}" == "public" ]]; then
            REGISTRY="${REGISTRY_PUBLIC}"
          else
            REGISTRY="${REGISTRY_PRIVATE}"
          fi
          GIT_SHA=$(git rev-parse HEAD)
          SOURCE_DATE_EPOCH=$(git log -1 --format=%ct HEAD)
          # Validate hf_revision is 40 hex chars
          if ! [[ "${HF_REVISION}" =~ ^[0-9a-f]{40}$ ]]; then
            echo "::error::hf_revision must be a 40-char hex SHA, got: ${HF_REVISION}"
            exit 1
          fi
          echo "bundle_slug=${BUNDLE_SLUG}" >> "$GITHUB_OUTPUT"
          echo "bundle_version=${BUNDLE_VERSION}" >> "$GITHUB_OUTPUT"
          echo "hf_revision=${HF_REVISION}" >> "$GITHUB_OUTPUT"
          echo "registry=${REGISTRY}" >> "$GITHUB_OUTPUT"
          echo "git_sha=${GIT_SHA}" >> "$GITHUB_OUTPUT"
          echo "source_date_epoch=${SOURCE_DATE_EPOCH}" >> "$GITHUB_OUTPUT"

  build-1:
    name: Build (run 1)
    needs: setup
    runs-on: ubuntu-22.04
    timeout-minutes: 30
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-python@v5
        with: { python-version: "${{ env.PYTHON_VERSION }}" }
      - name: Install build deps
        run: |
          pip install \
              "onnxruntime==${{ env.ONNXRUNTIME_VERSION }}" \
              "huggingface_hub>=0.20" \
              "transformers>=4.35" \
              "sentence-transformers>=2.2" \
              "onnx>=1.15" \
              "cryptography>=42" \
              "pyyaml" \
              "jsonschema"
      - name: Build bundle (run 1)
        env:
          SOURCE_DATE_EPOCH: ${{ needs.setup.outputs.source_date_epoch }}
        run: |
          mkdir -p _build1/
          python3 scripts/release/build_real_bundle.py \
              --out "_build1/${{ needs.setup.outputs.bundle_slug }}-${{ needs.setup.outputs.bundle_version }}" \
              --slug "${{ needs.setup.outputs.bundle_slug }}-${{ needs.setup.outputs.bundle_version }}" \
              --hf-revision "${{ needs.setup.outputs.hf_revision }}"
          # Compute manifest SHA256
          sha256sum "_build1/${{ needs.setup.outputs.bundle_slug }}-${{ needs.setup.outputs.bundle_version }}/manifest.json" \
              > _build1.manifest.sha
      - uses: actions/upload-artifact@v4
        with:
          name: build-1
          path: _build1/
          retention-days: 7

  build-2:
    name: Build (run 2 - reproducibility)
    needs: setup
    runs-on: ubuntu-22.04
    timeout-minutes: 30
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-python@v5
        with: { python-version: "${{ env.PYTHON_VERSION }}" }
      - name: Install build deps
        run: |
          pip install \
              "onnxruntime==${{ env.ONNXRUNTIME_VERSION }}" \
              "huggingface_hub>=0.20" \
              "transformers>=4.35" \
              "sentence-transformers>=2.2" \
              "onnx>=1.15" \
              "cryptography>=42" \
              "pyyaml" \
              "jsonschema"
      - name: Build bundle (run 2)
        env:
          SOURCE_DATE_EPOCH: ${{ needs.setup.outputs.source_date_epoch }}
        run: |
          mkdir -p _build2/
          python3 scripts/release/build_real_bundle.py \
              --out "_build2/${{ needs.setup.outputs.bundle_slug }}-${{ needs.setup.outputs.bundle_version }}" \
              --slug "${{ needs.setup.outputs.bundle_slug }}-${{ needs.setup.outputs.bundle_version }}" \
              --hf-revision "${{ needs.setup.outputs.hf_revision }}"
          sha256sum "_build2/${{ needs.setup.outputs.bundle_slug }}-${{ needs.setup.outputs.bundle_version }}/manifest.json" \
              > _build2.manifest.sha
      - uses: actions/upload-artifact@v4
        with:
          name: build-2
          path: _build2/
          retention-days: 7

  verify-repro:
    name: Verify reproducibility
    needs: [setup, build-1, build-2]
    runs-on: ubuntu-22.04
    timeout-minutes: 5
    steps:
      - uses: actions/download-artifact@v4
        with: { name: build-1, path: _build1/ }
      - uses: actions/download-artifact@v4
        with: { name: build-2, path: _build2/ }
      - name: Compare manifests
        run: |
          SHA1=$(sha256sum "_build1/${{ needs.setup.outputs.bundle_slug }}-${{ needs.setup.outputs.bundle_version }}/manifest.json" | awk '{print $1}')
          SHA2=$(sha256sum "_build2/${{ needs.setup.outputs.bundle_slug }}-${{ needs.setup.outputs.bundle_version }}/manifest.json" | awk '{print $1}')
          if [ "$SHA1" != "$SHA2" ]; then
            echo "::error::REPRODUCIBILITY_FAIL :: build1 sha=${SHA1}, build2 sha=${SHA2}"
            diff -ru "_build1/" "_build2/" | head -200
            exit 3
          fi
          echo "::notice::REPRODUCIBILITY_OK :: ${SHA1}"
          echo "manifest_sha256=${SHA1}" >> "$GITHUB_OUTPUT"

  sign:
    name: Sign Ed25519 + cosign
    needs: [setup, verify-repro]
    runs-on: ubuntu-22.04
    timeout-minutes: 5
    environment: production  # gates GH secret access
    steps:
      - uses: actions/download-artifact@v4
        with: { name: build-1, path: _build/ }
      - name: Install cosign
        run: |
          curl -fsSLo cosign \
              "https://github.com/sigstore/cosign/releases/download/${{ env.COSIGN_VERSION }}/cosign-linux-amd64"
          chmod +x cosign
          sudo mv cosign /usr/local/bin/
      - name: Load privkey from GH Secret + mask
        env:
          PRIVKEY_PEM: ${{ secrets.EFE_BUNDLE_PRIVKEY_PEM }}
        run: |
          # CRITICAL: mask the privkey value in logs as soon as we have it.
          echo "::add-mask::${PRIVKEY_PEM}"
          # Persist to a tmp file with restrictive perms
          umask 077
          echo "${PRIVKEY_PEM}" > /tmp/efe-bundle.privkey.pem
          set +x   # never echo commands using the key
      - name: Ed25519 sign manifest
        run: |
          BUNDLE_DIR="_build/${{ needs.setup.outputs.bundle_slug }}-${{ needs.setup.outputs.bundle_version }}"
          python3 - <<'PY'
          import sys
          from pathlib import Path
          from cryptography.hazmat.primitives import serialization
          from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
          privkey_data = Path("/tmp/efe-bundle.privkey.pem").read_bytes()
          privkey = serialization.load_pem_private_key(privkey_data, password=None)
          assert isinstance(privkey, Ed25519PrivateKey), "Privkey is not Ed25519"
          import os
          bundle_dir = Path(os.environ["BUNDLE_DIR"])
          manifest_bytes = (bundle_dir / "manifest.json").read_bytes()
          sig = privkey.sign(manifest_bytes)
          (bundle_dir / "manifest.json.sig").write_bytes(sig)
          print(f"Signed manifest.json ({len(manifest_bytes)} bytes) → {len(sig)} byte sig")
          PY
        env:
          BUNDLE_DIR: "_build/${{ needs.setup.outputs.bundle_slug }}-${{ needs.setup.outputs.bundle_version }}"
      - name: Verify locally before push
        run: |
          BUNDLE_DIR="_build/${{ needs.setup.outputs.bundle_slug }}-${{ needs.setup.outputs.bundle_version }}"
          python3 - <<'PY'
          import os
          from pathlib import Path
          from cryptography.hazmat.primitives import serialization
          from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
          privkey_data = Path("/tmp/efe-bundle.privkey.pem").read_bytes()
          privkey = serialization.load_pem_private_key(privkey_data, password=None)
          pubkey = privkey.public_key()
          bundle_dir = Path(os.environ["BUNDLE_DIR"])
          pubkey.verify(
              (bundle_dir / "manifest.json.sig").read_bytes(),
              (bundle_dir / "manifest.json").read_bytes(),
          )
          print("VERIFY_OK")
          PY
        env:
          BUNDLE_DIR: "_build/${{ needs.setup.outputs.bundle_slug }}-${{ needs.setup.outputs.bundle_version }}"
      - name: Cleanup privkey
        if: always()
        run: rm -f /tmp/efe-bundle.privkey.pem
      - uses: actions/upload-artifact@v4
        with:
          name: build-signed
          path: _build/

  sbom:
    name: Generate SBOM CycloneDX 1.5
    needs: [setup, sign]
    runs-on: ubuntu-22.04
    timeout-minutes: 10
    steps:
      - uses: actions/checkout@v4
      - uses: actions/download-artifact@v4
        with: { name: build-signed, path: _build/ }
      - uses: actions/setup-python@v5
        with: { python-version: "${{ env.PYTHON_VERSION }}" }
      - run: pip install "cyclonedx-bom==${{ env.CYCLONEDX_VERSION }}" "cyclonedx-python-lib"
      - name: Generate SBOM
        run: |
          BUNDLE_DIR="_build/${{ needs.setup.outputs.bundle_slug }}-${{ needs.setup.outputs.bundle_version }}"
          python3 scripts/release/generate_bundle_sbom.py \
              --bundle-dir "${BUNDLE_DIR}" \
              --hf-revision "${{ needs.setup.outputs.hf_revision }}" \
              --output "${BUNDLE_DIR}/SBOM.json"
      - name: Validate SBOM
        run: |
          BUNDLE_DIR="_build/${{ needs.setup.outputs.bundle_slug }}-${{ needs.setup.outputs.bundle_version }}"
          # Install cyclonedx-cli
          curl -fsSL -o cyclonedx-cli "https://github.com/CycloneDX/cyclonedx-cli/releases/download/v0.25.0/cyclonedx-linux-x64"
          chmod +x cyclonedx-cli
          ./cyclonedx-cli validate \
              --input-file "${BUNDLE_DIR}/SBOM.json" \
              --schema-version 1.5 \
              --fail-on-errors
      - uses: actions/upload-artifact@v4
        with:
          name: build-sbom
          path: _build/

  license-gate:
    name: License allowlist gate
    needs: [setup, sbom]
    runs-on: ubuntu-22.04
    timeout-minutes: 2
    steps:
      - uses: actions/checkout@v4
      - uses: actions/download-artifact@v4
        with: { name: build-sbom, path: _build/ }
      - run: |
          BUNDLE_DIR="_build/${{ needs.setup.outputs.bundle_slug }}-${{ needs.setup.outputs.bundle_version }}"
          python3 scripts/release/license_gate.py \
              --manifest "${BUNDLE_DIR}/manifest.json"

  publish:
    name: Publish to OCI registry
    needs: [setup, license-gate]
    runs-on: ubuntu-22.04
    timeout-minutes: 15
    steps:
      - uses: actions/download-artifact@v4
        with: { name: build-sbom, path: _build/ }
      - name: Install ORAS
        run: |
          curl -fsSL -o oras.tar.gz \
              "https://github.com/oras-project/oras/releases/download/v${{ env.ORAS_VERSION }}/oras_${{ env.ORAS_VERSION }}_linux_amd64.tar.gz"
          tar xzf oras.tar.gz
          chmod +x oras
          sudo mv oras /usr/local/bin/
      - name: Login to GHCR
        if: contains(needs.setup.outputs.registry, 'ghcr.io')
        run: |
          echo "${{ secrets.GITHUB_TOKEN }}" | \
              oras login ghcr.io \
                  --username "${{ github.actor }}" \
                  --password-stdin
      - name: Push bundle as OCI artifact
        run: |
          BUNDLE_DIR="_build/${{ needs.setup.outputs.bundle_slug }}-${{ needs.setup.outputs.bundle_version }}"
          MANIFEST_SHA=$(sha256sum "${BUNDLE_DIR}/manifest.json" | awk '{print $1}')
          cd "${BUNDLE_DIR}"
          oras push \
              "${{ needs.setup.outputs.registry }}/${{ needs.setup.outputs.bundle_slug }}:${{ needs.setup.outputs.bundle_version }}" \
              --artifact-type "application/vnd.efe.bundle.v1+json" \
              --annotation "efe.bundle.id=${{ needs.setup.outputs.bundle_slug }}" \
              --annotation "efe.bundle.manifest_sha256=${MANIFEST_SHA}" \
              --annotation "efe.bundle.signature_algo=ed25519" \
              --annotation "efe.bundle.signing_key_id=efe-prod-bundle-2026q1" \
              --annotation "org.opencontainers.image.source=https://github.com/${{ github.repository }}" \
              --annotation "org.opencontainers.image.revision=${{ needs.setup.outputs.git_sha }}" \
              "manifest.json:application/vnd.efe.bundle.manifest.v1+json" \
              "manifest.json.sig:application/octet-stream" \
              "weights/model.onnx:application/vnd.onnx.model.v1" \
              "tokenizer/tokenizer.json:application/vnd.huggingface.tokenizer.v1+json" \
              "tokenizer/tokenizer_config.json:application/json" \
              "tokenizer/special_tokens_map.json:application/json" \
              "runtime/runtime_config.json:application/json" \
              "SBOM.json:application/vnd.cyclonedx+json" \
              "LICENSE:text/plain" \
              "README.md:text/markdown"

  verify-pull:
    name: Pull + verify integrity
    needs: [setup, publish]
    runs-on: ubuntu-22.04
    timeout-minutes: 10
    steps:
      - uses: actions/checkout@v4
      - name: Install ORAS
        run: |
          curl -fsSL -o oras.tar.gz \
              "https://github.com/oras-project/oras/releases/download/v${{ env.ORAS_VERSION }}/oras_${{ env.ORAS_VERSION }}_linux_amd64.tar.gz"
          tar xzf oras.tar.gz
          chmod +x oras
          sudo mv oras /usr/local/bin/
      - name: Pull artifact
        run: |
          mkdir _verify/
          oras pull "${{ needs.setup.outputs.registry }}/${{ needs.setup.outputs.bundle_slug }}:${{ needs.setup.outputs.bundle_version }}" \
              -o _verify/
          ls -la _verify/
      - name: Verify integrity
        run: |
          python3 scripts/release/verify_pulled_bundle.py \
              --bundle-dir _verify/ \
              --pubkey-id efe-prod-bundle-2026q1

  notify:
    name: Notify on completion
    needs: [setup, verify-pull]
    runs-on: ubuntu-22.04
    if: always()
    steps:
      - name: Slack notification
        run: |
          STATUS="${{ needs.verify-pull.result }}"
          curl -fsSL -X POST \
              -H 'Content-Type: application/json' \
              -d "{
                \"text\": \"Bundle release ${{ needs.setup.outputs.bundle_slug }}:${{ needs.setup.outputs.bundle_version }} → ${STATUS}\"
              }" \
              "${{ secrets.SLACK_WEBHOOK }}"
```

---

## 24. Appendix D — `build_real_bundle.py` blueprint

```python
#!/usr/bin/env python3
"""
Sprint M13 D26 — Production multilingual bundle build pipeline.

Cf. docs/EFE_BUNDLES_EXTENSION_ARCHITECTURE.md §5.

Extends `scripts/release/build_sample_bundle.py` (D20) with:
  - Real HF model pull (PINNED revision, T-M13-7 mitigation)
  - FP16 quantization (vs INT8 for D26 multilingual)
  - SBOM CycloneDX 1.5 generation
  - Sentencepiece tokenizer handling
  - Cross-lingual evaluation gate
"""

from __future__ import annotations
import argparse, datetime, hashlib, json, os, shutil, sys
from pathlib import Path

# ... constants
BUNDLE_SCHEMA_VERSION = "1.0"
DEFAULT_HF_REVISION_D26 = "bf3bf13ab40c3157080a7ab344c831b9ad18b5eb"
TARGET_SIZE_MIB_D26 = 80

def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--slug", type=str)
    parser.add_argument("--hf-revision", type=str, required=True)
    parser.add_argument("--privkey-path", type=Path)
    parser.add_argument("--quantization",
                        choices=("fp16", "fp32", "int8-dynamic"),
                        default="fp16")
    parser.add_argument("--license-override", type=str)
    parser.add_argument("--ack-nonredist", action="store_true")
    parser.add_argument("--max-size-mib", type=int, default=TARGET_SIZE_MIB_D26)
    parser.add_argument("--dev", action="store_true")
    args = parser.parse_args(argv)

    # Phase 1 : pull
    phase_pull_multilingual(args.out, hf_revision=args.hf_revision, dev=args.dev)
    # Phase 2 : quantize
    phase_quantize_fp16(args.out)
    # Phase 3 : write manifest
    manifest, manifest_path = phase_write_manifest_d26(
        args.out,
        hf_revision=args.hf_revision,
        license_override=args.license_override,
    )
    # Phase 4 : license gate
    phase_license_gate(manifest, ack_nonredist=args.ack_nonredist)
    # Phase 5 : sign
    phase_sign(manifest_path, privkey_path=args.privkey_path, dev=args.dev)
    # Phase 6 : SBOM
    phase_generate_sbom(args.out, manifest=manifest)
    # Phase 7 : final size check
    phase_check_size(args.out, max_mib=args.max_size_mib)
    # Phase 8 : summary
    print(f"BUNDLE_OK path={args.out} sha256_manifest={sha256_file(manifest_path)}")
    return 0

if __name__ == "__main__":
    sys.exit(main())
```

---

## 25. Appendix E — pin tests inventory complète

(Détail complet en §16. Total : **38 pin tests Python**, organisés en
4 fichiers de test, couvrant D20 + D26 + D27 + cross-cutting M13.)

---

**Fin du document M13 architecture extension.**

> Volumétrie totale : ~2 250 lignes. Suit le pattern M12 (2 269 lignes)
> et D16 (2 213 lignes). Couverture : 25 sections + 5 appendices.
>
> Prochaine étape (post-architecture) : code drop ultra-détaillé en
> 3 sprints distincts (D20 → D26 → D27) suivant le pattern M12 livraison
> + audit pass 1.
