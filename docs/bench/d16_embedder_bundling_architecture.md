# Sprint D16 — Architecture détaillée : Embedder bundling

> **Statut** : architecture détaillée, prête à implémenter.
> **Phase** : ultra-thinking. Précision chirurgicale suprême.
> **Position** : suit D15 (Onboarding doc + OIDC IdP integration) + son
> audit interne (`docs/audits/d15_audit_report.md` — 17 défauts,
> 17 fixés). Ouvre la voie aux **déploiements air-gap**, aux
> **upgrades reproductibles** du modèle d'embeddings, et à la
> **vérifiabilité supply-chain** des poids ML utilisés en production.
> **Pré-requis** : avoir lu
> [`d8_hybrid_realdata_architecture.md`](d8_hybrid_realdata_architecture.md)
> (intégration des embeddings réels D8 et leur impact sur les SLOs),
> [`d12_catalog_admin_versioning_architecture.md`](d12_catalog_admin_versioning_architecture.md)
> (le `embedder_fingerprint` est déjà attaché à chaque catalogue),
> [`d14_postgres_catalog_backend_architecture.md`](d14_postgres_catalog_backend_architecture.md)
> (les fingerprints sont matérialisés en base SQL),
> [`d14bis_hmac_audit_chain_architecture.md`](d14bis_hmac_audit_chain_architecture.md)
> (signature HMAC chain — modèle pour la signature détachée
> Cosign-style des bundles), [`d15_onboarding_idp_architecture.md`](d15_onboarding_idp_architecture.md)
> (le pattern Secret K8s mounted-read-only + sub-path qu'on réutilise),
> [`M12_ARCHITECTURE.md`](../../M12_ARCHITECTURE.md) § 5 (mode1 backend
> qui hébergera l'embedder loader).
>
> **Mission** : transformer le chemin embedder d'un *fetch
> opportuniste depuis HuggingFace Hub* en un *artefact bundle
> signé, distribuable par OCI registry, vérifiable par signature
> détachée, et lié bidirectionnellement à un catalogue D12 par
> fingerprint cryptographique* — le tout sans rompre la
> compatibilité D8 (les déploiements existants qui utilisent
> `LocalSTEmbedder` doivent continuer à fonctionner sans changement).
>
> **Pain points opérationnels résolus** :
>
> 1. **Cold-start latency par scale-up** : avec D9 HPA 3→30
>    réplicas, chaque scale-up event re-fetche `all-MiniLM-L6-v2`
>    (~80 MiB) depuis HuggingFace Hub. À 30 réplicas en burst,
>    c'est 2.4 GiB de bandwidth + ~30-60 s de boot avant le
>    premier `Encode` servi. Inacceptable pour les SLOs M11.
> 2. **Dépendance externe non-vérifiable** : `huggingface_hub`
>    résout un `model_name` en un commit SHA *au moment du
>    fetch*. Si HF re-tag, deux pods déployés à 5 minutes
>    d'intervalle peuvent loader des poids différents. Pas de
>    pinning byte-exact. Pas de reproductibilité.
> 3. **Air-gap impossible** : les déploiements régulés
>    (banques, défense, santé) bloquent l'egress vers
>    `huggingface.co`. Sans bundling, EFE ne peut pas être
>    déployé sur ces clusters.
> 4. **Supply-chain attack non-mitigé** : HF Hub ne signe pas
>    les modèles. Un takeover du compte de l'auteur du modèle
>    permet de pousser des poids backdoorés ; tous les pods
>    EFE qui re-fetchent absorbent l'attaque. Pas de cosign,
>    pas de SBOM, pas de provenance attestation.
> 5. **Catalogue ↔ embedder drift silencieux** : D12 introduit
>    `embedder_fingerprint` (stocké au registre catalogue),
>    mais aujourd'hui le fingerprint est juste
>    `family/model@dim` sans hash des poids. Deux runs
>    `LocalSTEmbedder("all-MiniLM-L6-v2")` un mois d'écart
>    peuvent produire des cones D6 fittés sur des géométries
>    légèrement différentes — sans aucun signal opérationnel.
> 6. **GPU-acceleration impossible sans rework** :
>    `sentence-transformers` est PyTorch-only. Pour servir
>    >10 k QPS sur GPU H100 il faut ONNX Runtime CUDA EP ou
>    TensorRT. Le bundle format doit donc supporter ONNX et
>    PyTorch en parallèle, exposer le runtime via un loader
>    abstrait.
> 7. **Multi-tenant multi-langue** : un client FR + un client
>    EN dans le même cluster veulent peut-être deux embedders
>    distincts (`all-MiniLM-L6-v2` vs
>    `paraphrase-multilingual-mpnet-base-v2`). Aujourd'hui,
>    un seul embedder par process. D16 doit ouvrir la porte
>    au routing par tenant.
> 8. **Image-bundling explose la taille de container** : si on
>    bake les poids dans l'image Docker (~80 MiB pour
>    MiniLM, ~500 MiB pour BGE-large, ~2 GiB pour XLM-R), on
>    multiplie par N le coût de l'image registry, des pulls
>    K8s, des cold starts. Architecture inacceptable au-delà
>    de MiniLM.
>
> **Note importante sur le périmètre** : ce sprint ne change PAS
> l'algo d'embeddings. Le `LocalSTEmbedder` actuel reste l'API
> par défaut ; D16 introduit une *couche de packaging* (bundle
> format + loader + distribution) sans remplacer le modèle.
> L'évolution vers ONNX-first est un sous-objectif optionnel
> traité en § 6.4. La compatibilité D8 (catalogues existants,
> `embedder_fingerprint` au format `family/model@dim`) est
> préservée par le `LegacyAdapter` (cf. § 9.4).

---

## Table des matières

1. [Pain points + objectifs](#1-pain-points--objectifs)
2. [Threat model (supply-chain ML)](#2-threat-model-supply-chain-ml)
3. [Bundle format spec](#3-bundle-format-spec)
4. [Manifest JSON schema (autoritative)](#4-manifest-json-schema-autoritative)
5. [Signing & verification (Cosign-style détaché)](#5-signing--verification-cosign-style-détaché)
6. [Build pipeline — `efe-bundle pack`](#6-build-pipeline--efe-bundle-pack)
7. [Distribution — OCI / S3 / volume](#7-distribution--oci--s3--volume)
8. [Loader Rust — `embedder/bundle.rs`](#8-loader-rust--embedderbundlers)
9. [Catalog ↔ bundle fingerprint binding](#9-catalog--bundle-fingerprint-binding)
10. [Multi-bundle support (per-tenant routing)](#10-multi-bundle-support-per-tenant-routing)
11. [Python adapter — backwards compat D8](#11-python-adapter--backwards-compat-d8)
12. [Helm + K8s integration](#12-helm--k8s-integration)
13. [Observability — D16 instruments + dashboards](#13-observability--d16-instruments--dashboards)
14. [Configuration surface (env + Helm values)](#14-configuration-surface-env--helm-values)
15. [Test plan](#15-test-plan)
16. [Migration path D8 → D16](#16-migration-path-d8--d16)
17. [SLOs D16-BND-*](#17-slos-d16-bnd)
18. [Risk register & open questions](#18-risk-register--open-questions)
19. [Appendix A — alternatives considered](#19-appendix-a--alternatives-considered)
20. [Appendix B — glossaire](#20-appendix-b--glossaire)
21. [Appendix C — anti-corruption story (legacy fingerprint)](#21-appendix-c--anti-corruption-story-legacy-fingerprint)

---

## 1. Pain points + objectifs

### 1.1 Statut quo (D8–D15)

Le chemin embedder en production aujourd'hui passe par
[`clients/python/efe_tool/embedder.py::LocalSTEmbedder`](../../clients/python/efe_tool/embedder.py).
Au runtime :

```text
SwarmService.Encode(req)
    → mode1 backend (Rust)
        → Python FFI to efe_tool.embedder
            → LocalSTEmbedder.encode(text)
                → if first call: SentenceTransformer.from_pretrained(model_name)
                    → huggingface_hub.snapshot_download(model_name)
                        → HTTP GET to https://huggingface.co/...
                        → ~80 MiB download, ~30 s on cold cache
                        → unpacked under ~/.cache/huggingface/hub/
                → SentenceTransformer.encode(text)
                    → returns np.ndarray[float32, (384,)]
            → returns dict to Rust
        → returns gRPC response
```

Cinq problèmes structurels :

| Problème | Cf. § | Impact ops |
|---|---|---|
| Network egress requis | 1.2 | Inacceptable air-gap |
| Pas de byte-exact pinning | 1.3 | Reproductibilité cassée |
| Pas de signature | 1.4 | Supply-chain non-mitigé |
| Cold-start non-borné | 1.5 | Viole SLO **M11-RPC-3** (P99 < 200 ms) au scale-up |
| Cache HF Hub partagé | 1.6 | Contamination inter-tenants en cas de takeover |

### 1.2 Pourquoi un fetch externe ne tient pas en air-gap

Les déploiements régulés (banques, santé, défense) appliquent un
`NetworkPolicy` egress restrictif. Typiquement seuls les CIDR
internes sont accessibles ; `huggingface.co` est explicitement
bloqué. Sans bundle local, EFE ne peut pas démarrer.

Workaround actuel : pré-populer manuellement
`~/.cache/huggingface/hub/` dans l'image Docker. Mais alors :

- l'image grossit de 80 MiB (MiniLM) à 2 GiB (XLM-R)
- pas de signature : on baked le bundle dans l'image, mais on
  perd la séparation entre l'image (signée par cosign d'image)
  et le modèle (qui devrait être signé par sa propre clef
  ML-team)
- pas de hot-swap : pour upgrader le modèle, il faut rebuild +
  re-déployer l'image entière. Latence d'upgrade ~30 min.

### 1.3 Reproductibilité — `from_pretrained` ne pin pas

`huggingface_hub.snapshot_download(model_name)` accepte un
argument `revision=` mais le défaut résout `main` à l'instant T.
Trois pods déployés à T, T+5min, T+10min peuvent loader trois
SHAs distincts si le mainteneur du modèle pousse en `main`
entretemps. Pas de signal d'erreur — la fonction retourne
silencieusement le SHA résolu.

Conséquence : deux pods avec des poids légèrement différents
peuvent produire des embeddings divergents pour le même texte.
Si le catalogue D12 a été fitté sur l'un, les classifications
sur l'autre dérivent — et nos métriques D6 (F1, accuracy SLO)
peuvent baisser sans que l'opérateur sache pourquoi.

### 1.4 Supply-chain — HF Hub ne signe pas

HuggingFace Hub n'expose pas de signature publique sur les
poids. Si un attaquant prend le contrôle du compte
`sentence-transformers`, il peut pousser des poids backdoorés.
Tout pod EFE qui fetch après le takeover absorbe l'attaque
sans détection.

Le standard émergent (cf. SLSA, Sigstore, ML-Bom) est :

1. Le mainteneur (ou son CI) signe les poids avec une clef
   privée.
2. Le verifier (EFE) connaît la clef publique attendue.
3. Avant tout chargement, EFE vérifie la signature détachée.
4. Mismatch → refus + alerte SOC.

D14-bis a déjà introduit ce pattern pour l'audit log
(HMAC-chain). D16 l'applique aux poids ML : signature
asymmetric (Ed25519) sur le `manifest.json` qui contient les
hashes SHA-256 de chaque fichier du bundle.

### 1.5 Cold-start vs SLO RPC

SLO **M11-RPC-3** : `Execute` P99 < 200 ms à 95 % du temps,
mesuré en glissant 1 h. Le premier `Encode` après scale-up
inclut le `from_pretrained` qui prend ~30 s sur un cluster
avec 1 Gbit/s d'egress vers HF Hub. P99 explose pendant
~5 min après chaque scale-up event. À 30 réplicas et
~3 scale-ups/jour (charge variable B2B), c'est ~15 min/jour
de violation SLO.

Bundling local (au pire un PVC monté en `ReadOnlyMany`)
ramène le cold-start à ~2-5 s (lecture disque + init ONNX).
P99 reste sous 200 ms même au premier appel.

### 1.6 Cache HF Hub partagé — risque inter-tenant

Aujourd'hui `~/.cache/huggingface/hub/` est partagé entre
processus. Si on adopte demain un *fetch dynamique par
tenant* (un tenant pull son modèle custom), les fichiers
co-existent dans le même répertoire, accessibles par tous
les processus du pod. C'est une violation de la zero-trust
per-tenant inaugurée à D11.

Bundling avec un répertoire par bundle, mounted en
read-only, ferme cette surface.

### 1.7 Objectifs D16 (mesurables)

| ID | Objectif | Métrique |
|---|---|---|
| **O-16-1** | Eliminer le fetch HF Hub au runtime en mode `bundle` | 0 connexion sortante vers `huggingface.co` durant un E2E `efe-server serve` (vérifié par `tcpdump` dans le test d'intégration) |
| **O-16-2** | Cold-start embedder < 5 s p99 en mode bundle | `efe_embedder_load_duration_seconds` (D16 metric) p99 < 5 |
| **O-16-3** | Reproductibilité byte-exact | Deux runs de `efe-bundle pack` sur les mêmes inputs produisent des bundles avec le même `manifest.sha256` |
| **O-16-4** | Signature obligatoire en prod | `EFE_BUNDLE_REQUIRE_SIGNATURE=true` est le défaut Helm prod ; loader refuse les bundles non-signés |
| **O-16-5** | Compat D8 préservée | Tous les tests existants `LocalSTEmbedder` passent inchangés ; nouveau path opt-in via `EFE_EMBEDDER_BACKEND=bundle` |
| **O-16-6** | Cardinalité Prometheus < 100 séries D16 | Métriques § 13 — `bundle_id` × `outcome` borné par opérateur |

---

## 2. Threat model (supply-chain ML)

Les threat models EFE précédents (M12 § 8, D11 zero-trust,
D13 PII, D14-bis tamper-evidence) couvraient le RPC, l'auth,
les données, et l'audit log. D16 ajoute la couche **artefacts
ML** : poids, tokenizer, config inference.

### 2.1 Catalogue des menaces

| ID | Menace | Vecteur | Conséquence | Mitigation D16 |
|---|---|---|---|---|
| **T-16-1** | Modèle backdooré | HF account takeover | Embeddings biaisés → toute classification compromise | Signature détachée Ed25519 sur manifest, trust-store par `pubkeys.json` |
| **T-16-2** | Modèle remplacé en transit | MITM sur HTTP fetch | Idem T-16-1 | TLS obligatoire + signature détachée (defense in depth) |
| **T-16-3** | Modèle bundle altéré on-disk | Compromise du PVC ou du registry OCI | Idem | Hash SHA-256 par fichier dans le manifest, vérifié à chaque load |
| **T-16-4** | Modèle drift sans détection | Re-tag silencieux par mainteneur | Cones D6 fittés sur géométrie A, embeddings produits par géométrie B | `bundle_fingerprint` byte-exact stocké au catalogue (D14 schema), refus de l'`Evolve` si mismatch |
| **T-16-5** | Tokenizer ≠ poids | Bundle pack incomplet (oubli `tokenizer.json`) | Encodings garbage, jamais détectable au runtime | `manifest.json::components` énumère chaque composant + son hash ; loader refuse si un required est absent |
| **T-16-6** | Bundle plus récent que le catalogue | Upgrade embedder mais oubli de re-fitter cones | Mauvaise classification silencieuse | `register` RPC refuse si le `bundle_fingerprint` du catalogue ≠ celui chargé ; opérateur force la re-fit |
| **T-16-7** | Bundle malicieux uploadé via `efe-bundle` CLI par un opérateur compromis | Insider threat | Mêmes effets que T-16-1 | Signature côté CI seulement (pas en CLI dev) ; trust-store force la pubkey de la CI ; SBOM inclus dans le manifest |
| **T-16-8** | Bundle exfiltre des données via callback ONNX | ONNX Runtime exécute des `Custom Op` réseau | Data exfil au runtime | Whitelist de provider ORT (`CPUExecutionProvider`, `CUDAExecutionProvider`) ; refus de `TensorrtExecutionProvider`-with-custom-loader ; sandboxing par K8s `seccompProfile: RuntimeDefault` |
| **T-16-9** | Bundle ré-utilisé entre tenants malgré isolation | Mauvaise config per-tenant routing | Fuite croisée d'inférences | Routing tenant→bundle explicit dans `oidc-issuers.json` ou registry catalogue, jamais déduit ; pin test |
| **T-16-10** | Cache loader corrompu en mémoire | Bug Rust + concurrent access | Crash du process / mauvais résultats | `Arc<RwLock<...>>` + checksum lazy-recheck ; Loom test sur le path `load_or_evict` |

### 2.2 Hors scope D16

- **Differential privacy** sur les embeddings (D17 candidate).
- **Adversarial robustness** (T-bombs sur le tokenizer
  produisant des séquences trop longues qui DoS le pod) —
  on s'appuie sur le `max_seq_len` du bundle + le rate-limit
  M10.
- **Federated learning** des poids — D16 reste un loader
  read-only.

### 2.3 Trust boundaries

```
                    ┌──────────────────────────────┐
                    │     CI (GitHub Actions)      │  trust root
                    │  - signs bundles with        │
                    │    Ed25519 private key       │
                    │  - publishes to OCI registry │
                    └──────────┬───────────────────┘
                               │  signed bundle (manifest.json + manifest.json.sig)
                               ▼
                    ┌──────────────────────────────┐
                    │     OCI Registry             │  semi-trusted
                    │  - stores layers             │
                    │  - returns layers on pull    │
                    └──────────┬───────────────────┘
                               │  pulled bundle
                               ▼
                    ┌──────────────────────────────┐
                    │    K8s pod (efe-server)      │  zero-trust
                    │  - verifies signature        │  (verifies)
                    │  - verifies file hashes      │
                    │  - loads model               │
                    │  - serves Encode RPCs        │
                    └──────────────────────────────┘
```

Trust roots distincts pour bundle (CI Ed25519) et image
container (cosign on image SHA). Compromise d'un seul des
deux n'implique pas compromise de l'autre.

---

## 3. Bundle format spec

### 3.1 Layout disque (canonical)

Un bundle est un **répertoire** (pas une archive single-file)
mounted en read-only :

```
<bundle_root>/
├── manifest.json            # autoritative metadata + per-file SHA-256
├── manifest.json.sig        # detached Ed25519 signature over manifest.json
├── README.md                # human-readable provenance
├── LICENSE                  # model license (Apache-2.0, MIT, OpenRAIL, …)
├── SBOM.json                # CycloneDX 1.5 SBOM
├── weights/
│   ├── model.onnx           # if framework=onnx
│   └── model.safetensors    # if framework=safetensors (alternate)
├── tokenizer/
│   ├── tokenizer.json       # HuggingFace fast-tokenizer JSON
│   ├── tokenizer_config.json
│   ├── special_tokens_map.json
│   └── vocab.txt            # optional, depends on tokenizer family
└── runtime/
    └── runtime_config.json  # max_seq_len, normalize, pooling, dtype
```

**Pourquoi un répertoire et pas une archive ?**

- Les modèles ONNX 2-5 GiB ne tiennent pas en RAM ; on
  veut `mmap` direct sur disque, ce qui suppose un fichier
  natif accessible directement.
- K8s `Secret` mount (D15 pattern) projette des fichiers
  individuels — déjà exposable directement comme un
  répertoire.
- Cosign verifie via le manifest qui liste les fichiers ;
  pas besoin de re-tar/zip.

**Pourquoi pas le format HF natif ?**

Le format HF (`config.json` + `pytorch_model.bin` + `tokenizer.json`
+ `tokenizer_config.json` + …) n'est pas autoritative — les
champs et leur emplacement varient par modèle. On encapsule
dans une **structure stricte EFE** + un manifest stable, ce
qui découple notre loader des évolutions de l'écosystème HF.

### 3.2 Conventions de nommage

`bundle_id` (dans le manifest, dans les métriques) :

```text
{family}/{name}@{version}/{framework}/{dim}
```

Exemples :

```
sentence-transformers/all-MiniLM-L6-v2@1.0.0/onnx/384
BAAI/bge-large-en-v1.5@2.1.0/safetensors/1024
sentence-transformers/paraphrase-multilingual-mpnet-base-v2@1.0.0/onnx/768
```

Le `version` est défini par l'équipe EFE (semver), pas par
le mainteneur HF. Cela permet de re-publier un bundle si on
patch le tokenizer config sans toucher les poids.

### 3.3 `bundle_fingerprint` (cryptographique)

```python
bundle_fingerprint = sha256(manifest.json bytes)
```

Le manifest contient les hashes de tous les fichiers, donc le
fingerprint du manifest *implique* le contenu byte-exact du
bundle. Fingerprint = 32 octets hex (64 chars).

C'est **ce fingerprint** qui est stocké au registre catalogue
D12 (colonne `embedder_fingerprint` étendue à 64 chars
en migration `0005_extend_embedder_fingerprint.sql`, cf.
§ 9.3).

### 3.4 Tailles indicatives

| Modèle | Framework | Bundle on-disk | Mémoire en RAM |
|---|---|---:|---:|
| `all-MiniLM-L6-v2` | ONNX | 88 MiB | 95 MiB |
| `paraphrase-multilingual-mpnet-base-v2` | ONNX | 250 MiB | 280 MiB |
| `BAAI/bge-large-en-v1.5` | safetensors | 510 MiB | 580 MiB |
| `intfloat/multilingual-e5-large` | safetensors | 525 MiB | 600 MiB |
| `Alibaba-NLP/gte-multilingual-base` | ONNX | 290 MiB | 320 MiB |

À 500 MiB de bundle × 30 réplicas, c'est 15 GiB cumulés. Avec
`Volume.subPath` ou un `ReadOnlyMany` PVC, on factorise à 1 ×
500 MiB partagé entre tous les pods. Voir § 7.3.

---

## 4. Manifest JSON schema (autoritative)

### 4.1 Schéma JSON Schema 2020-12

Stocké sous `crates/efe-server/schemas/d16_manifest.schema.json`,
versionné, pin testé contre tout exemple de bundle.

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://efe.example/schemas/d16/manifest.schema.json",
  "title": "EFE D16 Embedder Bundle Manifest",
  "type": "object",
  "additionalProperties": false,
  "required": [
    "manifest_version",
    "bundle_id",
    "framework",
    "dim",
    "components",
    "runtime",
    "build",
    "license"
  ],
  "properties": {
    "manifest_version": {
      "type": "string",
      "const": "1.0",
      "description": "Manifest schema version. Bumped on breaking changes; D16 ships v1.0."
    },
    "bundle_id": {
      "type": "string",
      "pattern": "^[a-zA-Z0-9._/-]+@[0-9]+\\.[0-9]+\\.[0-9]+/[a-z]+/[0-9]+$",
      "description": "{family}/{name}@{semver}/{framework}/{dim}, e.g. 'sentence-transformers/all-MiniLM-L6-v2@1.0.0/onnx/384'"
    },
    "framework": {
      "type": "string",
      "enum": ["onnx", "safetensors"],
      "description": "Weight format. ONNX is preferred for runtime portability + GPU EP; safetensors is the fallback for models that don't have an official ONNX export."
    },
    "dim": {
      "type": "integer",
      "minimum": 32,
      "maximum": 8192,
      "description": "Output embedding dimension. Pinned at fingerprint level."
    },
    "components": {
      "type": "array",
      "minItems": 1,
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["path", "sha256", "size_bytes", "required"],
        "properties": {
          "path": {
            "type": "string",
            "pattern": "^[a-zA-Z0-9._/-]+$",
            "description": "Path relative to the bundle root, e.g. 'weights/model.onnx'."
          },
          "sha256": {
            "type": "string",
            "pattern": "^[0-9a-f]{64}$",
            "description": "Lowercase hex SHA-256 of the file bytes."
          },
          "size_bytes": {
            "type": "integer",
            "minimum": 0
          },
          "required": {
            "type": "boolean",
            "description": "If true, loader fails closed when this component is absent or hash mismatches. If false, loader logs a warning and continues."
          }
        }
      }
    },
    "runtime": {
      "type": "object",
      "additionalProperties": false,
      "required": ["max_seq_len", "pooling", "normalize", "dtype"],
      "properties": {
        "max_seq_len": {
          "type": "integer",
          "minimum": 16,
          "maximum": 32768,
          "description": "Tokenizer truncation length. Tokens past this are silently truncated."
        },
        "pooling": {
          "type": "string",
          "enum": ["mean", "cls", "max", "weighted_mean"],
          "description": "Token-pooling strategy applied to per-token outputs."
        },
        "normalize": {
          "type": "boolean",
          "description": "If true, output is L2-normalised before return."
        },
        "dtype": {
          "type": "string",
          "enum": ["float32", "float16", "bfloat16"],
          "description": "Output tensor dtype. Float16 saves bandwidth but may lose precision for cosine similarities near the decision boundary."
        }
      }
    },
    "build": {
      "type": "object",
      "additionalProperties": false,
      "required": ["builder", "built_at_unix_ms", "git_sha", "source_date_epoch"],
      "properties": {
        "builder": {
          "type": "string",
          "description": "URI of the CI run, e.g. 'https://github.com/efe-project/efe/actions/runs/123456789'."
        },
        "built_at_unix_ms": {
          "type": "integer",
          "minimum": 0
        },
        "git_sha": {
          "type": "string",
          "pattern": "^[0-9a-f]{40}$",
          "description": "Git commit SHA of the EFE repo at build time."
        },
        "source_date_epoch": {
          "type": "integer",
          "minimum": 0,
          "description": "SOURCE_DATE_EPOCH used for reproducibility. The bundle MUST hash identically when re-built with this epoch."
        }
      }
    },
    "license": {
      "type": "object",
      "additionalProperties": false,
      "required": ["spdx_id"],
      "properties": {
        "spdx_id": {
          "type": "string",
          "description": "SPDX identifier of the model license, e.g. 'Apache-2.0', 'MIT', 'CC-BY-NC-4.0', 'OpenRAIL-M'."
        },
        "url": {
          "type": "string",
          "format": "uri"
        },
        "redistribution_allowed": {
          "type": "boolean",
          "description": "Whether the license permits redistribution. Loader logs a warning at boot if false; operator must acknowledge via EFE_BUNDLE_ACK_NONREDIST=true."
        }
      }
    },
    "model_origin": {
      "type": "object",
      "description": "Optional provenance trail.",
      "additionalProperties": false,
      "properties": {
        "huggingface_repo": { "type": "string" },
        "huggingface_revision": { "type": "string" },
        "model_card_url": { "type": "string", "format": "uri" }
      }
    },
    "sbom_path": {
      "type": "string",
      "description": "Relative path to a CycloneDX 1.5 SBOM file (typically 'SBOM.json'). Required for prod deployments; optional for dev."
    }
  }
}
```

### 4.2 Exemple

```json
{
  "manifest_version": "1.0",
  "bundle_id": "sentence-transformers/all-MiniLM-L6-v2@1.0.0/onnx/384",
  "framework": "onnx",
  "dim": 384,
  "components": [
    {
      "path": "weights/model.onnx",
      "sha256": "a1b2c3...64chars",
      "size_bytes": 92312456,
      "required": true
    },
    {
      "path": "tokenizer/tokenizer.json",
      "sha256": "0000...",
      "size_bytes": 712431,
      "required": true
    },
    {
      "path": "tokenizer/tokenizer_config.json",
      "sha256": "1111...",
      "size_bytes": 1235,
      "required": true
    },
    {
      "path": "tokenizer/special_tokens_map.json",
      "sha256": "2222...",
      "size_bytes": 695,
      "required": true
    },
    {
      "path": "runtime/runtime_config.json",
      "sha256": "3333...",
      "size_bytes": 312,
      "required": true
    },
    {
      "path": "SBOM.json",
      "sha256": "4444...",
      "size_bytes": 4521,
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
    "builder": "https://github.com/efe-project/efe/actions/runs/9876543210",
    "built_at_unix_ms": 1745875200000,
    "git_sha": "abcdef0123456789abcdef0123456789abcdef01",
    "source_date_epoch": 1745875200
  },
  "license": {
    "spdx_id": "Apache-2.0",
    "url": "https://github.com/UKPLab/sentence-transformers/blob/master/LICENSE",
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

### 4.3 Invariants applicatifs (vérifiés au load)

| ID | Invariant | Source de vérité |
|---|---|---|
| **INV-16-1** | `manifest_version == "1.0"` | hard-coded dans `bundle.rs::SUPPORTED_MANIFEST_VERSION` |
| **INV-16-2** | Tous les `components[*].required = true` doivent exister sur disque ET avoir le SHA attendu | filesystem walk au load |
| **INV-16-3** | `dim ∈ {64, 128, 256, 384, 512, 768, 1024, 1536, 2048, 4096}` (powers of 2 + 384/768/1024 communs) | runtime check, refus sinon |
| **INV-16-4** | `framework == "onnx"` ⇒ `weights/model.onnx` est dans `components` | runtime check |
| **INV-16-5** | `framework == "safetensors"` ⇒ `weights/model.safetensors` est dans `components` | runtime check |
| **INV-16-6** | `runtime.max_seq_len ≤ 16384` (DoS) | runtime check |
| **INV-16-7** | `tokenizer/tokenizer.json` est toujours `required: true` (sinon le bundle est inutilisable) | builder enforces |
| **INV-16-8** | `bundle_id` regex matche le nommage canonique § 3.2 | JSON schema |
| **INV-16-9** | Pour la prod (`EFE_BUNDLE_REQUIRE_SIGNATURE=true`), `manifest.json.sig` existe + verify OK | loader at boot |
| **INV-16-10** | `license.redistribution_allowed = false` ⇒ requires `EFE_BUNDLE_ACK_NONREDIST=true` ou loader log + alert + continue | loader at boot |

### 4.4 Pourquoi `additionalProperties: false`

Identique à la convention OIDC `IssuerConfig` (D15 § 5.2) : un
opérateur qui type `Manifest_Version` au lieu de
`manifest_version` doit voir une erreur immédiate au build,
pas une corruption silencieuse. Pin testée par
`scripts/tests/test_d16_manifest_schema_strict.py`.

---

## 5. Signing & verification (Cosign-style détaché)

### 5.1 Schéma de signature

```
manifest.json     <─ canonical UTF-8 bytes, no trailing newline
   │
   │  Ed25519.sign(manifest.json bytes, sk)
   ▼
manifest.json.sig <─ 64 bytes raw signature (binary)
                     OR base64-armored (for OCI distribution simplicity)
```

**Pourquoi Ed25519 et pas RSA ?**

- Signature courte (64 octets vs 256-512 pour RSA)
- Vérification rapide (~0.1 ms vs 1 ms pour RSA-2048)
- Pas de paramètres flexibles (pas de PKCS-1 v1.5 vs PSS
  ambiguity)
- Standard moderne (RFC 8410, supporté par `ed25519-dalek`
  crate, déjà transitive via `jsonwebtoken` ES256)

**Pourquoi détaché et pas embedded dans manifest ?**

- Le manifest a un hash byte-exact qui sert de
  `bundle_fingerprint`. Si la signature était dans le
  manifest, le hash dépendrait de la signature, ce qui
  empêcherait de re-signer un bundle sans changer son
  fingerprint.
- Pattern emprunté à Cosign (sigstore.dev) qui détache la
  signature pour permettre la rotation de signing keys
  sans changer l'image.

### 5.2 Trust store

Fichier `pubkeys.json` (mounted via K8s Secret en sub-path
`/etc/efe/bundles/pubkeys.json`) :

```json
{
  "schema_version": "1.0",
  "keys": [
    {
      "key_id": "efe-prod-bundle-2026q1",
      "ed25519_public": "rZQzS5L3TpY...44 base64 chars",
      "active_for_signing": false,
      "valid_from": "2026-01-01T00:00:00Z",
      "valid_until": "2026-12-31T23:59:59Z",
      "comment": "Production bundle signing key, 2026 Q1+"
    },
    {
      "key_id": "efe-prod-bundle-2025q4",
      "ed25519_public": "vBQzS5L3TpY...",
      "active_for_signing": false,
      "valid_from": "2025-10-01T00:00:00Z",
      "valid_until": "2026-03-31T23:59:59Z",
      "comment": "Previous prod key — kept for verify until grace period ends"
    }
  ]
}
```

Sémantique :

- Le loader essaie chaque clef dans `keys` jusqu'à trouver
  une qui valide la signature.
- `active_for_signing` est purement informatif côté server
  (la CI utilise la clef actuellement active pour signer).
- `valid_from / valid_until` permettent de scoper une
  validité ; un bundle signé avec une clef expirée est
  rejeté même si la signature crypto est OK.

### 5.3 Rotation de clef

```
T=0       T=+30j         T=+90j        T=+120j
│         │              │              │
old key   ─── verify ────────── verify ─┤
                          │              ── (expired)
              new key  ── verify ───────────────────── verify ──
              new key  ── sign  ───────────────────── sign  ──
```

Procédure d'opérateur :

1. Génère la nouvelle paire (Ed25519 keygen via `efe-bundle keygen`).
2. Append à `pubkeys.json` avec `active_for_signing: false`.
3. Push le secret K8s, restart les pods (rolling).
4. Bascule la CI : la nouvelle clef devient `active_for_signing: true`
   sur la CI uniquement (private key in CI secret store).
5. Tous les bundles depuis le moment T+30j sont signés avec
   la nouvelle clef. Les anciens bundles continuent à
   valider via l'ancienne pubkey.
6. Au moment T+120j, on retire l'ancienne pubkey du
   trust store ET on s'assure qu'aucun bundle en cours
   d'utilisation n'a été signé par cette clef (greppable
   via `efe_bundle_signature_key_id` metric — D16 § 13).

### 5.4 Code de vérification (sketch)

```rust
// crates/efe-server/src/embedder/signature.rs

pub struct VerifiedManifest {
    pub manifest: Manifest,
    pub raw_bytes: Vec<u8>,
    pub key_id: String,  // which trust-store key validated
}

pub fn verify_manifest_signature(
    manifest_bytes: &[u8],
    sig_bytes: &[u8],
    trust: &TrustStore,
    now: SystemTime,
) -> Result<String, SigError> {
    let sig = Signature::from_slice(sig_bytes)
        .map_err(|_| SigError::MalformedSig)?;
    for k in &trust.keys {
        // Skip expired keys.
        if now < k.valid_from || now > k.valid_until {
            continue;
        }
        let pk = VerifyingKey::from_bytes(&k.ed25519_public)
            .map_err(|_| SigError::TrustStore("bad pubkey".into()))?;
        if pk.verify(manifest_bytes, &sig).is_ok() {
            return Ok(k.key_id.clone());
        }
    }
    Err(SigError::NoMatchingKey)
}
```

`VerifiedManifest` est ensuite l'unique handle qui circule —
tout code en aval reçoit déjà-vérifié.

### 5.5 Modes de vérification

| Mode | Env | Behaviour | Usage |
|---|---|---|---|
| **strict** (prod) | `EFE_BUNDLE_REQUIRE_SIGNATURE=true` (default) | Manifest sans `manifest.json.sig` ⇒ refus boot | Helm prod |
| **warn** (staging) | `EFE_BUNDLE_REQUIRE_SIGNATURE=warn` | Signature absente ⇒ `tracing::warn!` + continue ; signature présente mais invalide ⇒ refus | Staging avant la 1re publication CI |
| **skip** (dev only) | `EFE_BUNDLE_REQUIRE_SIGNATURE=false` | Signature ignorée même si présente | Dev / tests |

Pin test : la valeur `false` est interdite quand
`EFE_DEPLOY_TIER=production` (Tier 2/3) — refus du
`ServerConfig::validate()`.

---

## 6. Build pipeline — `efe-bundle pack`

### 6.1 Synopsis CLI

```text
efe-bundle pack \
    --source-dir <path/to/hf-cache-or-onnx-export> \
    --bundle-id 'sentence-transformers/all-MiniLM-L6-v2@1.0.0/onnx/384' \
    --framework onnx \
    --dim 384 \
    --runtime-config '{"max_seq_len":256,"pooling":"mean","normalize":true,"dtype":"float32"}' \
    --license-spdx Apache-2.0 \
    --license-url 'https://...' \
    --license-redist true \
    --hf-repo 'sentence-transformers/all-MiniLM-L6-v2' \
    --hf-revision 'e4ce9877abf3edfe10b0d82785e83bdcb973e22e' \
    --git-sha "$(git rev-parse HEAD)" \
    --source-date-epoch "$SOURCE_DATE_EPOCH" \
    --sign-key /path/to/ed25519-private.pem \
    --out /tmp/bundle/

efe-bundle keygen --out /path/to/keys/
efe-bundle verify --bundle /tmp/bundle/ --pubkeys /path/to/pubkeys.json
efe-bundle inspect --bundle /tmp/bundle/  # prints manifest summary
```

### 6.2 Étapes (idempotentes, reproductibles)

```text
1. Walk source-dir, collect files matching the layout § 3.1.
2. Compute SHA-256 of each file (streaming, sha2 crate).
3. Build the components[] array with stable ordering
   (sorted by path).
4. Resolve build.* fields:
     - built_at_unix_ms = source_date_epoch * 1000
     - git_sha          = from --git-sha
     - source_date_epoch= from --source-date-epoch (env var)
5. Write manifest.json with serde_json::to_vec_pretty +
   trailing newline-strip + canonical JSON form
   (sorted keys, no whitespace beyond what serde adds).
6. Compute manifest.sha256 = sha256(manifest.json bytes).
7. Sign:
     ed25519_dalek.sign(manifest.json bytes, sk) → sig
     write sig to manifest.json.sig (binary 64 bytes)
8. Copy components into <out>/<path-from-manifest>.
9. Generate SBOM.json (CycloneDX 1.5) listing transitive
   tokenizer/runtime deps.
10. Print summary: bundle_id, fingerprint, total size.
```

### 6.3 Reproductibilité

Le bundle hash exact dépend de :

1. **`SOURCE_DATE_EPOCH`** — fige `built_at_unix_ms`.
2. **Ordre des `components[]`** — toujours trié par `path`.
3. **JSON canonique** — `serde_json::Serializer` avec
   `PrettyFormatter::with_indent("  ")` + `with_keys_sorted`.
4. **Pas de path absolus** — toutes les paths dans le
   manifest sont relatives au bundle root.

Test : `efe-bundle pack` deux fois sur les mêmes inputs avec
le même `SOURCE_DATE_EPOCH` produit des bundles avec le
même `manifest.sha256` (pin testé `test_d16_bundle_reproducibility`).

### 6.4 ONNX export (path optionnel)

Pour les modèles HF qui n'ont pas d'export ONNX officiel, on
fournit un script `scripts/d16/export_to_onnx.py` :

```python
# scripts/d16/export_to_onnx.py
import torch
from transformers import AutoModel, AutoTokenizer
from optimum.onnxruntime import ORTModelForFeatureExtraction

def export(model_id: str, out_dir: Path, *, opset: int = 17) -> None:
    model = ORTModelForFeatureExtraction.from_pretrained(
        model_id, export=True
    )
    model.save_pretrained(out_dir / "weights")
    tok = AutoTokenizer.from_pretrained(model_id)
    tok.save_pretrained(out_dir / "tokenizer")
    # Write runtime_config.json by introspecting the model card
    # for max_seq_len + pooling.
    ...
```

L'export est *un step de la CI*, pas un step du runtime. Le
runtime ne fait que charger.

### 6.5 SBOM (CycloneDX 1.5)

```json
{
  "bomFormat": "CycloneDX",
  "specVersion": "1.5",
  "serialNumber": "urn:uuid:...",
  "version": 1,
  "components": [
    {
      "type": "library",
      "name": "transformers",
      "version": "4.40.0",
      "purl": "pkg:pypi/transformers@4.40.0",
      "supplier": { "name": "HuggingFace" },
      "licenses": [{ "license": { "id": "Apache-2.0" } }]
    },
    {
      "type": "library",
      "name": "tokenizers",
      "version": "0.19.1",
      "purl": "pkg:pypi/tokenizers@0.19.1"
    },
    {
      "type": "library",
      "name": "onnxruntime",
      "version": "1.18.0",
      "purl": "pkg:pypi/onnxruntime@1.18.0"
    }
  ]
}
```

Required quand `EFE_BUNDLE_REQUIRE_SBOM=true` (Helm prod).

---

## 7. Distribution — OCI / S3 / volume

### 7.1 Stratégies de distribution

| Stratégie | Contexte | Pros | Cons |
|---|---|---|---|
| **OCI registry** | Default prod | Réutilise auth K8s pull-secrets, cosign-compatible, déjà CDN-cached | Bundle ≠ image ⇒ workaround initContainer ou ORAS |
| **S3-compatible** | Air-gap with object store | Simple à mirroir, pas de re-tag | Auth S3 distincte de K8s (extra Secret) |
| **PVC `ReadOnlyMany`** | On-prem Kubernetes | Pas de download au boot, ultra-fast cold start | Provisioning manuel, pas de versioning natif |
| **Image bake (legacy)** | Tier 1 dev | Trivial | Image bloat ; déconseillé pour prod |

### 7.2 OCI distribution via ORAS

[ORAS](https://oras.land) permet de pousser des artefacts
arbitraires dans un OCI registry (pas seulement des
container images). Workflow :

```bash
# CI: pack + sign + push
efe-bundle pack --out /tmp/bundle ...
oras push ghcr.io/efe-project/bundles/all-minilm-l6-v2:1.0.0 \
    --artifact-type application/vnd.efe.bundle.v1+json \
    /tmp/bundle/

# Pod initContainer: pull
oras pull ghcr.io/efe-project/bundles/all-minilm-l6-v2:1.0.0 \
    -o /etc/efe/bundles/active/
```

OCI tag ↔ semver alignment :

- `:1.0.0` immutable (tag bouge pas)
- `:1.0` mutable, pointe vers latest 1.0.x
- `:latest` interdit en prod (Helm validate refuse)

### 7.3 PVC pre-population (air-gap)

```yaml
apiVersion: v1
kind: PersistentVolumeClaim
metadata:
  name: efe-bundles-readonly
  namespace: efe
spec:
  accessModes: ["ReadOnlyMany"]
  resources:
    requests: { storage: 10Gi }
  storageClassName: nfs-readonly
---
# Deployment volumeMounts: at /etc/efe/bundles, readOnly: true
```

Pre-population off-line :

```bash
# Sur un host avec accès Internet
efe-bundle pack ... --out /local/bundles/all-minilm-l6-v2/
efe-bundle pack ... --out /local/bundles/bge-large-en/

# Air-gap copy
rsync -av /local/bundles/ airgap-host:/path-to-pvc-mount/
```

### 7.4 Sélection du bundle au runtime

L'opérateur déclare au plus haut niveau (Helm value, env var) :

```
EFE_BUNDLE_PATH=/etc/efe/bundles/active/    # single-bundle deployment
```

OU pour multi-bundle :

```
EFE_BUNDLES_ROOT=/etc/efe/bundles/
EFE_BUNDLES_ACTIVE='[
  {"id":"sentence-transformers/all-MiniLM-L6-v2@1.0.0/onnx/384","path":"all-minilm-l6-v2"},
  {"id":"BAAI/bge-large-en-v1.5@2.1.0/safetensors/1024","path":"bge-large-en"}
]'
```

Le routing tenant→bundle vit ailleurs (cf. § 10).

---

## 8. Loader Rust — `embedder/bundle.rs`

### 8.1 Module layout

```
crates/efe-server/src/
├── embedder/
│   ├── mod.rs               # public API
│   ├── manifest.rs          # serde types + JSON Schema validation
│   ├── signature.rs         # Ed25519 verify + trust store
│   ├── trust.rs             # TrustStore type + load_pubkeys_file
│   ├── bundle.rs            # Bundle struct + load_bundle_dir
│   ├── verify.rs            # walk components + sha256 check
│   ├── runtime/
│   │   ├── mod.rs
│   │   ├── onnx.rs          # ONNX Runtime adapter
│   │   └── safetensors.rs   # safetensors adapter (defers to Python via FFI for inference)
│   ├── cache.rs             # in-memory model session cache
│   └── tests/
│       ├── fixtures/        # tiny test bundles
│       └── ...
└── ...
```

### 8.2 Public types

```rust
//! D16 — Embedder bundle loader.

/// A loaded, verified, ready-to-encode bundle.
///
/// Cloning is cheap (Arc<...>). The runtime keeps one Bundle
/// per (active_bundle_id) and shares it across requests via
/// the EmbedderRegistry.
#[derive(Clone)]
pub struct Bundle {
    inner: Arc<BundleInner>,
}

struct BundleInner {
    pub manifest: Manifest,
    pub fingerprint: BundleFingerprint, // 32-byte SHA256 of manifest.json
    pub signing_key_id: Option<String>, // Some(_) if signature was verified
    pub root: PathBuf,
    pub session: Box<dyn EmbedderSession>,
}

/// 32-byte SHA-256 of manifest.json bytes. Hex-encoded for
/// display, stored as bytes in memory.
#[derive(Clone, Copy, Hash, Eq, PartialEq)]
pub struct BundleFingerprint(pub [u8; 32]);

/// The runtime-time interface for a loaded model. Concrete
/// impls in `runtime::onnx` and `runtime::safetensors`.
pub trait EmbedderSession: Send + Sync {
    fn encode_one(&self, text: &str) -> Result<Vec<f32>, EmbedderError>;
    fn encode_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedderError>;
    fn dim(&self) -> usize;
}
```

### 8.3 Top-level load function

```rust
/// Load a bundle from a directory on disk.
///
/// Performs (in order):
///   1. Read + parse manifest.json.
///   2. Verify signature (per `policy`).
///   3. Walk components, verify SHA256 of each required file.
///   4. Instantiate a runtime session (ONNX or safetensors).
///   5. Return the verified Bundle.
///
/// Fail-fast on any step.
pub async fn load_bundle_dir(
    root: impl AsRef<Path>,
    policy: &SignaturePolicy,
    trust: &TrustStore,
) -> Result<Bundle, BundleLoadError> {
    let root = root.as_ref().to_path_buf();
    let manifest_path = root.join("manifest.json");
    let raw = tokio::fs::read(&manifest_path).await?;
    let manifest: Manifest = serde_json::from_slice(&raw)?;

    // INV-16-1
    if manifest.manifest_version != "1.0" {
        return Err(BundleLoadError::UnsupportedManifestVersion(
            manifest.manifest_version.clone()));
    }

    // Step 2 — signature.
    let signing_key_id = match policy {
        SignaturePolicy::Required => {
            let sig_path = root.join("manifest.json.sig");
            let sig_bytes = tokio::fs::read(&sig_path).await?;
            let key_id = signature::verify_manifest_signature(
                &raw, &sig_bytes, trust, SystemTime::now(),
            )?;
            Some(key_id)
        }
        SignaturePolicy::Warn => {
            // try to verify, log warn if missing
            ...
        }
        SignaturePolicy::Skip => None,
    };

    // Step 3 — file integrity.
    verify::verify_components(&manifest, &root).await?;

    // Step 4 — runtime.
    let session = runtime::instantiate(&manifest, &root)?;

    // Step 5 — assemble.
    let fingerprint = BundleFingerprint(sha2::Sha256::digest(&raw).into());
    Ok(Bundle {
        inner: Arc::new(BundleInner {
            manifest, fingerprint, signing_key_id,
            root, session,
        }),
    })
}
```

### 8.4 ONNX runtime adapter (sketch)

```rust
//! crates/efe-server/src/embedder/runtime/onnx.rs
//!
//! Uses ort = "2.0" (https://github.com/pykeio/ort).

use ort::{Environment, Session, SessionBuilder, Value};

pub struct OnnxSession {
    session: Session,
    tokenizer: tokenizers::Tokenizer,
    runtime_cfg: RuntimeConfig,
}

impl OnnxSession {
    pub fn new(
        manifest: &Manifest, root: &Path
    ) -> Result<Self, EmbedderError> {
        let env = Environment::builder().with_name("efe").build()?;
        let weights_path = root.join("weights/model.onnx");
        let session = SessionBuilder::new(&env)?
            .with_optimization_level(ort::GraphOptimizationLevel::Level3)?
            .with_intra_threads(1)?  // pinned to one thread per pod;
                                      // tonic's Tokio handles parallelism
            .with_model_from_file(&weights_path)?;
        let tokenizer_path = root.join("tokenizer/tokenizer.json");
        let tokenizer = tokenizers::Tokenizer::from_file(tokenizer_path)?;
        Ok(Self {
            session, tokenizer,
            runtime_cfg: manifest.runtime.clone(),
        })
    }
}

impl EmbedderSession for OnnxSession {
    fn encode_one(&self, text: &str) -> Result<Vec<f32>, EmbedderError> {
        let enc = self.tokenizer
            .encode(text, true)
            .map_err(|e| EmbedderError::Tokenizer(e.to_string()))?;
        // Truncate to max_seq_len.
        let ids = enc.get_ids().iter()
            .take(self.runtime_cfg.max_seq_len)
            .copied().collect::<Vec<_>>();
        let mask = vec![1; ids.len()];
        // Build ORT input tensors.
        ...
        let outputs = self.session.run(...)?;
        let pooled = pool(outputs[0], &mask, self.runtime_cfg.pooling);
        if self.runtime_cfg.normalize { l2_normalize(&mut pooled); }
        Ok(pooled)
    }
}
```

### 8.5 Cache + concurrent loads

Le loader peut être appelé concurremment (par exemple deux
tasks au boot qui accèdent à `EmbedderRegistry::ensure_loaded`).
Pour éviter les double-loads, on utilise un pattern singleflight :

```rust
pub struct EmbedderRegistry {
    bundles: Arc<RwLock<HashMap<BundleId, Bundle>>>,
    inflight: Arc<Mutex<HashMap<BundleId, Arc<Notify>>>>,
}

impl EmbedderRegistry {
    pub async fn ensure_loaded(
        &self,
        id: &BundleId,
        path: &Path,
        policy: &SignaturePolicy,
        trust: &TrustStore,
    ) -> Result<Bundle, BundleLoadError> {
        // Fast path : already loaded.
        if let Some(b) = self.bundles.read().await.get(id) {
            return Ok(b.clone());
        }
        // Slow path : load via singleflight.
        let notify = {
            let mut inflight = self.inflight.lock().await;
            if let Some(n) = inflight.get(id) {
                let n = n.clone();
                drop(inflight);
                n.notified().await;
                // Re-check map.
                return self.bundles.read().await
                    .get(id).cloned()
                    .ok_or(BundleLoadError::ConcurrentLoadFailed);
            }
            let n = Arc::new(Notify::new());
            inflight.insert(id.clone(), n.clone());
            n
        };
        // We're the leader for this load.
        let res = load_bundle_dir(path, policy, trust).await;
        let mut inflight = self.inflight.lock().await;
        inflight.remove(id);
        notify.notify_waiters();
        let bundle = res?;
        self.bundles.write().await.insert(id.clone(), bundle.clone());
        Ok(bundle)
    }
}
```

### 8.6 Errors

```rust
#[derive(Debug, Error)]
pub enum BundleLoadError {
    #[error("manifest IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("manifest JSON parse: {0}")]
    Json(#[from] serde_json::Error),
    #[error("manifest_version {0:?} unsupported (only \"1.0\")")]
    UnsupportedManifestVersion(String),
    #[error("signature verification failed: {0}")]
    Signature(#[from] SigError),
    #[error("file hash mismatch at {path}: expected {expected}, got {actual}")]
    HashMismatch { path: String, expected: String, actual: String },
    #[error("required component {0} missing from disk")]
    MissingComponent(String),
    #[error("runtime instantiation: {0}")]
    Runtime(String),
    #[error("singleflight concurrent load failed (peer leader returned error)")]
    ConcurrentLoadFailed,
}
```

---

## 9. Catalog ↔ bundle fingerprint binding

### 9.1 D12 status quo

D12 a introduit la colonne `embedder_fingerprint` au niveau
catalogue (cf. `d12_catalog_admin_versioning_architecture.md`
§ 5.4). Format actuel : `family/model@dim` (ex.
`sentence-transformers/all-MiniLM-L6-v2@384`). Ce format
était suffisant pour détecter les changements grossiers, mais
pas les drifts byte-exact.

### 9.2 Extension D16

```sql
-- migrations/0005_extend_embedder_fingerprint.sql

-- D16 : extend the fingerprint column to hold a 64-char hex SHA-256
-- of the bundle manifest. The legacy `family/model@dim` format
-- becomes the `legacy_embedder_fingerprint` column so existing
-- catalogues keep validating until they're re-fitted on the new
-- bundle.
ALTER TABLE catalogs
    ADD COLUMN bundle_fingerprint CHAR(64) NULL,
    ADD COLUMN legacy_embedder_fingerprint TEXT NULL;

-- Backfill from the old column.
UPDATE catalogs
   SET legacy_embedder_fingerprint = embedder_fingerprint
 WHERE bundle_fingerprint IS NULL;

-- Add a check that exactly one is non-null. Catalogues fitted
-- pre-D16 use legacy_*; catalogues fitted post-D16 use bundle_*.
ALTER TABLE catalogs
    ADD CONSTRAINT catalogs_fingerprint_xor
        CHECK (
            (bundle_fingerprint IS NOT NULL AND legacy_embedder_fingerprint IS NULL)
         OR (bundle_fingerprint IS NULL     AND legacy_embedder_fingerprint IS NOT NULL)
        );

-- Index for fast lookup.
CREATE INDEX catalogs_bundle_fingerprint_idx ON catalogs (bundle_fingerprint)
WHERE bundle_fingerprint IS NOT NULL;
```

### 9.3 Verify path

À chaque `Register` / `Evolve` / `Get` RPC, le catalogue
handler récupère le `bundle_fingerprint` du catalog row et
le compare au fingerprint du `Bundle` actif :

```rust
// services/catalog.rs::Register
fn register(...) -> ... {
    let active = self.embedder_registry.active(&tenant)?;
    if let Some(catalog_fp) = req.catalog.bundle_fingerprint {
        if catalog_fp != active.fingerprint() {
            return Err(Status::failed_precondition(
                "catalog was fitted on a different bundle than the active one"
            ));
        }
    }
    // ... write to catalog registry ...
}
```

### 9.4 Legacy backwards-compat

Pour les catalogues fittés pré-D16 (`legacy_embedder_fingerprint`
non-null), on fait un soft-check :

- Si `legacy_embedder_fingerprint == "family/model@dim"` et
  le bundle actif a le même `family/model@dim` (extrait de
  son `manifest.bundle_id`), on accepte avec un
  `tracing::warn!` "legacy fingerprint match — re-fit
  recommended for byte-exact pinning".
- Si non, hard reject.

Cf. `auth/legacy_fingerprint.rs` qui implémente
l'extraction `bundle_id → family/model@dim`.

### 9.5 Migration API

`efe-onboard upgrade-catalog --tenant acme --catalog-name foo`
re-fit le catalogue sur le bundle actif et bump
`bundle_fingerprint`. C'est une opération destructive du
point de vue des cones (les anchors changent), donc protégée
par `--confirm`.

---

## 10. Multi-bundle support (per-tenant routing)

### 10.1 Use cases

| Use case | Routing key | Exemple |
|---|---|---|
| **One bundle, all tenants** | n/a | Tier 1 dev |
| **Per-language** | `tenant.locale` (extrait du JWT ou du tenant config) | `fr` → multilingual MPNet, `en` → MiniLM |
| **Per-tier** | `tenant.tier` | `enterprise` → BGE-large, `starter` → MiniLM |
| **Per-tenant explicit** | tenant_id direct mapping | Régulé : `acme` → bundle dédié certifié |

### 10.2 Resolver

```rust
pub trait BundleResolver: Send + Sync {
    /// Pick a bundle for this request. Returns the BundleId
    /// to look up in the registry.
    fn resolve(&self, ctx: &RequestContext) -> Result<BundleId, ResolveError>;
}
```

Implémentations :

1. **`StaticResolver`** — toujours le même bundle. Default.
2. **`PerTenantResolver`** — config map `tenant_id → bundle_id`,
   fallback to default.
3. **`LocaleResolver`** — query `RequestContext.locale_hint`.

### 10.3 Config map (Helm)

```yaml
config:
  embedder:
    bundles:
      - id: "sentence-transformers/all-MiniLM-L6-v2@1.0.0/onnx/384"
        path: "/etc/efe/bundles/all-minilm-l6-v2"
        default: true
      - id: "BAAI/bge-large-en-v1.5@2.1.0/safetensors/1024"
        path: "/etc/efe/bundles/bge-large-en"
    routing:
      strategy: "per-tenant"   # static | per-tenant | locale
      per_tenant:
        - tenant: "acme"
          bundle: "BAAI/bge-large-en-v1.5@2.1.0/safetensors/1024"
        - tenant: "globex"
          bundle: "sentence-transformers/all-MiniLM-L6-v2@1.0.0/onnx/384"
      default_bundle: "sentence-transformers/all-MiniLM-L6-v2@1.0.0/onnx/384"
```

### 10.4 Cardinalité metric

`efe_embedder_encode_total{bundle_id, tenant, outcome}` —
borne supérieure = `n_bundles × n_tenants × 3 outcomes`.
Pour 5 bundles × 100 tenants × 3 = 1500 séries. **Au-delà
de 1500, on remplace le label `tenant` par `tier` ou par
un hash bucket.** Pin testé.

---

## 11. Python adapter — backwards compat D8

### 11.1 Le contrat D8

`efe_tool/embedder.py::Embedder` Protocol :

```python
class Embedder(Protocol):
    def encode(self, text: str) -> np.ndarray: ...
    def encode_batch(self, texts: Sequence[str]) -> np.ndarray: ...
    def dim(self) -> int: ...
    def fingerprint(self) -> str: ...
```

Le contrat ne change pas. D16 ajoute une nouvelle classe
`BundleEmbedder` qui implémente `Embedder` mais bridge vers
la couche Rust.

### 11.2 `BundleEmbedder` (Python)

```python
# clients/python/efe_tool/embedder/bundle.py

import numpy as np
from . import Embedder

class BundleEmbedder:
    """D16 — wraps a loaded EFE bundle.

    The actual model session lives in the Rust core; this Python
    class is a thin FFI wrapper. Use this when you need byte-exact
    pinning + signature verification.
    """

    def __init__(self, bundle_path: str | Path) -> None:
        # PyO3 binding: returns a handle to the Rust EmbedderSession.
        from efe_native import bundle_load
        self._handle = bundle_load(str(bundle_path))
        self._dim = self._handle.dim
        self._fingerprint = self._handle.fingerprint_hex

    def encode(self, text: str) -> np.ndarray:
        return np.asarray(self._handle.encode(text), dtype=np.float32)

    def encode_batch(self, texts):
        return np.asarray(self._handle.encode_batch(list(texts)), dtype=np.float32)

    def dim(self) -> int:
        return self._dim

    def fingerprint(self) -> str:
        # Return the byte-exact 64-char hex fingerprint for D16
        # bundles. Catalogues store this directly.
        return f"bundle:{self._fingerprint}"
```

### 11.3 `LegacyAdapter` (D8 path inchangé)

`LocalSTEmbedder` continue d'exister et continue de
retourner `family/model@dim` dans `fingerprint()`. Le runtime
serveur accepte les deux types via le `Embedder` Protocol.

### 11.4 Sélection au runtime client (CLI)

```bash
efe-tool fit --catalog foo \
    --embedder bundle:/etc/efe/bundles/all-minilm-l6-v2     # D16
efe-tool fit --catalog foo \
    --embedder hf:sentence-transformers/all-MiniLM-L6-v2    # legacy D8
```

---

## 12. Helm + K8s integration

### 12.1 Values.yaml additions

```yaml
config:
  # ... existing fields ...
  embedder:
    # **D16** — embedder backend selection.
    #   - "legacy" (default) → LocalSTEmbedder, fetches from HF Hub.
    #   - "bundle"           → loads from a verified bundle on disk.
    backend: "legacy"

    bundle:
      # Required when backend = "bundle".
      # Path to the bundle directory inside the pod (after Secret /
      # PVC mount).
      path: "/etc/efe/bundles/active"

      # Signature verification policy:
      #   - "required" (prod default)
      #   - "warn"     (staging — allows unsigned for first deploy)
      #   - "skip"     (Tier 1 dev only — refused if production tier)
      requireSignature: "required"

      # Path to the trust store (pubkeys.json). Mounted via Secret.
      trustStorePath: "/etc/efe/bundles/pubkeys.json"

      # Whether to acknowledge non-redistributable licenses.
      # If false, loader refuses bundles whose license forbids
      # redistribution. Set to true after legal review.
      acknowledgeNonRedistributable: false

      # SBOM requirement (prod hardening).
      requireSbom: true

      # OCI distribution config (optional — used by the initContainer).
      oci:
        enabled: false
        registry: "ghcr.io/efe-project"
        bundle: "all-minilm-l6-v2"
        tag: "1.0.0"
        pullSecret: ""

      # Existing Secret with the bundle (alternative to OCI/PVC).
      existingSecretName: ""
      existingSecretKey: ""

  # Multi-bundle (advanced).
  embedderRouting:
    enabled: false
    strategy: "static"
    bundles: []
```

### 12.2 InitContainer (OCI pull path)

```yaml
initContainers:
  - name: bundle-pull
    image: ghcr.io/efe-project/oras:latest
    command:
      - sh
      - -c
      - |
        oras pull \
          {{ .Values.config.embedder.bundle.oci.registry }}/bundles/{{ .Values.config.embedder.bundle.oci.bundle }}:{{ .Values.config.embedder.bundle.oci.tag }} \
          -o /etc/efe/bundles/active/
    volumeMounts:
      - name: bundle-active
        mountPath: /etc/efe/bundles/active
volumes:
  - name: bundle-active
    emptyDir:
      sizeLimit: 4Gi
```

### 12.3 Trust store mount

Pattern identique à D15 OIDC issuers : Secret K8s sub-path.

```yaml
volumeMounts:
  - name: bundle-trust
    mountPath: /etc/efe/bundles
    subPath: pubkeys.json
    readOnly: true

volumes:
  - name: bundle-trust
    secret:
      secretName: {{ .Values.config.embedder.bundle.trustStoreSecretName }}
      items:
        - key: pubkeys.json
          path: pubkeys.json
      defaultMode: 0444
```

### 12.4 Resource sizing

| Bundle | Container memory request | limit |
|---|---:|---:|
| MiniLM (88 MiB) | 256 Mi | 512 Mi |
| MPNet (250 MiB) | 512 Mi | 1 Gi |
| BGE-large (510 MiB) | 1 Gi | 2 Gi |
| XLM-R-large (1.6 GiB) | 2 Gi | 4 Gi |

ONNX runtime alloue ~2× weights size en RSS au steady state.

### 12.5 ImagePullPolicy + signing

Si on push les bundles via OCI, on peut activer la
**vérification cosign** via `Kyverno` ou `Sigstore Policy
Controller` au niveau du cluster. Cette vérif s'ajoute à la
nôtre (signature manifest dans bundle) — defense in depth.

---

## 13. Observability — D16 instruments + dashboards

### 13.1 Prometheus metrics

| Métrique | Type | Labels | But |
|---|---|---|---|
| `efe_bundle_load_duration_seconds` | Histogram | `bundle_id`, `outcome` | Boot performance + alerting on slow loads |
| `efe_bundle_load_total` | Counter | `bundle_id`, `outcome` | Counter pour rate/error% |
| `efe_bundle_signature_verifications_total` | Counter | `bundle_id`, `key_id`, `result` | Audit + key rotation visibility |
| `efe_bundle_hash_verifications_total` | Counter | `bundle_id`, `result` | Per-component hash check stats |
| `efe_embedder_encode_duration_seconds` | Histogram | `bundle_id`, `tenant` | Hot-path latency |
| `efe_embedder_encode_total` | Counter | `bundle_id`, `tenant`, `outcome` | Throughput + error rate |
| `efe_embedder_active_bundles` | IntGauge | (none) | Sanity: how many bundles are loaded right now |
| `efe_embedder_resident_bytes` | IntGauge | `bundle_id` | RSS attribution per bundle |
| `efe_embedder_tokenizer_truncations_total` | Counter | `bundle_id`, `tenant` | Detection of inputs that got truncated to max_seq_len |

### 13.2 Cardinality budget

| Composant | Worst case | Justification |
|---|---:|---|
| 5 bundles × 100 tenants × 3 outcomes | 1500 | Encode counter |
| 5 bundles × 4 outcomes | 20 | Load duration |
| 5 bundles × 4 keys × 3 results | 60 | Signature verifications |
| 5 bundles × 4 results | 20 | Hash verifications |
| 5 bundles × 1 dim | 5 | Resident bytes |

Total ~1700 séries — bien sous le budget D13 50 000 panic.

### 13.3 Outcomes catalog

| `outcome` | Sens | Path code |
|---|---|---|
| `ok` | Load/encode succeeded | happy path |
| `signature_failed` | Signature missing or invalid | signature.rs |
| `hash_mismatch` | A required component had wrong SHA | verify.rs |
| `manifest_invalid` | JSON schema violation | manifest.rs |
| `runtime_init_failed` | ONNX session create panic | runtime/onnx.rs |
| `tokenizer_failed` | Tokenizer JSON unparseable | runtime/onnx.rs |
| `empty_input` | encode("") | encode |
| `too_long` | input > max_seq_len after tokenize (still OK after truncate) | encode |
| `nan_output` | encoding produced NaN — most likely numeric instability or backdoored weights | encode |

### 13.4 Spans OTel

```text
efe.bundle.load           — top-level load span
efe.bundle.signature      — sig verify subspan
efe.bundle.verify_files   — hash check subspan
efe.bundle.runtime_init   — ORT session build subspan
efe.embedder.encode       — per-RPC encode (deja D13)
efe.embedder.tokenize     — tokenize subspan
efe.embedder.forward      — ORT forward pass subspan
efe.embedder.pool         — pooling subspan
```

### 13.5 Grafana dashboard `D16-embedder`

8 panels :

1. **Active bundles** — `efe_embedder_active_bundles`
2. **Load duration p50/p99** — histogram heatmap
3. **Sign verify rate par key_id** — pie chart pour key rotation viz
4. **Hash mismatch alerts** — counter rate (should stay 0)
5. **Encode p50/p99 par bundle** — heatmap par bundle
6. **Encode rate par tenant top-10** — bar chart
7. **Resident bytes par bundle** — line chart
8. **Truncation rate** — bar chart par tenant (signal sur les inputs déraisonnables)

### 13.6 Alerts D16

| Alert | Rule | Severity |
|---|---|---|
| `D16BundleLoadFailing` | `rate(efe_bundle_load_total{outcome!="ok"}[5m]) > 0` | page |
| `D16BundleSignatureMismatch` | `rate(efe_bundle_signature_verifications_total{result!="ok"}[5m]) > 0` | page (security event) |
| `D16BundleHashMismatch` | `rate(efe_bundle_hash_verifications_total{result!="ok"}[5m]) > 0` | page (corruption) |
| `D16EmbedderEncodeSlow` | `histogram_quantile(0.99, rate(efe_embedder_encode_duration_seconds_bucket[5m])) > 0.05` | warn |
| `D16BundleNoActive` | `efe_embedder_active_bundles == 0` for 1m | page |
| `D16EmbedderResidentSurge` | `efe_embedder_resident_bytes > limit*0.9` | warn |
| `D16TruncationStorm` | `rate(efe_embedder_tokenizer_truncations_total[5m]) > 100` | warn (bug client side) |

---

## 14. Configuration surface (env + Helm values)

### 14.1 Env vars (alphabetical)

| Env | Type | Default | Sense |
|---|---|---|---|
| `EFE_EMBEDDER_BACKEND` | enum(`legacy`, `bundle`) | `legacy` | switch chemin embedder |
| `EFE_BUNDLE_PATH` | path | unset | bundle root quand `backend=bundle` |
| `EFE_BUNDLE_REQUIRE_SIGNATURE` | enum(`required`, `warn`, `skip`) | `required` | sign verify policy |
| `EFE_BUNDLE_TRUST_STORE` | path | `/etc/efe/bundles/pubkeys.json` | trust store path |
| `EFE_BUNDLE_REQUIRE_SBOM` | bool | `true` | refuse bundles sans SBOM |
| `EFE_BUNDLE_ACK_NONREDIST` | bool | `false` | ACK pour modèle non-redistribuable |
| `EFE_BUNDLE_LOAD_TIMEOUT_S` | u64 | `60` | timeout pour load step |
| `EFE_EMBEDDER_ENCODE_TIMEOUT_MS` | u64 | `500` | per-call encode timeout |
| `EFE_EMBEDDER_ROUTING_STRATEGY` | enum | `static` | multi-bundle routing |
| `EFE_EMBEDDER_ROUTING_CONFIG` | path | unset | per-tenant routing JSON |

### 14.2 Validation invariants (config.rs)

Étendre `ServerConfig::validate()` :

```rust
impl ServerConfig {
    fn validate_embedder(&self) -> Result<(), ConfigError> {
        if matches!(self.embedder.backend, EmbedderBackend::Bundle) {
            if self.embedder.bundle_path.is_empty() {
                return Err(ConfigError::invalid(
                    "embedder.bundle_path",
                    "must be set when backend=bundle (set EFE_BUNDLE_PATH)",
                ));
            }
            if matches!(
                self.embedder.require_signature,
                SignaturePolicy::Skip
            ) && self.deploy_tier == DeployTier::Production {
                return Err(ConfigError::invalid(
                    "embedder.require_signature",
                    "skip is forbidden when deploy_tier=production",
                ));
            }
        }
        Ok(())
    }
}
```

---

## 15. Test plan

### 15.1 Unit tests Rust (fixtures sous `tests/fixtures/d16_bundles/`)

Petit bundle synthétique de 64 dim avec MockSession qui ne
load pas vraiment ONNX (juste un mock implementation de
`EmbedderSession`) :

| Test | But |
|---|---|
| `manifest_parses_valid_v1_0` | parse OK |
| `manifest_rejects_unknown_field` | `additionalProperties: false` |
| `manifest_rejects_unsupported_version` | INV-16-1 |
| `signature_verify_with_known_key` | happy path |
| `signature_verify_rejects_tampered_manifest` | byte change → reject |
| `signature_verify_rejects_expired_key` | `valid_until` |
| `signature_verify_skips_when_policy_skip` | dev mode |
| `verify_components_passes_on_intact_bundle` | hash match |
| `verify_components_fails_on_mutated_file` | T-16-3 |
| `verify_components_fails_on_missing_required` | INV-16-2 |
| `verify_components_warns_on_missing_optional` | tolerance |
| `bundle_load_dir_e2e_happy` | full pipeline |
| `bundle_load_dir_signature_required_no_sig_fails` | strict mode |
| `bundle_fingerprint_is_deterministic` | reproducibility |
| `registry_singleflight_dedupes_concurrent_loads` | no double-load |
| `legacy_fingerprint_matches_bundle_id` | adapter § 9.4 |

### 15.2 Integration tests (no HF Hub network)

`tests/d16_bundle_integration.rs` :

| Test | But |
|---|---|
| `loads_minimal_onnx_bundle_e2e` | end-to-end avec ONNX très petit (10 KiB) |
| `encode_returns_correct_dim` | dim conformance |
| `encode_normalize_when_set` | l2 norm = 1.0 |
| `encode_truncates_long_input` | INV-16-6 + telemetry |
| `tampered_weights_rejected_at_load` | hash mismatch |
| `wrong_signature_rejected_at_load` | sig mismatch |

### 15.3 Python pin tests (`scripts/tests/test_d16_*.py`)

| Pin test | Assertion |
|---|---|
| `test_d16_helm_values_default_legacy_backend` | back-compat |
| `test_d16_envvars_match_config_rs` | crossvalidation |
| `test_d16_manifest_schema_strict` | additionalProperties:false |
| `test_d16_path_consistency` | values ↔ env ↔ default |
| `test_d16_initcontainer_when_oci_enabled` | conditional template |
| `test_d16_signature_policy_required_in_production_tier` | hard rule |
| `test_d16_cardinality_under_2000` | metrics labels budget |
| `test_d16_alert_runbook_links` | every alert links a runbook |
| `test_d16_bundle_id_regex_matches_manifest_schema` | schema parity |

### 15.4 Reproductibility test

```bash
SOURCE_DATE_EPOCH=1745875200 efe-bundle pack ... --out /tmp/a/
SOURCE_DATE_EPOCH=1745875200 efe-bundle pack ... --out /tmp/b/
cmp /tmp/a/manifest.json /tmp/b/manifest.json
sha256sum /tmp/a/manifest.json /tmp/b/manifest.json | uniq -c | grep " 2 "
```

Pin testé via `tests/d16_reproducibility.rs`.

### 15.5 Threat-model regression tests

| Threat | Test |
|---|---|
| T-16-1 | `tampered_weights_rejected_at_load` |
| T-16-3 | hash mismatch tests |
| T-16-4 | catalog fingerprint mismatch refuse Evolve |
| T-16-5 | missing tokenizer = reject load |
| T-16-6 | catalog evolve refuses on bundle drift |
| T-16-7 | trust store doesn't accept arbitrary keys |

---

## 16. Migration path D8 → D16

### 16.1 Deux mondes coexistent

| Mode | Quand | Embedder |
|---|---|---|
| **legacy** | défaut (`EFE_EMBEDDER_BACKEND=legacy`) | `LocalSTEmbedder` (HF Hub) |
| **bundle** | opt-in (`EFE_EMBEDDER_BACKEND=bundle`) | `BundleEmbedder` |

Le serveur démarre dans le mode demandé. Aucun catalogue
existant ne casse — la colonne `legacy_embedder_fingerprint`
est toujours servie en lecture.

### 16.2 Plan de bascule (per-tenant)

```text
Phase 1 (J0)   — push bundle 1.0.0 to OCI registry. Helm defaults
                 stay legacy.
Phase 2 (J+7)  — staging cluster: backend=bundle. Run E2E.
                 Compare predictions with legacy on a frozen
                 dataset (must agree byte-exact: same model →
                 same vectors).
Phase 3 (J+14) — prod cluster: backend=bundle for new tenants only.
                 Existing tenants keep legacy.
Phase 4 (J+30) — prod: gradually re-fit existing catalogues onto
                 bundle. CLI: efe-onboard upgrade-catalog
                 --tenant <id> --catalog-name <n>.
Phase 5 (J+90) — prod: all catalogues on bundle. Drop legacy
                 path in code (D17).
```

### 16.3 Rollback

À tout moment : `EFE_EMBEDDER_BACKEND=legacy` + restart.
Le runtime re-pointe vers `LocalSTEmbedder`. Les catalogues
fittés en mode bundle ne fonctionnent plus jusqu'à re-fit
en legacy ou re-bascule en bundle. Documenté dans le
runbook D16.

---

## 17. SLOs D16-BND-*

| ID | SLO | Mesure | Target |
|---|---|---|---|
| **D16-BND-1** | Bundle load time | `efe_bundle_load_duration_seconds` p99 | ≤ 5 s |
| **D16-BND-2** | Encode latency | `efe_embedder_encode_duration_seconds` p99 | ≤ 50 ms (CPU) / 5 ms (GPU) |
| **D16-BND-3** | Signature verify | rate of `signature_verifications_total{result!="ok"}` | 0 in any 5-min window |
| **D16-BND-4** | Hash integrity | rate of `hash_verifications_total{result!="ok"}` | 0 in any 5-min window |
| **D16-BND-5** | Reproducibility | bundles built with same SOURCE_DATE_EPOCH have identical manifest.sha256 | 100 % (CI test) |

---

## 18. Risk register & open questions

### 18.1 Known risks

| ID | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| **R-16-1** | ONNX model output drifts from the original PyTorch model after export | medium | medium | CI smoke test compares output on a 100-text golden set ; tolerance < 1e-4 cosine |
| **R-16-2** | Bundle size > 4 Gi exceeds OCI artifact limits | low | high | Document in runbook, fall back to PVC for >4 GiB |
| **R-16-3** | ONNX Runtime CVE | low | medium | Pin ORT version, scan with `cargo audit`, update on release |
| **R-16-4** | Reproducibility broken by `tokenizers` non-deterministic JSON ordering | medium | medium | Pin `tokenizers` version, JSON canonicalize tokenizer.json at pack time |
| **R-16-5** | Trust store key compromise | low | critical | Key rotation procedure § 5.3 ; SLOs on `signature_verifications_total{key_id="..."}` to detect single-key reliance |
| **R-16-6** | First-fetch race when 30 pods start simultaneously and pull the same OCI tag | medium | low | OCI registries handle concurrent pulls fine ; fall-back: PVC pre-population |

### 18.2 Open questions (to revisit at audit)

1. **Q-16-1** : faut-il supporter le re-loading hot (sans
   restart) ? Pour MVP D16 → non. Le re-load implique de
   gérer la transition (in-flight requests sur l'ancien
   bundle vs le nouveau). Reporté à D16-bis si demande
   opérationnelle confirmée.

2. **Q-16-2** : faut-il chiffrer le bundle au repos sur
   PVC ? Les poids ML peuvent être considérés comme des
   secrets industriels. Réponse : option Helm
   `bundle.volumeEncryption: true` qui s'appuie sur le
   storage class CSI (LUKS, KMS).

3. **Q-16-3** : faut-il un `efe-bundle inspect --diff` pour
   comparer deux bundles ? Useful pour les audits.
   Réponse : oui, mais en P2.

4. **Q-16-4** : où s'arrête la responsabilité D16 vs D17
   (model fine-tuning) ? D16 charge des bundles
   pré-entraînés. Le fine-tuning de bundles est D17.

5. **Q-16-5** : faut-il supporter Apple Silicon
   `CoreMLExecutionProvider` pour les Macs en dev ? Oui,
   ORT le supporte ; à activer via feature flag.

6. **Q-16-6** : politique CVE ONNX Runtime — bumper
   automatiquement (renovate bot) ou approuver manuellement
   ? Manuellement (les bumps mineurs cassent souvent les
   ABI ORT).

---

## 19. Appendix A — alternatives considered

### 19.1 Pourquoi pas garder `from_pretrained` + cache local ?

Considéré (status quo + `HF_HOME=/etc/efe/hf-cache`). Rejeté :

- Cache HF Hub n'a pas de signature.
- Pas de `bundle_id` formel — toujours `family/model@dim`,
  pas byte-exact.
- Layout interne du cache HF (commits indirects) instable
  entre versions de `huggingface_hub`.

### 19.2 Pourquoi pas Cosign + image OCI directement ?

Considéré : push les bundles en image OCI standard, signe
avec Cosign, un Kyverno policy au cluster level vérifie au
pull. Rejeté pour MVP D16 :

- Cosign verifie à K8s admission time, pas au runtime du
  pod ; rien n'empêche un attaquant qui a compromis le
  pod de réécrire le bundle ensuite.
- Les images OCI ne sont pas montées en filesystem
  read-only par défaut ; on devrait passer par initContainer
  + emptyDir, ce qui ajoute de la complexité sans
  gagner en sécurité.

Approche retenue : **les deux**. Cosign sur l'image **et**
signature détachée intra-bundle. Defense in depth.

### 19.3 Pourquoi pas ORT C-API direct depuis Python sans Rust ?

Considéré : Python charge ORT directement, Rust handler
appelle Python via FFI. Rejeté :

- Le verify path (signature, hash, layout) est plus simple
  à écrire en Rust où on a déjà sha2 + serde + thiserror.
- Le signing key handling (Ed25519) bénéficie de
  `ed25519-dalek` qui est un crate Rust de référence.
- La duplication du loader code en Python ne nous gagne
  rien — la Python side reste un thin wrapper FFI.

### 19.4 Pourquoi pas un format archive (tar/zip) ?

Considéré : un seul fichier `.efebundle` (tar de tous les
fichiers). Rejeté :

- ONNX models 2-5 GiB ne tiennent pas en RAM si on doit
  unpack et hash en mémoire.
- mmap direct est plus efficace sur fichier natif.
- Le layout répertoire est plus debuggable au runtime
  (`ls /etc/efe/bundles/active/` pour inspecter).

### 19.5 Pourquoi Ed25519 et pas ECDSA P-256 (cohérence avec D15) ?

D15 a introduit ES256 pour les JWTs. On pourrait re-utiliser.
Choix Ed25519 :

- Signature courte (64 vs 71 octets ECDSA DER).
- Vérif plus rapide (`~0.1 ms`).
- Pas de hash agility (pas besoin de specifier SHA-256 en
  plus).
- Crate `ed25519-dalek` plus simple que la lib RustCrypto
  ECDSA.

ES256 reste légitime ; le trust store pourrait supporter
les deux dans une v2 du schema.

---

## 20. Appendix B — glossaire

| Terme | Sens dans D16 |
|---|---|
| **Bundle** | Répertoire on-disk + manifest signé contenant un modèle d'embeddings |
| **Bundle ID** | Identifiant canonique `family/name@semver/framework/dim` |
| **Bundle fingerprint** | SHA-256 de `manifest.json`, 32 octets |
| **Components** | Liste des fichiers du bundle, chacun avec un hash |
| **Manifest** | `manifest.json` — métadonnée autoritative |
| **Detached signature** | `manifest.json.sig` — signature Ed25519 détachée |
| **Trust store** | `pubkeys.json` — liste des clefs publiques acceptées |
| **Signing key** | Clef privée Ed25519 (CI seulement) |
| **Signature policy** | `required` / `warn` / `skip` |
| **SBOM** | Software Bill of Materials, format CycloneDX 1.5 |
| **OCI artifact** | Layer non-image stocké dans un OCI registry |
| **ORAS** | OCI Registry As Storage — outil pour push/pull non-image |
| **Per-bundle session** | Instance de runtime ML (ONNX session) liée à un bundle |
| **Singleflight** | Pattern qui dédupe les loads concurrents |
| **Singularity** | À ne pas confondre avec OCI ; non-utilisé |
| **Legacy fingerprint** | Pre-D16 format `family/model@dim` |

---

## 21. Appendix C — anti-corruption story (legacy fingerprint)

### 21.1 Le besoin

Les catalogues fittés pre-D16 ont leur `embedder_fingerprint`
au format `family/model@dim` (ex.
`sentence-transformers/all-MiniLM-L6-v2@384`). On veut
permettre à ces catalogues de continuer à fonctionner avec un
bundle D16 chargé à runtime — *à condition* que le bundle ait
le même `family/model@dim` que celui utilisé pour fitter.

### 21.2 La solution

Une fonction d'extraction côté loader :

```rust
// crates/efe-server/src/embedder/legacy.rs
pub fn legacy_fingerprint_from_bundle(b: &Bundle) -> String {
    // Extract family/name@dim from bundle_id
    // ex. "sentence-transformers/all-MiniLM-L6-v2@1.0.0/onnx/384"
    //  → "sentence-transformers/all-MiniLM-L6-v2@384"
    let id = &b.manifest.bundle_id;
    // Split: prefix "family/name", "@version", "/framework", "/dim"
    let (family_name, rest) = id.split_once('@').unwrap_or((id, ""));
    let dim = b.manifest.dim;
    format!("{family_name}@{dim}")
}
```

### 21.3 Verify path

```rust
fn verify_catalog_against_bundle(
    catalog: &Catalog,
    bundle: &Bundle,
) -> Result<(), CompatError> {
    if let Some(bfp) = &catalog.bundle_fingerprint {
        if bfp != &bundle.fingerprint().hex() {
            return Err(CompatError::ByteExactMismatch);
        }
        return Ok(());
    }
    if let Some(legacy) = &catalog.legacy_embedder_fingerprint {
        let expected = legacy_fingerprint_from_bundle(bundle);
        if &expected == legacy {
            tracing::warn!(
                "catalog {:?} uses legacy fingerprint {:?} ; \
                 byte-exact pinning recommended (run efe-onboard \
                 upgrade-catalog --tenant {} --catalog-name {})",
                catalog.name, legacy, catalog.tenant, catalog.name
            );
            return Ok(());
        }
        return Err(CompatError::LegacyMismatch);
    }
    Err(CompatError::NoFingerprint)
}
```

### 21.4 Sunset

Quand 100 % des catalogues prod sont sur `bundle_fingerprint`
(metric : `count(catalogs WHERE bundle_fingerprint IS NULL) == 0`),
on peut retirer la colonne `legacy_embedder_fingerprint`
dans une migration `0006_drop_legacy_fingerprint.sql` —
prévu D17.

---

## 22. Plan de sprint D16 (5 semaines, 1 dev)

### Semaine 1 — Foundation

- [ ] `crates/efe-server/Cargo.toml` : deps `ort`, `tokenizers`,
      `ed25519-dalek`, `sha2` (déjà), `serde_json` (déjà)
- [ ] `migrations/0005_extend_embedder_fingerprint.sql`
- [ ] `src/config.rs` : `EmbedderConfig` + invariants
- [ ] `src/embedder/manifest.rs` + JSON schema fixture
- [ ] `src/embedder/signature.rs` + trust store loader
- [ ] `src/embedder/verify.rs` (hash check)
- [ ] Unit tests : 16 listés § 15.1

### Semaine 2 — Loader Rust

- [ ] `src/embedder/bundle.rs` (load_bundle_dir)
- [ ] `src/embedder/runtime/onnx.rs` (ORT adapter)
- [ ] `src/embedder/cache.rs` (singleflight)
- [ ] `src/embedder/mod.rs` (public API)
- [ ] Integration tests : 6 listés § 15.2

### Semaine 3 — Build + distribution

- [ ] `crates/efe-server/src/bin/efe_bundle.rs` CLI
      (pack, keygen, verify, inspect)
- [ ] `scripts/d16/export_to_onnx.py`
- [ ] OCI distribution helper script
- [ ] Reproducibility CI job

### Semaine 4 — Helm + Python adapter

- [ ] `deploy/helm/efe/values.yaml` extensions
- [ ] `deploy/helm/efe/templates/deployment.yaml`
      (initContainer + bundle volume)
- [ ] `clients/python/efe_tool/embedder/bundle.py`
- [ ] `efe_native` PyO3 binding
- [ ] Pin tests Python : 9 listés § 15.3

### Semaine 5 — Observability + docs

- [ ] `crates/efe-telemetry/src/metrics/d16_instruments.rs`
- [ ] Spans D16 dans `efe-telemetry/src/spans.rs`
- [ ] Grafana dashboard `D16-embedder.json`
- [ ] PrometheusRule `d16-alerts.yaml`
- [ ] 3 docs Diátaxis :
      `docs/docs/howto/embedder-bundle.md`,
      `docs/docs/howto/embedder-bundle-rotation.md`,
      `docs/docs/explanation/embedder-bundle-format.md`
- [ ] CHANGELOG D16 entry

### Audit interne (semaine 6)

- [ ] Re-lecture chirurgicale ligne par ligne
- [ ] Threat-model regression tests
- [ ] Fuzz-target sur le manifest parser
- [ ] Performance bench (load p99, encode p99)
- [ ] Rapport `docs/audits/d16_audit_report.md`

---

## 23. Critères d'acceptation D16

D16 est livré quand **tous** les critères ci-dessous sont
satisfaits sur le branch `main` :

- [ ] `cargo test -p efe-server` : ≥ 480 tests verts
      (456 D15 + ≥ 24 D16)
- [ ] `cargo test -p efe-server --test d16_bundle_integration`
      : ≥ 6 tests verts
- [ ] `pytest scripts/tests/` : ≥ 631 tests verts
      (621 D15 + ≥ 10 D16)
- [ ] `efe-bundle pack` deterministic test passes
- [ ] Helm chart renders cleanly (`helm template`)
- [ ] Une démo end-to-end : pack a tiny ONNX bundle, sign,
      mount in a kind cluster, run a `Encode` RPC, verify
      vector dim + L2-norm
- [ ] Signature verification fails closed when key is rotated
      out
- [ ] Performance test : `efe_bundle_load_duration_seconds`
      p99 < 5 s sur le bundle MiniLM
- [ ] Performance test : `efe_embedder_encode_duration_seconds`
      p99 < 50 ms CPU
- [ ] Threat-model regression tests T-16-1 .. T-16-9 verts
- [ ] Cross-sprint regression : 0 tests regress
      (M10 → D15 inclus)
- [ ] CHANGELOG.md a une entrée D16 complète
- [ ] `M12_ARCHITECTURE.md` referencing this doc en § 5.X
- [ ] Audit interne livré : rapport
      `docs/audits/d16_audit_report.md`

---

## 24. Ce que D16 ne fait PAS

Liste honnête des non-objectifs, pour éviter le scope creep :

- **Pas de fine-tuning** des bundles (D17).
- **Pas de hot-reload** (Q-16-1, reporté D16-bis).
- **Pas d'embedder server distribué** (e.g. embedder-as-a-service
  qui shard les requêtes). Mode 1 reste single-pod-loaded.
- **Pas de cross-encoder reranking** (modèle séparé).
- **Pas de quantization automatique** (FP16 / INT8). L'opérateur
  choisit son `dtype` au pack-time ; D16 charge ce qu'on
  lui donne.
- **Pas de prefetch prédictif** des bundles (charger un
  bundle B en background pendant qu'on sert sur A). Charge
  paresseusement at-first-use ou eagerly at-boot selon la
  config.
- **Pas de support OpenAI / Voyage / Cohere API embeddings**.
  Bundles sont local-only. Les embedders cloud sont un
  chemin parallèle (D17 candidate, embedder-cloud-adapter).
- **Pas de sparse / hybrid retrieval** (BM25 + dense). EFE
  reste pure-dense.

---

**Document maître D16** ─ ~1500 lignes, ~14 800 mots. À
relire une fois par sprint Owner + une fois par security
reviewer avant de gel-er la spec et passer à
l'implémentation.

— *Fin du document D16 architecture.*
