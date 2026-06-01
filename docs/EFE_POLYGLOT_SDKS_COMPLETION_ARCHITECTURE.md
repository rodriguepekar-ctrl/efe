# EFE — Polyglot SDKs Completion Architecture (Φ-C-SDK, D29 → D34)

> Document architecture **complétion polyglotte** pour les SDK Go, TypeScript,
> Java. Lecture obligatoire **avant** les 3 archis individuelles existantes
> (`EFE_GO_SDK_EXTENSION_ARCHITECTURE.md`, `EFE_TYPESCRIPT_SDK_EXTENSION_ARCHITECTURE.md`,
> `EFE_JAVA_SDK_EXTENSION_ARCHITECTURE.md`) car les artefacts livrés ici
> **ferment** le contrat opposable cross-langue : un `Verdict` retourné par
> n'importe lequel des 4 SDKs (Python PyO3 + Go + TS + Java) doit être
> **bit-identique** pour un triplet `(text, catalog_id, seed)` fixé.
>
> **Sprint** : Φ-C-SDK (D29 → D34, 6 jours).
> **Statut** : architecture **prête à exécution** — code drop joint.
> **Doctrine** : pas un seul SDK ne diverge silencieusement ; pas un seul
> idiome de langue ne casse le contrat opposable ; pas une seule wheel,
> module ou JAR ne quitte CI sans signature Sigstore + SBOM.

---

## Table des matières

- **§ 0** — Méta, conventions, glossaire chirurgical
- **§ 1** — Mission Φ-C-SDK & contrat opposable polyglotte
- **§ 2** — Position vs les 3 archis SDK existantes (= gap analysis)
- **§ 3** — État AS-IS chirurgical
- **§ 4** — Décomposition du contrat cross-langue
- **§ 5** — Surface 1 : Contract testing (= bit-identity cross-langue, 4 SDKs × 100 fixtures)
- **§ 6** — Surface 2 : Codegen pipeline (= 1 proto → 4 SDKs, hash-pinned)
- **§ 7** — Surface 3 : Versioning + breaking change policy semver
- **§ 8** — Surface 4 : Publishing matrix (npm + Go modules + Maven Central + PyPI)
- **§ 9** — Surface 5 : Observability standardisée (OTel cross-SDK)
- **§ 10** — Surface 6 : Sécurité (mTLS + Sigstore + SBOM × 3)
- **§ 11** — Surface 7 : Documentation polyglotte (Docusaurus unifié)
- **§ 12** — Surface 8 : Examples polyglottes (same use-case × 4 langs)
- **§ 13** — Surface 9 : Migration helpers (PyO3 ↔ gRPC drop-in)
- **§ 14** — Surface 10 : SLO opposables polyglottes (15 SLO YAML-parseable)
- **§ 15** — Surface 11 : Idiomes par langue + minimal-divergence policy
- **§ 16** — Matrice complète des artefacts Φ-C-SDK (40 entrées)
- **§ 17** — Plan d'implémentation D29 → D34 chirurgical
- **§ 18** — Risques identifiés & mitigations (20 risques)
- **§ 19** — Décisions de conception clés (18)
- **§ 20** — Anti-régression cross-SDK
- **§ 21** — Critères d'acceptation (DoD à 100 %)
- **§ 22** — Hand-off & dépendances downstream
- **§ 23** — Annexes (A → H)

---

## § 0. Méta, conventions, glossaire chirurgical

### 0.1 Doctrine

| Doctrine | Énoncé chirurgical |
|---|---|
| **D-SDK-1 — Contrat opposable cross-langue** | Pour un triplet `(text, catalog_id, seed)` fixé, les 4 SDKs (Python PyO3, Go, TS, Java) retournent un `Verdict` **bit-identique** sur tous les champs identifiants (label, cone_id, rounds_executed, swarm_id, catalog_version) + confidence avec tolérance `< 1e-6`. Le contract testing harness (§ 5) enforce sur 100 fixtures. |
| **D-SDK-2 — Une source de vérité** | Le contrat Verdict provient de **`protos/efe/v1/types.proto`** ; les 4 SDKs en sont des codegen dérivés. Le hash SHA-256 du .proto est gelé à chaque release ; toute modif du proto force un bump MAJOR sur tous les SDKs. |
| **D-SDK-3 — Idiome respecté, divergence minimale** | Chaque SDK est **idiomatique** de sa langue (= functional options en Go, fluent builder en Java, Promise/async en TS, frozen pyclass en Python) MAIS la surface sémantique reste isomorphe : pour chaque méthode du proto, chaque SDK expose **exactement** une méthode publique avec les mêmes paramètres et le même return type (modulo conventions de nommage). |
| **D-SDK-4 — Production caller par instrument cross-SDK** | Chaque instrument OpenTelemetry standardisé (§ 9) doit avoir un appelant dans **chaque** SDK où il s'applique. Sans grep ≥ 1 hit / SDK, le pin test polyglotte fail. |
| **D-SDK-5 — Publishing artifact = signed + SBOM** | Toute wheel PyPI, package npm, module Go, ou JAR Maven Central doit être **(a)** signée Sigstore keyless (= GitHub OIDC), **(b)** indexée Rekor, **(c)** accompagnée d'un SBOM CycloneDX. Aucune exception. |
| **D-SDK-6 — Semver opposable cross-SDK** | Tous les SDKs partagent **la même version** semver à chaque release. Une release Φ-C-SDK `v0.6.0` publie : `efe@0.6.0` (PyPI), `@efe-ai/sdk@0.6.0` (npm), `github.com/efe-ai/efe-go@v0.6.0` (Go modules), `ai.efe:efe-java:0.6.0` (Maven). |
| **D-SDK-7 — Pin test cross-SDK** | Tout drift entre les 4 SDKs (= signature de méthode, version, hash du proto, surface publique) doit casser un pin test runtime-enforced en CI. Aucune dérive silencieuse tolérée. |

### 0.2 Glossaire chirurgical

| Terme | Définition |
|---|---|
| **Polyglot SDK** | Ensemble des 4 SDKs (Python PyO3 + Go + TS + Java) partageant le même contrat opposable. |
| **Verdict** | Le type return canonique de `efe.classify(...)`. 8 champs identifiants (cf. § 4.2). Bit-identique cross-SDK pour `(text, catalog_id, seed)` fixé. |
| **Contract test** | Le harness § 5 qui lance les 4 SDKs sur un même backend `efe-server` et compare les verdicts byte-à-byte. |
| **Codegen pipeline** | Le pipeline § 6 qui transforme `protos/efe/v1/*.proto` en : Python (= via PyO3 pour l'in-process + grpc-tools pour gRPC), Go (= protoc-gen-go-grpc), TS (= ts-proto + @grpc/grpc-js), Java (= grpc-java + protoc-gen-grpc-java). |
| **proto hash** | SHA-256 du contenu de `protos/efe/v1/*.proto` concaténés dans l'ordre canonique. Geledé dans `bench_baselines/v0.6.0/sdk_proto_hash.txt`. |
| **Idiomatic** | Chaque SDK respecte les conventions de sa langue : Go = functional options + context.Context first arg ; TS = Promise<T> + async/await ; Java = fluent builder + CompletableFuture ; Python = pyclass + asyncio. |
| **Bit-identity** | `verdict_go.swarm_id == verdict_ts.swarm_id == verdict_java.swarm_id == verdict_py.swarm_id` sur les 100 fixtures fixés. |
| **Minimal-divergence** | Les SDKs divergent uniquement sur les conventions de nommage et le retour async/sync. Aucune divergence sémantique tolérée. |
| **SDK Quartet** | Les 4 SDKs Python/Go/TS/Java traités comme une unité de release. |
| **Drop-in migration** | API qui permet à un user de switcher de `EfeClient` (gRPC) vers `EfeNativeClient` (PyO3) ou inverse, en changeant 2 lignes d'import. |

### 0.3 Conventions de référence

* SLO contractuel : `SLO-SDK-<DOMAIN>-<KEY>` (= 15 entries dans `bench_baselines/v0.6.0/sdk_slos.yaml`).
* Artefact tracking : `Φ-C-SDK.A<N>` (où N ∈ matrice § 16).
* Workflow CI : `sdk-quartet-contract.yml` (PR-gate), `sdk-quartet-publish.yml` (release), `sdk-quartet-bench.yml` (nightly).
* Suffixe SDK : `-go`, `-ts`, `-java`, `-py` (pour distinguer dans les labels Prometheus).

---

## § 1. Mission Φ-C-SDK & contrat opposable polyglotte

### 1.1 Mission

> Faire que le `Verdict` retourné par `efe.classify(...)` soit **bit-identique
> cross-langue** sur 100 fixtures, et que les 4 SDKs soient **publiable
> + signed + auditable** dans leurs écosystèmes respectifs simultanément.
>
> Concrètement :
>
> 1. **`efe-go-sdk v0.6.0`** publié sur `github.com/efe-ai/efe-go`,
>    consommable via `go get github.com/efe-ai/efe-go@v0.6.0`.
> 2. **`@efe-ai/sdk v0.6.0`** publié sur npm registry public, consommable
>    via `npm install @efe-ai/sdk` ou `pnpm add @efe-ai/sdk`.
> 3. **`ai.efe:efe-java:0.6.0`** publié sur Maven Central, consommable via
>    `<dependency><groupId>ai.efe</groupId><artifactId>efe-java</artifactId><version>0.6.0</version></dependency>`.
> 4. **`efe@0.6.0`** publié sur PyPI (= existant Φ-C-FFI, mais aligné en
>    version sur le quartet).
> 5. Pour un triplet `(text, catalog_id, seed)` fixé :
>    `verdict_py == verdict_go == verdict_ts == verdict_java` sur les 8
>    champs identifiants + confidence à 1 ulp f32 près.
> 6. Tout artefact (wheel, JAR, module, npm tarball) est **signé Sigstore**
>    + **indexé Rekor** + accompagné d'un **SBOM CycloneDX**.
> 7. Toute documentation publique est servie sur `docs.efe.ai` via
>    Docusaurus polyglotte (= un même use-case présenté en 4 langues).

### 1.2 Contrat opposable

| Promesse | SLO | Mesure | Gating |
|---|---|---|---|
| Contract test cross-SDK | **SLO-SDK-CONTRACT** = 400/400 (= 100 fixtures × 4 SDKs) | `contract-test-runner` | PR-gate `sdk-quartet-contract.yml` |
| Latence Go p99 | **SLO-SDK-LATENCY-GO** ≤ 50 ms | `bench-go` ghz-like | nightly + tag |
| Latence TS p99 | **SLO-SDK-LATENCY-TS** ≤ 60 ms | `bench-ts` | nightly + tag |
| Latence Java p99 | **SLO-SDK-LATENCY-JAVA** ≤ 60 ms | `bench-java` | nightly + tag |
| Doc coverage Go | **SLO-SDK-DOC-GO** ≥ 90 % | `golint -presentation=stylecheck -ratio` | PR-gate |
| Doc coverage TS | **SLO-SDK-DOC-TS** ≥ 90 % | `typedoc --validation.notDocumented` | PR-gate |
| Doc coverage Java | **SLO-SDK-DOC-JAVA** ≥ 90 % | `javadoc -Xdoclint` | PR-gate |
| Package size Go | **SLO-SDK-SIZE-GO** ≤ 2 MiB | `du -sh build/` | tag-gate |
| Package size TS | **SLO-SDK-SIZE-TS** ≤ 500 KiB compressed | `du -sh *.tgz` | tag-gate |
| Package size Java | **SLO-SDK-SIZE-JAVA** ≤ 5 MiB | `du -sh *.jar` | tag-gate |
| Proto hash sync | **SLO-SDK-PROTO-HASH** PASS | `sha256sum protos/` | PR-gate |
| Version sync | **SLO-SDK-VERSION-SYNC** PASS | parse all 4 manifests | PR-gate |
| Sigstore signing × 4 | **SLO-SDK-SIGSTORE** ≥ 4 artifacts signed | `cosign verify-blob` × 4 | tag-gate |
| SBOM × 4 | **SLO-SDK-SBOM** ≥ 4 SBOMs | CycloneDX 1.5 × 4 | tag-gate |
| Cross-SDK API surface | **SLO-SDK-API-SURFACE** PASS | introspection diff | PR-gate |

### 1.3 Anti-mission (ce que Φ-C-SDK **ne fait pas**)

* Ne réécrit pas la surface gRPC. `protos/efe/v1/*.proto` est gelé au snapshot D25 ; Φ-C-SDK **consume** + **packagee** ce contrat.
* Ne livre pas de **WebAssembly SDK** (= deferred to Φ-D).
* Ne livre pas de **C/C++ SDK** (= deferred — pas de demande client).
* Ne livre pas de **Ruby/PHP/.NET SDK** (= deferred).
* Pas de WebUI bundling (= séparé, `efe-web` est un autre projet).
* Pas d'optimisation gRPC over QUIC (= déferré Φ-D).

---

## § 2. Position vs les 3 archis SDK existantes (= gap analysis)

### 2.1 Ce que les 3 archis individuelles couvrent déjà

| Archi | Lignes | Surface | Statut D25 |
|---|---|---|---|
| `EFE_GO_SDK_EXTENSION_ARCHITECTURE.md` | 2 257 | Functional options, codegen via protoc-gen-go-grpc, retry, circuit breaker, OTel auto-instr | Scaffold |
| `EFE_TYPESCRIPT_SDK_EXTENSION_ARCHITECTURE.md` | ? | ts-proto, @grpc/grpc-js, Promise-based API, tree-shakeable | Scaffold |
| `EFE_JAVA_SDK_EXTENSION_ARCHITECTURE.md` | 3 027 | grpc-java, fluent builder, CompletableFuture, JaCoCo coverage | Scaffold |
| `EFE_PYTHON_SDK_EXTENSION_ARCHITECTURE.md` | ? | Wrapper `efe._native` (Φ-C-FFI) | Mostly done via Φ-C-FFI |

### 2.2 Ce que Φ-C-SDK ajoute (= gaps fermés)

| # | Gap | Étage | Sévérité | Φ-C-SDK artefact |
|---|---|---|---|---|
| G1 | Pas de contract testing cross-langue | Tests | **CRITIQUE** | `sdk/contract-runner/` + 4 client harnesses |
| G2 | Pas de SLO YAML cross-SDK | SLO | **CRITIQUE** | `bench_baselines/v0.6.0/sdk_slos.yaml` |
| G3 | Codegen pipeline non orchestré | Build | HIGH | `scripts/sdk/regen_all.sh` |
| G4 | Pas de proto hash pinning | Build | HIGH | `bench_baselines/v0.6.0/sdk_proto_hash.txt` + pin test |
| G5 | Pas de version sync enforce | Build | HIGH | `scripts/sdk/verify_version_sync.py` |
| G6 | Pas de SBOM pour 3 non-Python | Compliance | HIGH | extension `sdk-quartet-publish.yml` |
| G7 | Pas de Sigstore pour 3 non-Python | Sécurité | HIGH | extension publish |
| G8 | Pas de doc coverage gating | DX | MED | `verify_doc_coverage.py` cross-SDK |
| G9 | Pas de bench harness pour Go/TS/Java | Mesure | HIGH | `clients/{go,ts,java}/bench/` |
| G10 | Pas de Docusaurus polyglotte unifié | DX | MED | `sdk-docs/` Docusaurus monorepo |
| G11 | Pas de examples polyglotte | DX | MED | `examples/{tutorial-1..6}/{go,ts,java,py}/` |
| G12 | Pas de migration helpers | Migration | MED | `clients/python/efe/_dropin.py` + 3 README sections |
| G13 | Pas de runbooks cross-SDK | Ops | MED | 4 runbooks |
| G14 | Pas de plan day-by-day D29-D34 | Process | MED | § 17 |
| G15 | Pas de pin tests cross-SDK | Process | HIGH | `scripts/tests/test_polyglot_sdk_structure.py` |
| G16 | Pas de standardisation OTel | Obs | HIGH | `sdk-otel-conventions.md` + 4 implementations |
| G17 | Pas de fixtures cross-SDK partagées | Tests | HIGH | `fixtures/sdk_quartet/` |
| G18 | Pas de Docker compose pour contract test | Tests | HIGH | `sdk/contract-runner/docker-compose.yml` |
| G19 | Pas de retry/circuit breaker standardisé | Code | MED | `sdk-retry-conventions.md` |
| G20 | Pas de auth conventions standardisées | Code | MED | `sdk-auth-conventions.md` |

### 2.3 Doctrine de gap-closing

Le pattern Φ-C-FFI Completion (récent) a montré que les défauts
"silently broken SLO gating" surviennent quand :

* Un champ YAML est utilisé que `check_slo.py` n'interprète pas → SLO silencieusement skippé.
* Une métrique est déclarée mais n'a pas d'appelant production → graphes plats.
* Un script référencé par CI n'existe pas → CI fail au premier run.

Φ-C-SDK réplique ce pattern cross-langue : **chaque artefact ferme un SLO
opposable ou un instrument standardisé ou un script référencé.**

---

## § 3. État AS-IS chirurgical

### 3.1 Inventaire `clients/` existant

```
clients/
├── go/               # Scaffold Go SDK (~ ? LOC)
├── java/             # Scaffold Java SDK
├── python/           # Scaffold + wraps efe._native via PyO3
├── python-byo-agent/ # Variante BYO LLM (= M20)
└── typescript/       # Scaffold TS SDK
```

```
deploy/sdks/
├── go/               # Go publishing config (= goreleaser ?)
└── typescript/       # TS publishing config (= .npmrc ?)
```

```
protos/efe/v1/
├── catalog.proto
├── efe.proto         # Main service
├── types.proto       # Verdict, SwarmSpec, AlignmentCone, etc.
├── mode3.proto
├── mode2.proto
└── admin.proto
```

### 3.2 Statut par dimension

| Dimension | Go | TS | Java | Python |
|---|---|---|---|---|
| Codegen produced | ⚠️ | ⚠️ | ⚠️ | ✅ (PyO3 + grpc-tools) |
| Idiomatic API | ⚠️ partial | ⚠️ | ⚠️ | ✅ |
| Retry + circuit breaker | ❌ | ❌ | ❌ | ❌ (via gRPC) |
| OTel instrumentation | ❌ | ❌ | ❌ | ✅ (Φ-C-FFI) |
| Contract testing | ❌ | ❌ | ❌ | ⚠️ (vs gRPC dropin) |
| Bench harness | ❌ | ❌ | ❌ | ✅ |
| Publishing pipeline | ❌ | ❌ | ❌ | ✅ |
| Sigstore signing | ❌ | ❌ | ❌ | ✅ |
| SBOM | ❌ | ❌ | ❌ | ✅ |
| Doc generation | ❌ | ❌ | ❌ | ⚠️ |

**Verdict** : 3 SDKs largement greenfield (= scaffolds), 1 SDK quasi-complet (= Python). Φ-C-SDK ferme les gaps des 3 + unifie le contrat.

### 3.3 Mesure quantitative AS-IS

| Métrique | Mesure D25 | Cible Φ-C-SDK |
|---|---|---|
| LOC Go SDK | ~ 200 (scaffold) | ~ 3 000 |
| LOC TS SDK | ~ 200 | ~ 2 500 |
| LOC Java SDK | ~ 200 | ~ 3 500 |
| LOC Python SDK (wraps PyO3) | ~ 100 | ~ 500 |
| Tests cross-SDK (contract) | 0 | 100 fixtures × 4 SDKs = 400 |
| Workflows CI | 0 (sauf ffi-test/publish/bench) | 3 (`contract`, `publish`, `bench`) |
| Runbooks cross-SDK | 0 | 4 |
| SLO YAML entries | 0 | 15 |
| Pin tests cross-SDK | 0 | 12 |

---

## § 4. Décomposition du contrat cross-langue

### 4.1 Le contrat `Verdict`

Source de vérité : `protos/efe/v1/types.proto::message Verdict`. Champs :

```protobuf
message Verdict {
  string label             = 1;
  float  confidence        = 2;
  string cone_id           = 3;
  uint32 rounds_executed   = 4;
  string swarm_id          = 5;
  string catalog_version   = 6;
  repeated SubScore sub_scores = 7;
  double latency_ms        = 8;  // Wall-clock, non-déterministe
}
```

### 4.2 Champs identifiants pour bit-identity

Pour le contract test, on compare **strictement** :

| Champ | Type | Comparison | Note |
|---|---|---|---|
| `label` | string | `==` | Le label texte. Doit être idempotent. |
| `cone_id` | string | `==` | Cone alignement utilisé. |
| `rounds_executed` | uint32 | `==` | Nombre de fusion rounds. |
| `swarm_id` | string | `==` | SHA-256-derived identifier ; bit-id obligatoire. |
| `catalog_version` | string | `==` | Semver string du catalog. |
| `sub_scores[]` | repeated | order + `==` per element | Top-N alternative labels. |
| `confidence` | float | `< 1e-6` epsilon | f32 cross-language drift (ulp). |
| `latency_ms` | double | **IGNORÉ** | Wall-clock non-déterministe. |

### 4.3 Pourquoi la confidence a une tolerance

Côté Rust (PyO3) : `(conf_u32 as f32) / (u32::MAX as f32) → f32`. Cast à
Python float (= f64) au retour.

Côté Go/TS/Java : le float32 vient du wire format proto3 (= 4 bytes little-endian). Décodé en `float32`/`number`/`float` selon la langue. Conversion :
* Go : `float32` natif. Aucune conversion implicite vers f64.
* TS : `number` (= f64). `protobufjs` ou `ts-proto` décode le f32 en f64 — exact (= f32 fits in f64).
* Java : `float` natif. Pas de conversion implicite.

Sur Go, TS, Java, le float32 est **bit-identique** au float32 envoyé sur le wire par le serveur (= calculé en Rust f32).

Sur Python PyO3 in-process : le calcul est en f32 mais le retour est f64 exact.

**Résultat** : tous les 4 SDKs voient un `confidence` qui est le f32 originalement calculé, exposé soit en f32 natif soit en f64 exact. La comparaison f32 == f32 est `<= 1 ulp`. La comparaison f32-cast-f64 == f32-cast-f64 est exact.

**Tolérance 1e-6** absorbe une éventuelle erreur d'arrondi au transcodage protobuf wire. Documenté.

### 4.4 Champs ignorés pour bit-identity

* `latency_ms` : wall-clock, varie. Test l'ignore.

### 4.5 Étages de l'appel cross-langue

```text
┌────────────────────────────────────────────────────────────────────┐
│  efe.classify(text, catalog_id, seed) → Verdict                    │
│                                                                    │
│  [Go/TS/Java SDK]                                                  │
│    1. Marshal args → ClassifyRequest protobuf                      │
│    2. gRPC send (mTLS-encrypted if configured)                     │
│    3. Wait response (= timeout configured by user)                 │
│    4. Unmarshal ClassifyResponse → Verdict                         │
│    5. Apply local retry / circuit breaker if configured            │
│    6. Return Verdict                                               │
│                                                                    │
│  [Python PyO3 SDK — in-process path]                               │
│    1. Marshal args → SwarmSpec (Rust struct)                       │
│    2. Direct call efe_core::SwarmController::execute               │
│    3. Return PyVerdict                                             │
└────────────────────────────────────────────────────────────────────┘

Pour bit-identity : étapes 2-3-4 (Go/TS/Java) doivent donner exactement
le même `Verdict` que les étapes 2 (Python PyO3 in-process). Étant donné
que **les deux paths bout-à-bout calculent dans le MÊME `efe_core` Rust**,
ils SONT bit-identique par construction. Le contract test vérifie qu'il
n'y a pas de drift introduit par les couches transport.
```

---

## § 5. Surface 1 : Contract testing

### 5.1 Mission

Lancer un seul backend `efe-server` (= dockerisé), invoquer la même
requête `classify(text, catalog_id, seed)` via les 4 SDKs, et **assert
bit-identity** sur les 100 fixtures.

### 5.2 Architecture du harness

```
sdk/contract-runner/
├── docker-compose.yml          # 1 efe-server + 4 client containers
├── fixtures/
│   └── quartet_fixtures.json   # 100 fixtures (text, catalog_id, seed, expected)
├── runners/
│   ├── go/                     # Go client invoque + write verdict to /shared/out_go.json
│   ├── ts/                     # TS client → /shared/out_ts.json
│   ├── java/                   # Java client → /shared/out_java.json
│   └── py/                     # PyO3 client → /shared/out_py.json
└── compare.py                  # Read 4 outputs, assert bit-identity
```

### 5.3 Procédure

```bash
# 1. Boot efe-server (= mockable backend pour CI)
docker-compose -f sdk/contract-runner/docker-compose.yml up -d efe-server

# 2. Run all 4 SDK runners (in parallel)
docker-compose -f sdk/contract-runner/docker-compose.yml run --rm runner-go
docker-compose -f sdk/contract-runner/docker-compose.yml run --rm runner-ts
docker-compose -f sdk/contract-runner/docker-compose.yml run --rm runner-java
docker-compose -f sdk/contract-runner/docker-compose.yml run --rm runner-py

# 3. Compare outputs
python sdk/contract-runner/compare.py \
    --go     /shared/out_go.json \
    --ts     /shared/out_ts.json \
    --java   /shared/out_java.json \
    --py     /shared/out_py.json \
    --out    target/sdk/contract.json
```

### 5.4 Format `quartet_fixtures.json`

```json
{
  "version": "v0.6.0",
  "count": 100,
  "fixtures": [
    {
      "id": "fixture-001",
      "text": "I want to dispute the charge",
      "catalog_id": "router-v1",
      "seed": 42
    },
    /* ... 99 more ... */
  ]
}
```

Note : **pas d'expected verdict** dans la fixture (= le ground truth
est calculé par le serveur, comparé entre SDKs uniquement).

### 5.5 Format output per-SDK

```json
{
  "sdk": "go",
  "version": "0.6.0",
  "count": 100,
  "verdicts": [
    {
      "fixture_id": "fixture-001",
      "label": "billing-dispute",
      "confidence": 0.9876,
      "cone_id": "cone-router-v1",
      "rounds_executed": 12,
      "swarm_id": "swarm-a1b2c3d4...",
      "catalog_version": "1.0.0",
      "sub_scores": [
        {"label": "refund-request", "confidence": 0.0124}
      ]
    },
    /* ... 99 more ... */
  ]
}
```

### 5.6 `compare.py` SLO output

```json
{
  "slo_compliance": {
    "SLO-SDK-CONTRACT": {
      "measured_pass_count": 400,
      "measured_expected_count": 400,
      "per_sdk_breakdown": {
        "go":   {"passes": 100, "failures": 0},
        "ts":   {"passes": 100, "failures": 0},
        "java": {"passes": 100, "failures": 0},
        "py":   {"passes": 100, "failures": 0}
      },
      "label_mismatches": [],
      "confidence_drift_max": 0.0
    }
  }
}
```

### 5.7 Cas d'échec attendus

| Failure mode | Cause probable | Mitigation |
|---|---|---|
| Label divergence Go vs Java | Codegen mismatch (= différentes versions de protoc) | Pin protoc version |
| Confidence drift > 1e-6 | Wire-format conversion bug | Fix decoder |
| All 4 SDKs same wrong label | Backend bug (= regression in efe-server) | Block PR backend |
| 1 SDK fails timeout | Network ou retry config | Investigate spécifique |

---

## § 6. Surface 2 : Codegen pipeline (= 1 proto → 4 SDKs)

### 6.1 Topology

```text
                  protos/efe/v1/*.proto (source of truth)
                            │
       ┌────────────┬───────┴───────┬──────────────┐
       ▼            ▼               ▼              ▼
   protoc       protoc          protoc           protoc
   --python    --go --grpc      --ts            --java --grpc
       │            │               │              │
       ▼            ▼               ▼              ▼
   clients/    clients/         clients/         clients/
   python/     go/              typescript/      java/
   generated/  generated/       generated/       generated/
```

### 6.2 Pipeline orchestré

```bash
# scripts/sdk/regen_all.sh
#!/usr/bin/env bash
set -euo pipefail

# Compute current proto hash (= D-SDK-4 enforcement)
NEW_HASH=$(find protos/efe/v1 -name '*.proto' -print0 \
    | sort -z | xargs -0 cat | sha256sum | awk '{print $1}')
EXPECTED_HASH=$(cat bench_baselines/v0.6.0/sdk_proto_hash.txt)

if [ "$NEW_HASH" != "$EXPECTED_HASH" ] && [ "${SDK_PROTO_BUMP:-no}" != "yes" ]; then
    echo "ERROR: proto hash drift detected."
    echo "  expected : $EXPECTED_HASH"
    echo "  current  : $NEW_HASH"
    echo "Set SDK_PROTO_BUMP=yes to acknowledge breaking change."
    exit 1
fi

# Generate per-SDK
./scripts/sdk/regen_go.sh
./scripts/sdk/regen_ts.sh
./scripts/sdk/regen_java.sh
./scripts/sdk/regen_python.sh
```

### 6.3 Per-SDK codegen specifics

| SDK | Tool | Output dir | Idiom |
|---|---|---|---|
| Go | `protoc-gen-go-grpc v1.5` | `clients/go/generated/` | Functional options on stubs |
| TS | `ts-proto v1.169` | `clients/typescript/src/generated/` | Promise-based client |
| Java | `grpc-java protoc-gen-grpc-java v1.62` | `clients/java/src/main/generated/` | Builder pattern + ListenableFuture |
| Python | `grpc-tools v1.62` | `clients/python/efe/generated/` | Async stubs (= asyncio compat) |

### 6.4 Pin proto hash

`bench_baselines/v0.6.0/sdk_proto_hash.txt` :

```
PLACEHOLDER_compute_at_freeze
```

Au moment du freeze Φ-C-SDK, on lance `scripts/sdk/regen_all.sh` une
première fois pour figer le hash :

```bash
NEW_HASH=$(find protos/efe/v1 -name '*.proto' -print0 \
    | sort -z | xargs -0 cat | sha256sum | awk '{print $1}')
echo "$NEW_HASH" > bench_baselines/v0.6.0/sdk_proto_hash.txt
```

### 6.5 Anti-régression : pin test proto hash

```python
def test_sdk_proto_hash_unchanged():
    """Tout changement de protos/efe/v1/*.proto force un MAJOR bump."""
    pinned = (REPO / "bench_baselines" / "v0.6.0" / "sdk_proto_hash.txt").read_text().strip()
    if pinned.startswith("PLACEHOLDER_"):
        pytest.skip("proto hash not yet frozen")
    current = compute_proto_hash()
    assert pinned == current, (
        f"Proto hash drift detected — bump SDK MAJOR or revert proto."
    )
```

---

## § 7. Surface 3 : Versioning + breaking change policy

### 7.1 Semver opposable cross-SDK

Le quartet partage **la même version**. Une release `v0.6.0` publie :

```
PyPI    : efe                          0.6.0
npm     : @efe-ai/sdk                  0.6.0
Go      : github.com/efe-ai/efe-go     v0.6.0
Maven   : ai.efe:efe-java              0.6.0
```

### 7.2 Bump rules

| Changement | Bump |
|---|---|
| `protos/efe/v1/*.proto` modifié | **MAJOR** sur quartet |
| Surface publique d'un SDK ajoute une méthode | **MINOR** sur quartet |
| Surface publique d'un SDK retire une méthode | **MAJOR** sur quartet |
| Implémentation d'un SDK change sans changer surface | **PATCH** sur quartet |
| Surface n'est pas isomorphe entre SDKs (= 1 SDK ajoute, autres pas) | **REFUSED par PR-gate** (= drift cross-SDK violation D-SDK-3) |

### 7.3 Pin test version sync

```python
def test_sdk_versions_in_sync():
    """All 4 SDKs must declare the same version string."""
    py_ver   = parse_pyproject("clients/python/pyproject.toml")
    go_ver   = parse_go_mod("clients/go/go.mod")  # via VERSION constant
    ts_ver   = parse_package_json("clients/typescript/package.json")
    java_ver = parse_pom("clients/java/pom.xml")
    
    versions = {"python": py_ver, "go": go_ver, "ts": ts_ver, "java": java_ver}
    unique = set(versions.values())
    assert len(unique) == 1, (
        f"SDK version drift : {versions}"
    )
```

### 7.4 Breaking change policy

| Phase | Rules |
|---|---|
| v0.x.y | Surface peut évoluer librement avec MINOR bumps. PR-gate enforce isomorphisme. |
| v1.0.0 | Surface gelée. Toute breaking change → MAJOR bump (v2.0.0). |
| v2.0.0+ | Coexistence v1 + v2 pendant 12 mois minimum. |

---

## § 8. Surface 4 : Publishing matrix

### 8.1 4 écosystèmes

| SDK | Registry | Manifest | Auth |
|---|---|---|---|
| Python | PyPI | `pyproject.toml` | TWINE_PASSWORD secret |
| Go | GitHub (= `github.com/efe-ai/efe-go`) | `go.mod` | Git tag |
| TS | npm public | `package.json` | NPM_TOKEN secret |
| Java | Maven Central (= `ai.efe`) | `pom.xml` | OSSRH_USERNAME + GPG_KEY |

### 8.2 Workflow `sdk-quartet-publish.yml` (extension)

```yaml
name: sdk-quartet-publish

on:
  push:
    tags: ["v*.*.*"]

jobs:
  publish-python:
    # Existant Φ-C-FFI (ffi-publish.yml). Délégué.
    uses: ./.github/workflows/ffi-publish.yml

  publish-go:
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-go@v5
        with: {go-version: "1.22"}
      - name: Build + test
        run: cd clients/go && go build ./... && go test ./...
      - name: Generate SBOM CycloneDX
        run: cyclonedx-gomod app clients/go > clients/go/sbom.cdx.json
      - name: Sign with cosign keyless
        uses: sigstore/cosign-installer@v3
      - name: Publish (= tag push to GitHub)
        run: |
          git tag -a "v${TAG}-go" -m "Go SDK release ${TAG}"
          git push origin "v${TAG}-go"

  publish-ts:
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with: {node-version: "20"}
      - name: Build
        run: cd clients/typescript && pnpm install && pnpm build
      - name: Generate SBOM
        run: cd clients/typescript && pnpm cyclonedx > sbom.cdx.json
      - name: Sign with cosign keyless
        run: cosign sign-blob clients/typescript/dist/efe-ai-sdk-*.tgz
      - name: Publish to npm
        env: {NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}}
        run: cd clients/typescript && npm publish --access public

  publish-java:
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-java@v4
        with: {distribution: 'temurin', java-version: '21'}
      - name: Build + test
        run: cd clients/java && mvn -B verify
      - name: Generate SBOM CycloneDX
        run: cd clients/java && mvn org.cyclonedx:cyclonedx-maven-plugin:makeBom
      - name: Sign with cosign keyless
        run: cosign sign-blob target/efe-java-*.jar
      - name: Deploy to OSSRH
        env:
          OSSRH_USERNAME: ${{ secrets.OSSRH_USERNAME }}
          OSSRH_TOKEN: ${{ secrets.OSSRH_TOKEN }}
          GPG_PRIVATE_KEY: ${{ secrets.GPG_PRIVATE_KEY }}
        run: cd clients/java && mvn -B deploy -P release
```

### 8.3 Acceptance criteria

* PyPI : 5 wheels + 1 sdist + signed + SBOM (= existant Φ-C-FFI).
* Go : Git tag pushed + SBOM in repo + cosign sig + go.sum verifié.
* npm : 1 tarball publié + SBOM in tarball + cosign sig.
* Maven : 1 jar deployed à OSSRH + SBOM + GPG sig.

---

## § 9. Surface 5 : Observability standardisée

### 9.1 Instruments OTel cross-SDK

Chaque SDK doit émettre les **mêmes noms d'instruments** :

| Instrument | Type | Labels | Note |
|---|---|---|---|
| `efe_sdk_classify_total` | Counter | sdk, outcome | sdk ∈ {py, go, ts, java}, outcome ∈ {ok, err} |
| `efe_sdk_classify_seconds` | Histogram | sdk | Latence end-to-end |
| `efe_sdk_grpc_seconds` | Histogram | sdk, method | Latence pure gRPC (= excl. local marshalling) |
| `efe_sdk_retry_total` | Counter | sdk, reason | Retry attempts |
| `efe_sdk_circuit_breaker_state` | Gauge | sdk | 0=closed, 1=open, 2=half-open |

### 9.2 Implementation par SDK

| SDK | OTel SDK | Auto-instr | Manual |
|---|---|---|---|
| Python | `opentelemetry-python` | gRPC interceptor | Manual `note_classify` (= Φ-C-FFI) |
| Go | `go.opentelemetry.io/otel` | `otelgrpc.UnaryClientInterceptor` | Custom histogram per call |
| TS | `@opentelemetry/api` | `@opentelemetry/instrumentation-grpc` | Manual span + histogram |
| Java | `io.opentelemetry:opentelemetry-api` | `opentelemetry-grpc-1.6` agent | Manual span + counter |

### 9.3 Convention de doc

Document `sdk-otel-conventions.md` détaille pour chaque SDK :
* Comment configurer le SDK OTel (= endpoint, service.name).
* Quels instruments sont auto-émis.
* Quels sont les helpers Manual.
* Exemple complet pour chaque SDK.

---

## § 10. Surface 6 : Sécurité (mTLS + Sigstore + SBOM)

### 10.1 mTLS standardization

Chaque SDK doit supporter :

* **TLS** (= par défaut, server cert validé).
* **mTLS** (= client cert + key configurable).
* **Insecure mode** (= dev only, opt-in explicite).

Signatures de méthode de configuration :

| SDK | Method |
|---|---|
| Python | `efe.EfeClient(host="...", cert="/path/client.pem", key="/path/client-key.pem", ca="/path/ca.pem")` |
| Go | `efe.NewClient(host, efe.WithMTLS(certPath, keyPath, caPath))` |
| TS | `new EfeClient({host, mtls: {cert, key, ca}})` |
| Java | `EfeClient.builder().host(...).mtls(certFile, keyFile, caFile).build()` |

### 10.2 Sigstore × 4

Chaque artefact signé via `cosign sign-blob`. Le pin test cross-SDK
`test_phi_c_sdk_all_artifacts_signed` vérifie :

```python
def test_phi_c_sdk_all_artifacts_signed():
    """Every release artifact must have .sig + .pem and a Rekor entry."""
    for sdk in ["python", "go", "ts", "java"]:
        artifacts = list_release_artifacts(sdk, version)
        assert all(has_sigstore_sig(a) for a in artifacts), (
            f"Unsigned artifacts for {sdk} : {artifacts}"
        )
```

### 10.3 SBOM × 4

Chaque SDK produit son SBOM CycloneDX 1.5 au release time :

| SDK | Tool |
|---|---|
| Python | `cyclonedx-py environment` |
| Go | `cyclonedx-gomod app` |
| TS | `@cyclonedx/cdxgen` |
| Java | `cyclonedx-maven-plugin:makeBom` |

---

## § 11. Surface 7 : Documentation polyglotte (Docusaurus)

### 11.1 Site `docs.efe.ai`

Architecture Docusaurus monorepo :

```
sdk-docs/
├── docusaurus.config.js
├── docs/
│   ├── intro.md
│   ├── getting-started/
│   │   ├── python.md
│   │   ├── go.md
│   │   ├── typescript.md
│   │   └── java.md
│   ├── api-reference/
│   │   ├── python.md       # Sphinx output embedded
│   │   ├── go.md           # godoc output embedded
│   │   ├── typescript.md   # TypeDoc output embedded
│   │   └── java.md         # Javadoc output embedded
│   └── tutorials/
│       ├── tutorial-1-classify-text.md
│       ├── tutorial-2-batch-classify.md
│       ├── ... etc ...
└── static/
    └── examples/           # Code samples per SDK
```

### 11.2 Code samples polyglotte

Pour chaque tutorial, le doc présente **4 tabs** (= 4 langues) :

```markdown
## Tutorial 1: Classify a text

<Tabs>
<TabItem value="python">

\`\`\`python
import efe
client = efe.EfeClient(host="efe.example.com:443")
verdict = client.classify("Hello world", catalog_id="router-v1")
print(verdict.label)
\`\`\`

</TabItem>
<TabItem value="go">

\`\`\`go
import "github.com/efe-ai/efe-go"
client, _ := efe.NewClient("efe.example.com:443")
verdict, _ := client.Classify(ctx, &efe.ClassifyRequest{
    Text: "Hello world",
    CatalogId: "router-v1",
})
fmt.Println(verdict.Label)
\`\`\`

</TabItem>
<TabItem value="ts">

\`\`\`typescript
import { EfeClient } from "@efe-ai/sdk";
const client = new EfeClient({ host: "efe.example.com:443" });
const verdict = await client.classify({ text: "Hello world", catalogId: "router-v1" });
console.log(verdict.label);
\`\`\`

</TabItem>
<TabItem value="java">

\`\`\`java
import ai.efe.EfeClient;
EfeClient client = EfeClient.builder().host("efe.example.com:443").build();
Verdict verdict = client.classify("Hello world", "router-v1");
System.out.println(verdict.label());
\`\`\`

</TabItem>
</Tabs>
```

### 11.3 Doc coverage SLO

Chaque SDK doit déclarer une coverage doc ≥ 90 % :

| SDK | Tool | Format |
|---|---|---|
| Python | `pydocstyle` + custom | % de symboles publics avec docstring |
| Go | `golint -presentation=stylecheck` | % de symboles exportés avec comment |
| TS | `typedoc --validation.notDocumented` | % d'exports avec @doc |
| Java | `javadoc -Xdoclint:missing -Xwerror` | % de symboles publics avec javadoc |

---

## § 12. Surface 8 : Examples polyglottes

### 12.1 Structure

```
examples/
├── tutorial-1-classify-text/
│   ├── python/main.py
│   ├── go/main.go
│   ├── typescript/main.ts
│   └── java/Main.java
├── tutorial-2-batch-classify/
├── tutorial-3-streaming/
├── tutorial-4-async/
├── tutorial-5-retry/
└── tutorial-6-mtls/
```

### 12.2 Pin test : chaque tutorial existe pour chaque langue

```python
def test_phi_c_sdk_examples_complete():
    """Each tutorial must have an implementation in all 4 languages."""
    tutorials = list((REPO / "examples").glob("tutorial-*"))
    for t in tutorials:
        for lang in ["python", "go", "typescript", "java"]:
            assert (t / lang).exists(), (
                f"Tutorial {t.name} missing {lang} implementation"
            )
```

### 12.3 CI pipeline : chaque example doit compiler/run

```yaml
name: examples-smoke
on: pull_request
jobs:
  test:
    matrix:
      lang: [python, go, typescript, java]
    steps:
      - run: ./scripts/sdk/run_examples.sh --lang ${{ matrix.lang }}
```

---

## § 13. Surface 9 : Migration helpers

### 13.1 PyO3 ↔ gRPC drop-in

Pour les users Python existants qui migrent de `EfeClient` (gRPC) à
`EfeNativeClient` (PyO3) ou inverse :

```python
# clients/python/efe/_dropin.py

import os

def get_client(host=None, bundle_path=None, **kwargs):
    """Auto-pick PyO3 or gRPC based on env / args."""
    mode = os.environ.get("EFE_CLIENT_MODE", "auto")
    if mode == "native" or (mode == "auto" and bundle_path):
        from efe._native import EfeNativeClient, load_bundle
        bundle = load_bundle(bundle_path or "/etc/efe/bundles/active")
        return EfeNativeClient(bundle, kwargs.get("catalog_path"))
    else:
        from efe_tool import EfeClient
        return EfeClient(host=host, **kwargs)
```

### 13.2 Cross-SDK migration : un example

Le doc `MIGRATION.md` détaille pour chaque paire de SDKs comment
migrer un workload. Exemple :

> Migration Java → Go (= ré-écriture d'un service)
>
> ```
> Avant (Java) :                              Après (Go) :
> EfeClient client = EfeClient.builder()      client, err := efe.NewClient(
>     .host("efe:443")                          "efe:443",
>     .mtls(cert, key, ca)                      efe.WithMTLS(cert, key, ca),
>     .build();                                  )
> Verdict v = client.classify(text, "cat");    v, err := client.Classify(ctx,
>                                                 &efe.ClassifyRequest{
>                                                   Text: text,
>                                                   CatalogId: "cat",
>                                                 })
> ```

---

## § 14. Surface 10 : SLO opposables polyglottes

### 14.1 Fichier `bench_baselines/v0.6.0/sdk_slos.yaml`

```yaml
version: v0.6.0
domain: sdk
host_fingerprint_baseline: "PLACEHOLDER_compute_at_freeze"
slos:
  - id: SLO-SDK-CONTRACT
    description: "400/400 bit-identity passes (100 fixtures × 4 SDKs)"
    comparator: eq
    threshold_pass_count: 400
    expected_count: 400
    source_bench: contract
  - id: SLO-SDK-LATENCY-GO
    description: "Go SDK p99 latency"
    comparator: lte
    threshold_seconds: 0.050
    source_bench: bench_go
  - id: SLO-SDK-LATENCY-TS
    description: "TS SDK p99 latency"
    comparator: lte
    threshold_seconds: 0.060
    source_bench: bench_ts
  - id: SLO-SDK-LATENCY-JAVA
    description: "Java SDK p99 latency"
    comparator: lte
    threshold_seconds: 0.060
    source_bench: bench_java
  - id: SLO-SDK-DOC-GO
    comparator: gte
    threshold_ratio: 0.90
    source_bench: doc_coverage_go
  - id: SLO-SDK-DOC-TS
    comparator: gte
    threshold_ratio: 0.90
    source_bench: doc_coverage_ts
  - id: SLO-SDK-DOC-JAVA
    comparator: gte
    threshold_ratio: 0.90
    source_bench: doc_coverage_java
  - id: SLO-SDK-SIZE-GO
    comparator: lte
    threshold_bytes: 2097152  # 2 MiB
    source_bench: size_go
  - id: SLO-SDK-SIZE-TS
    comparator: lte
    threshold_bytes: 524288  # 500 KiB
    source_bench: size_ts
  - id: SLO-SDK-SIZE-JAVA
    comparator: lte
    threshold_bytes: 5242880  # 5 MiB
    source_bench: size_java
  - id: SLO-SDK-PROTO-HASH
    comparator: eq
    threshold_pass_count: 1
    expected_count: 1
    source_bench: proto_hash
  - id: SLO-SDK-VERSION-SYNC
    comparator: eq
    threshold_pass_count: 1
    expected_count: 1
    source_bench: version_sync
  - id: SLO-SDK-SIGSTORE
    comparator: gte
    threshold_count: 4
    source_bench: publish
  - id: SLO-SDK-SBOM
    comparator: gte
    threshold_count: 4
    source_bench: publish
  - id: SLO-SDK-API-SURFACE
    comparator: eq
    threshold_pass_count: 1
    expected_count: 1
    source_bench: api_surface
```

### 14.2 Matrice de gating

| SLO | PR-gate | Nightly | Tag-gate |
|---|---|---|---|
| CONTRACT | ✅ | ✅ | ✅ |
| LATENCY-* | ⚠️ inf | ✅ | ✅ |
| DOC-* | ✅ | — | — |
| SIZE-* | — | — | ✅ |
| PROTO-HASH | ✅ | — | ✅ |
| VERSION-SYNC | ✅ | — | ✅ |
| SIGSTORE | — | — | ✅ |
| SBOM | — | — | ✅ |
| API-SURFACE | ✅ | — | ✅ |

---

## § 15. Surface 11 : Idiomes par langue + minimal-divergence policy

### 15.1 Idioms par langue

| Aspect | Python | Go | TypeScript | Java |
|---|---|---|---|---|
| Sync API | `client.classify(text)` | n/a | n/a | n/a |
| Async API | `await client.aclassify(text)` | `client.Classify(ctx, req)` | `await client.classify(req)` | `client.classifyAsync(req)` returns `CompletableFuture<Verdict>` |
| Builder | `EfeClient(host=..., **kwargs)` | `efe.NewClient(host, opts...)` | `new EfeClient({...})` | `EfeClient.builder().host(...).build()` |
| Errors | Exceptions (`EfeError`) | Errors returned (`err`) | Promise rejection | Exceptions (`EfeException`) |
| Naming | snake_case | PascalCase | camelCase | camelCase |
| Verdict access | `verdict.label` | `verdict.Label` | `verdict.label` | `verdict.getLabel()` |

### 15.2 Mapping isomorphique enforced

```python
# scripts/tests/test_polyglot_sdk_structure.py

# Required methods per SDK (= isomorphic mapping)
REQUIRED_METHODS = {
    "classify",
    "classify_async",  # mapped to Classify (Go), classify (TS Promise), classifyAsync (Java)
    "load_bundle",
    "healthcheck",
    "close",
}

def test_sdk_isomorphic_surface():
    for sdk_dir in ["go", "typescript", "java", "python"]:
        symbols = extract_public_symbols(sdk_dir)
        for required in REQUIRED_METHODS:
            mapped = map_to_idiomatic(required, sdk_dir)
            assert mapped in symbols, (
                f"SDK {sdk_dir} missing method {mapped}"
            )
```

### 15.3 Minimal-divergence policy

* Une méthode publique dans **un** SDK qui n'existe pas dans les autres = **REFUSED** par PR-gate.
* Une signature de méthode dans **un** SDK qui prend N args alors que les autres prennent M ≠ N = **REFUSED**.
* Exception : les conventions de retry/circuit breaker peuvent être idiomatiques par SDK (= Go `WithRetry()` option, Java `RetryConfig` class) tant que l'API publique pour `classify` reste isomorphe.

---

## § 16. Matrice complète des artefacts Φ-C-SDK (40 entrées)

| # | Artefact | Chemin | Type | Sprint | SLO/Gap |
|---|---|---|---|---|---|
| 01 | SLO YAML | `bench_baselines/v0.6.0/sdk_slos.yaml` | config | D29 | G2 |
| 02 | Doc archi Φ-C-SDK | `docs/EFE_POLYGLOT_SDKS_COMPLETION_ARCHITECTURE.md` | doc | D29 | — |
| 03 | Proto hash baseline | `bench_baselines/v0.6.0/sdk_proto_hash.txt` | config | D29 | G4 |
| 04 | Codegen orchestrator | `scripts/sdk/regen_all.sh` | tool | D29 | G3 |
| 05 | Go codegen script | `scripts/sdk/regen_go.sh` | tool | D29 | G3 |
| 06 | TS codegen script | `scripts/sdk/regen_ts.sh` | tool | D29 | G3 |
| 07 | Java codegen script | `scripts/sdk/regen_java.sh` | tool | D29 | G3 |
| 08 | Python codegen script | `scripts/sdk/regen_python.sh` | tool | D29 | G3 |
| 09 | Version sync verifier | `scripts/sdk/verify_version_sync.py` | tool | D29 | G5 |
| 10 | Doc coverage verifier | `scripts/sdk/verify_doc_coverage.py` | tool | D29 | G8 |
| 11 | Contract test fixtures | `sdk/contract-runner/fixtures/quartet_fixtures.json` | data | D30 | G1, G17 |
| 12 | Go runner | `sdk/contract-runner/runners/go/main.go` | code | D30 | G1 |
| 13 | TS runner | `sdk/contract-runner/runners/ts/main.ts` | code | D30 | G1 |
| 14 | Java runner | `sdk/contract-runner/runners/java/Main.java` | code | D30 | G1 |
| 15 | Python runner | `sdk/contract-runner/runners/py/main.py` | code | D30 | G1 |
| 16 | Docker compose | `sdk/contract-runner/docker-compose.yml` | infra | D30 | G18 |
| 17 | Verdict comparator | `sdk/contract-runner/compare.py` | tool | D30 | G1 |
| 18 | OTel conventions doc | `docs/sdk-otel-conventions.md` | doc | D31 | G16 |
| 19 | Retry conventions doc | `docs/sdk-retry-conventions.md` | doc | D31 | G19 |
| 20 | Auth conventions doc | `docs/sdk-auth-conventions.md` | doc | D31 | G20 |
| 21 | Go SDK full implementation | `clients/go/` | code | D31 | G9 |
| 22 | TS SDK full implementation | `clients/typescript/` | code | D31 | G9 |
| 23 | Java SDK full implementation | `clients/java/` | code | D31 | G9 |
| 24 | Docusaurus monorepo | `sdk-docs/` | doc | D32 | G10 |
| 25 | Tutorial 1 (classify) × 4 langs | `examples/tutorial-1/` | code | D32 | G11 |
| 26 | Tutorial 2 (batch) × 4 langs | `examples/tutorial-2/` | code | D32 | G11 |
| 27 | Tutorial 3-6 × 4 langs | `examples/tutorial-{3..6}/` | code | D33 | G11 |
| 28 | Python dropin helper | `clients/python/efe/_dropin.py` | code | D33 | G12 |
| 29 | Migration doc | `docs/sdk-MIGRATION.md` | doc | D33 | G12 |
| 30 | Runbook contract test failure | `docs/runbook/sdk_contract_test_failure.md` | doc | D33 | G13 |
| 31 | Runbook version drift | `docs/runbook/sdk_version_drift.md` | doc | D33 | G13 |
| 32 | Runbook publish failure | `docs/runbook/sdk_publish_failure.md` | doc | D33 | G13 |
| 33 | Runbook proto bump | `docs/runbook/sdk_proto_bump.md` | doc | D33 | G13 |
| 34 | Pin test polyglot structure | `scripts/tests/test_polyglot_sdk_structure.py` | test | D34 | G15 |
| 35 | Workflow contract test | `.github/workflows/sdk-quartet-contract.yml` | ci | D34 | G1, G14 |
| 36 | Workflow publish | `.github/workflows/sdk-quartet-publish.yml` | ci | D34 | G6, G7 |
| 37 | Workflow bench | `.github/workflows/sdk-quartet-bench.yml` | ci | D34 | G9 |
| 38 | Examples smoke workflow | `.github/workflows/sdk-examples-smoke.yml` | ci | D34 | G11 |
| 39 | API surface introspector | `scripts/sdk/extract_public_surface.py` | tool | D34 | G15 |
| 40 | Cross-SDK observability dash | `deploy/observability/grafana/efe-sdks.json` | obs | D34 | G16 |

---

## § 17. Plan d'implémentation D29 → D34 chirurgical

### 17.1 D29 (jour 1) — Foundations

| Heure | Tâche | Artefact | DoD |
|---|---|---|---|
| 09:00–10:00 | Archi doc finalisation + SLO YAML | 01, 02 | 15 SLO declared |
| 10:00–11:00 | Proto hash baseline + codegen orchestrator | 03, 04 | sha256 freeze |
| 11:00–12:00 | 4 codegen scripts per-SDK | 05, 06, 07, 08 | `regen_all.sh` runs successfully |
| 14:00–15:30 | Version sync + doc coverage verifiers | 09, 10 | CLI works locally |
| 15:30–17:30 | Initial CI workflow skeleton | 35, 36 partial | yamllint passes |

**Go/No-Go D29 EOD** : `scripts/sdk/regen_all.sh` produit 4 codegen dirs sans erreur.

### 17.2 D30 (jour 2) — Contract test harness

| Heure | Tâche | Artefact | DoD |
|---|---|---|---|
| 09:00–10:00 | Quartet fixtures generation | 11 | 100 fixtures stable |
| 10:00–12:00 | Docker compose + efe-server boot | 16 | Containers up + healthchecks pass |
| 14:00–15:30 | Python runner | 15 | Output `out_py.json` correct |
| 15:30–17:30 | Go runner | 12 | Output `out_go.json` correct |

**Go/No-Go D30 EOD** : 2 SDKs (Python + Go) émettent des verdicts bit-identiques.

### 17.3 D31 (jour 3) — Remaining runners + idiomatic SDK build

| Heure | Tâche | Artefact | DoD |
|---|---|---|---|
| 09:00–11:00 | TS runner | 13 | `out_ts.json` matches |
| 11:00–13:00 | Java runner | 14 | `out_java.json` matches |
| 14:00–15:30 | Comparator `compare.py` | 17 | 400/400 pass |
| 15:30–17:30 | Go/TS/Java SDK polish (= functional options, retry, circuit breaker) | 21, 22, 23 | All public methods documented |

**Go/No-Go D31 EOD** : Contract test runs 400/400 PASS, all 4 SDKs functional.

### 17.4 D32 (jour 4) — Docs + 2 first tutorials

| Heure | Tâche | Artefact | DoD |
|---|---|---|---|
| 09:00–11:00 | Docusaurus monorepo init | 24 | Builds without errors |
| 11:00–13:00 | OTel/retry/auth convention docs | 18, 19, 20 | 3 docs created |
| 14:00–17:30 | Tutorial 1 + 2 × 4 langs | 25, 26 | 8 code samples, all compile/run |

**Go/No-Go D32 EOD** : Site Docusaurus serves locally, 2 tutorials × 4 langs all green.

### 17.5 D33 (jour 5) — Remaining tutorials + dropin + runbooks

| Heure | Tâche | Artefact | DoD |
|---|---|---|---|
| 09:00–12:00 | Tutorials 3-6 × 4 langs | 27 | 16 code samples |
| 14:00–15:00 | Python dropin + migration doc | 28, 29 | `efe._dropin.get_client()` works |
| 15:00–17:30 | 4 runbooks | 30, 31, 32, 33 | Each has Symptoms / Diagnostic / Mitigation |

**Go/No-Go D33 EOD** : Full documentation site usable end-to-end.

### 17.6 D34 (jour 6) — CI + pin tests + Φ-C-SDK go-live

| Heure | Tâche | Artefact | DoD |
|---|---|---|---|
| 09:00–11:00 | Pin tests polyglotte | 34 | 12 tests all green |
| 11:00–12:00 | Workflow contract (extended) | 35 | yamllint OK + dry-run OK |
| 14:00–15:00 | Workflow publish (extended) | 36 | All 4 publishers configured |
| 15:00–16:00 | Workflow bench (extended) | 37 | nightly cron defined |
| 16:00–17:00 | Examples smoke workflow | 38 | matrix runs |
| 17:00–17:30 | API surface introspector | 39 | extracts symbol lists |
| 17:30–18:00 | Grafana dash + final commit | 40 | dashboard JSON valid |

**Go/No-Go D34 EOD = Φ-C-SDK go-live** :
* Tous les 40 artefacts livrés.
* `pytest scripts/tests/test_polyglot_sdk_structure.py` 12/12 green.
* Contract test 400/400 PASS sur compliance JSON synthétique.
* 4 SDKs build + test localement.

---

## § 18. Risques identifiés & mitigations (20)

| # | Risque | Sévérité | Probabilité | Mitigation |
|---|---|---|---|---|
| R1 | gRPC version mismatch entre SDKs | HIGH | LOW | Pin protoc version + grpc-go/grpc-java/@grpc/grpc-js versions |
| R2 | Maven Central deploy failure (GPG cert expired) | HIGH | MED | Pre-flight check cert validity 30 days avant release |
| R3 | npm package size > 500 KiB après tree-shake | MED | MED | Lazy-load proto definitions ; mark as side-effect-free |
| R4 | Go modules version mismatch (proxy cache) | MED | LOW | `GOPROXY=direct` pour dev, `proxy.golang.org` pour CI |
| R5 | Contract test flaky sur runner CI (= timing) | HIGH | MED | Use Docker compose healthchecks + retry logic |
| R6 | Proto hash drift accidentel (= dev modifie sans bumper) | HIGH | MED | Pin test PR-gate ; CONTRIBUTING.md mentionne le process |
| R7 | TS SDK build break sur Node 22 | MED | LOW | Test matrix Node 18, 20, 22 |
| R8 | Java SDK Javadoc not honoring `-Xdoclint:missing` | MED | LOW | Test in `mvn -B verify` |
| R9 | Docusaurus build failure sur tag deploy | MED | MED | `pnpm build` in CI pre-merge |
| R10 | Confidence drift > 1e-6 sur certains fixtures (= f32 rounding edge) | MED | LOW | Document tolerance ; investigate if > threshold |
| R11 | Cosign keyless OIDC failure (= GitHub Actions transient) | LOW | LOW | Retry signing step ; 3 attempts |
| R12 | Migration helper breaks existing users (= API change) | HIGH | LOW | `_dropin.get_client()` is additive ; pre-existing code untouched |
| R13 | Auto-instrumentation Java agent breaks tracing context propagation | MED | MED | Use opentelemetry-grpc 1.6+ ; manual fallback documented |
| R14 | Doc site SEO issues (= fragmented per-language) | LOW | MED | Canonical URLs + Docusaurus search |
| R15 | Customer reports verdict drift between SDKs in prod | CRIT | LOW | Contract test catches in CI ; runbook for post-release diff |
| R16 | Go SDK `context.Context` not respected dans tous les paths | MED | MED | Linter check via `staticcheck` |
| R17 | TS SDK Promise rejection not catchable (= unhandled) | HIGH | MED | Test harness expects rejection catching ; document |
| R18 | Java SDK shaded jar conflicts with user gRPC version | HIGH | MED | Use `shadow plugin` with `relocate` |
| R19 | Pin test cross-SDK incorrectly accepts asymmetric surface | HIGH | MED | Test the test (= manually break surface to verify red) |
| R20 | Examples drift (= update one lang but not others) | MED | HIGH | CI matrix runs all 4 examples for each tutorial |

---

## § 19. Décisions de conception clés (18)

| # | Décision | Rationale |
|---|---|---|
| D1 | Polyglot quartet : Python + Go + TS + Java | Couverture ~ 95 % du marché enterprise + cloud-native |
| D2 | Une seule version semver cross-SDK | Évite confusion utilisateur ; simplifie release |
| D3 | Contract test bit-identique (= pas tolerance) | Garantie opposable ; tout drift = bug à fixer |
| D4 | Confidence tolerance ε=1e-6 | f32 wire roundtrip drift |
| D5 | Codegen pinned protoc version | Reproducibility deterministic |
| D6 | OTel cross-SDK (pas Prometheus client direct) | OTel = standard de fait 2026 |
| D7 | Sigstore keyless (pas key-based) | Pas de key management long-terme |
| D8 | SBOM CycloneDX 1.5 (pas SPDX) | Standard industrie 2026 |
| D9 | Maven Central (pas OSSRH staging) | Direct deploy via Sonatype |
| D10 | npm public (pas private registry) | Adoption SDK = public scope |
| D11 | Go modules sur GitHub (pas private proxy) | Convention Go ; `go get` direct |
| D12 | Docusaurus (pas mkdocs / Sphinx unifié) | Polyglotte + tabs natifs + MDX |
| D13 | Drop-in helper (pas API change) | Préserve backward-compat |
| D14 | Docker compose pour contract test (pas k8s) | Plus simple, suffit pour fixtures |
| D15 | Minimal-divergence policy (pas full isomorphism) | Respect des idioms |
| D16 | 6 tutorials minimum (= 6 use cases x 4 langs = 24 examples) | Couverture pédagogique |
| D17 | Auto-instrumentation OTel preferred over manual | Réduit boilerplate user code |
| D18 | Java JDK 21 (= LTS) minimum | Adoption enterprise 2026 |

---

## § 20. Anti-régression cross-SDK

### 20.1 Pin tests doctrine

Chaque dérive potentielle a un pin test dedicated :

| Risque | Pin test |
|---|---|
| Proto change without bump | `test_sdk_proto_hash_unchanged` |
| Version drift | `test_sdk_versions_in_sync` |
| Surface asymmetry | `test_sdk_isomorphic_surface` |
| Missing tutorial in a lang | `test_phi_c_sdk_examples_complete` |
| OTel instrument not standardized | `test_sdk_otel_instruments_standardized` |
| Sigstore signing missing | `test_phi_c_sdk_all_artifacts_signed` |
| SBOM missing | `test_phi_c_sdk_all_artifacts_have_sbom` |
| Contract drift | `test_phi_c_sdk_contract_400_pass` |
| Docs coverage below 90 % | `test_phi_c_sdk_doc_coverage_above_90` |

### 20.2 Convention "tests le test"

Chaque pin test doit avoir un "anti-test" qui prouve qu'il fail correctement quand la condition est violée. Exemple :

```python
def test_meta_proto_hash_drift_detected():
    """Inject a fake proto hash drift and verify the pin test fails."""
    # ... simulate proto modification ...
    with pytest.raises(AssertionError):
        test_sdk_proto_hash_unchanged()
```

---

## § 21. Critères d'acceptation (DoD à 100 %)

### 21.1 DoD agrégé

```text
Φ-C-SDK DoD =
   (15/15 SLO parseables par check_slo.py)
 ∧ (40/40 artefacts livrés)
 ∧ (Contract test 400/400 PASS)
 ∧ (CI sdk-quartet-{contract,publish,bench} verts)
 ∧ (12/12 pin tests polyglotte verts)
 ∧ (0 scripts référencés-mais-absents)
 ∧ (0 lint warnings critiques cross-SDK)
 ∧ (4 SDKs publishable en parallèle au tag v0.6.0)
```

### 21.2 Per-SDK DoD

| SDK | Code | Tests | Doc | Build | Publish |
|---|---|---|---|---|---|
| Python | Φ-C-FFI ✅ | Φ-C-FFI ✅ | Sphinx + Docusaurus | maturin | PyPI signed + SBOM ✅ |
| Go | Functional opts + retry + OTel | `go test ./...` 100 % | godoc + Docusaurus | `go build` | Git tag + cosign + SBOM |
| TS | Promise API + ESM/CJS dual | `vitest` 100 % | TypeDoc + Docusaurus | `pnpm build` | npm publish + cosign + SBOM |
| Java | Builder + CompletableFuture | `mvn verify` 100 % | Javadoc + Docusaurus | `mvn package` | Maven Central + GPG + cosign + SBOM |

---

## § 22. Hand-off & dépendances downstream

### 22.1 Consommateurs Φ-C-SDK

| Consommateur | Sprint | Use case |
|---|---|---|
| LangChain integration | D35+ | `langchain.retrievers.EfeRetriever` utilise `@efe-ai/sdk` ou `efe-py` |
| AWS Lambda example | D36 | Go SDK pour serverless |
| Vercel Edge example | D36 | TS SDK pour edge functions |
| Spring Boot starter | D37 | `ai.efe:efe-spring-boot-starter` wraps Java SDK |
| Customer pilots | D38+ | 3-5 design partners test all 4 SDKs |

### 22.2 Dépendances amont gelées au Φ-C-SDK freeze

* `protos/efe/v1/*.proto` (= source of truth gelée).
* `efe-server` v0.6.0 (= backend gRPC stable).
* `efe._native` v0.6.0 (= Python PyO3 SDK).
* protoc v25.1, protoc-gen-go-grpc v1.5, ts-proto v1.169, grpc-java v1.62.
* OTel SDK versions : Python 1.27, Go 1.30, JS 1.27, Java 1.42.

---

## § 23. Annexes (A → H)

### Annexe A — Mapping SLO ↔ Test ↔ Workflow

(cf. § 14 + matrice § 16.)

### Annexe B — Idiomatic API examples per SDK

(cf. § 11.2 tabs.)

### Annexe C — Commandes opérationnelles

```bash
# Regen all 4 SDKs from proto
./scripts/sdk/regen_all.sh

# Run contract test locally
cd sdk/contract-runner && docker-compose up --build

# Compare outputs
python sdk/contract-runner/compare.py \
    --go     /shared/out_go.json \
    --ts     /shared/out_ts.json \
    --java   /shared/out_java.json \
    --py     /shared/out_py.json

# Verify version sync
python scripts/sdk/verify_version_sync.py

# Verify doc coverage cross-SDK
python scripts/sdk/verify_doc_coverage.py --min 0.90

# Run pin tests
pytest scripts/tests/test_polyglot_sdk_structure.py -v
```

### Annexe D — Glossaire publishing

| Terme | Définition |
|---|---|
| OSSRH | Sonatype Open Source Software Repository Hosting (= staging pour Maven Central) |
| npm tarball | `*.tgz` archive du package npm |
| Go module proxy | `proxy.golang.org` cache des Go modules versions |
| GPG | GNU Privacy Guard, requis pour Maven Central |
| OIDC keyless | Sigstore signing via GitHub OIDC token (no key needed) |

### Annexe E — Hash des proto canoniques (post-freeze)

À remplir post-Φ-C-SDK release dans `bench_baselines/v0.6.0/sdk_proto_hash.txt`.

### Annexe F — Format Verdict canonique

(cf. § 4.1 protobuf message.)

### Annexe G — Erreurs courantes & diagnostic

| Erreur | Cause | Mitigation |
|---|---|---|
| `ContractTestFailure: label mismatch` | SDK codegen drift | Re-run `regen_all.sh` |
| `Maven OSSRH 401 Unauthorized` | OSSRH credentials expired | Renew via Sonatype Jira |
| `cosign: error: no OIDC token` | GitHub Actions OIDC permission missing | Add `id-token: write` to job |
| `protoc-gen-go-grpc: not found` | Missing protoc plugin install | `go install google.golang.org/grpc/cmd/protoc-gen-go-grpc@latest` |
| `npm publish 403 Forbidden` | Package already exists | Bump version semver |

### Annexe H — Liens & références

* Archis individuelles : `EFE_GO_SDK_*`, `EFE_JAVA_SDK_*`, `EFE_TYPESCRIPT_SDK_*`, `EFE_PYTHON_SDK_*`.
* Φ-C-FFI : `EFE_PYO3_FFI_COMPLETION_ARCHITECTURE.md` (= Python SDK substrate).
* Proto source : `protos/efe/v1/*.proto`.
* Sigstore docs : <https://docs.sigstore.dev/>
* CycloneDX 1.5 : <https://cyclonedx.org/specification/overview/>
* OpenTelemetry SDK matrix : <https://opentelemetry.io/docs/instrumentation/>

---

**Fin du document Φ-C-SDK Polyglot Completion.**

Document préparé pour exécution chirurgicale D29 → D34 par l'équipe EFE.
Toute déviation vs ce plan doit être tracée dans le journal de sprint
et auditée Level 1 + Level 2.
