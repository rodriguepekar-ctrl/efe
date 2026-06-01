# `efe-sdk-java` (Java SDK) — Architecture détaillée de l'extension M11

> **Statut** : Draft v1.0.0 — Document de référence pour la
> **création from-scratch** du module **M11 = `clients/java/efe-sdk/`**.
> **Greenfield** : ce module n'existe PAS encore (✓ vérifié AS-IS au
> 2026-04-30 ; `clients/` ne contient que `python/`, `go/`, `typescript/`).
> **Auteur** : rodrigue.pekar@icloud.com
> **Date** : 2026-04-30.
>
> **Dépendances doc** :
>   * `docs/PRODUCT_COMPLETION_ARCHITECTURE.md` §6.4 — sprint D26
>     dans Φ-B, et §15 — surface SDK multi-runtime (Python M8 + Go
>     M9 + TypeScript M10 + Java M11).
>   * `docs/EFE_PYTHON_SDK_EXTENSION_ARCHITECTURE.md` — référence M8.
>   * `docs/EFE_GO_SDK_EXTENSION_ARCHITECTURE.md` — référence M9.
>   * `docs/EFE_TYPESCRIPT_SDK_EXTENSION_ARCHITECTURE.md` — référence
>     M10. Le Java SDK reprend le pattern d'isomorphisme (à un seul
>     runtime près — pas de variante "browser" pour Java).
>   * `LETTER_E_ARCHITECTURE.md` — service backend single-tenant que
>     le SDK consomme (via gRPC, post-D23).
>   * `protos/efe/v1/*.proto` — wire contract (efe.proto + types.proto
>     + catalog.proto + admin.proto + mode2.proto + mode3.proto).
>
> **Portée** : **1 sprint atomique D26** (Φ-B), 10 jours wallclock.
> Création complète d'un module Maven publishable :
> client gRPC auto-généré via `protobuf-maven-plugin` + API
> idiomatique Java (Builder pattern, AutoCloseable, CompletableFuture)
> + OTel auto-instrumentation + Prometheus auto-export + retry
> exponentiel + auth JWT + mTLS via `GrpcSslContexts` + 3 exemples
> Spring Boot (MVC + WebFlux + Actuator starter) + tests JUnit 5
> + AssertJ + Mockito + Maven Central pipeline (signed JARs via
> GPG) + Sonatype Central Portal staging + GitHub Packages private
> registry pour Φ-B.
>
> **Cible GA** : `ai.efe:efe-sdk-java:v1.0.0` packagé publiquement
> sur Maven Central ; aligné semver-wise avec le produit (D26 =
> v0.4.0 privé, GA = v1.0.0 public).

---

## Table des matières

§0. Méta, glossaire, conventions
§1. Mission de l'extension M11 (Φ-B)
§2. Position dans la topologie EFE post-M10
§3. État AS-IS au 2026-04-30 (greenfield)
§4. Définition opérationnelle de M11 v1.0 GA
§5. Principes directeurs (idiomes Java)
§6. Invariants structurels (layout Maven multi-module)
§7. Vue d'ensemble — sprint D26 unique
§8. Surface publique cible (API Java publishable)
§9. **Sprint D26 — module Maven entier**
§10. Sub-système 1 — `EfeClient` + Builder + AutoCloseable
§11. Sub-système 2 — gRPC code generation pipeline (`protobuf-maven-plugin`)
§12. Sub-système 3 — Errors + retry + `ClientInterceptor` chain
§13. Sub-système 4 — OpenTelemetry auto-instrumentation
§14. Sub-système 5 — Prometheus metrics
§15. Sub-système 6 — Auth (JWT `CallCredentials` + mTLS `GrpcSslContexts`)
§16. Sub-système 7 — 3 Spring Boot examples (MVC + WebFlux + Actuator)
§17. Sub-système 8 — Mock infrastructure (`InProcessServerBuilder` + `MockEfeClient`)
§18. Stratégie de tests (JUnit 5 + AssertJ + Mockito + JaCoCo coverage)
§19. JPMS `module-info.java` + Javadoc strategy
§20. JAR shading + dependency conflict isolation (`grpc-netty-shaded`)
§21. SLO mapping (cold start + latence + JAR size)
§22. Risques transverses + mitigations
§23. Volumétrie code attendue
§24. Plan d'implémentation J1-J10
§25. Versioning + publication Maven Central (private → public à GA)
§26. Compliance + sécurité (signed JARs, GPG, supply-chain)
§27. Annexes — `pom.xml` parent canonical
§28. Annexes — surface API Java complète (Javadoc-style)
§29. Annexes — Maven plugins config + scripts
§30. Annexes — RACI + budget + Gantt
§31. Mises à jour du document

---

## §0. Méta, glossaire, conventions

### §0.1 Conventions

* **AS-IS** = état du repo à fin D24 (post-TS SDK livré + audit pass 1
  GREEN, v0.3.0 publishable privé).
* **TO-BE** = état cible défini par le sprint D26.
* Le sprint D26 est **dans la phase Φ-B** (Lettre E + multi-runtime
  SDKs). D21 (Lettre E + AdminService) + D22 (mTLS + JWT scopes) +
  D23 (Go SDK) + D24 (TS SDK) sont des prérequis cohérents.
* Les défauts résiduels suivent **DT-J.x** (Defect Trace, Java SDK).
* **Module Maven** = `ai.efe:efe-sdk-java` à publier. Pendant le
  développement (avant publication Maven Central / GitHub Packages),
  le path filesystem est `clients/java/efe-sdk/`. Le scope `ai.efe`
  est privé pendant Φ-B (GitHub Packages registry) puis public à GA
  (Maven Central via Sonatype Central Portal).
* **Symboles `EfeClient`, `Verdict`, `ClientOptions`, `EfeException`**
  sont les noms publics canoniques (PascalCase Java). Méthodes en
  camelCase : `classify(text)`, `healthcheck()`, `close()`.
* **Verdict Java** = même 10 fields cross-language que Python M8 +
  Go M9 + TS M10. Java récord-style immuable + camelCase field names
  qui matchent le wire JSON proto3 default.
* `[ref:fichier:ligne]` = pointeur dans le code source.

### §0.2 Glossaire

| Terme | Définition |
|-------|------------|
| **`efe-sdk-java`** | Le module Maven `clients/java/efe-sdk/` qui héberge le SDK Java pour EFE. groupId : `ai.efe` ; artifactId : `efe-sdk-java`. AS-IS : 0 LOC (greenfield). |
| **`io.grpc:grpc-netty-shaded`** | Le client gRPC Java canonique pour la prod. Variante "shaded" qui embarque sa propre Netty + boringssl pour ALPN, évitant les conflits avec d'autres versions de Netty dans l'application consommateur. Alternative `grpc-netty` (non-shaded) impose à l'opérateur de configurer manuellement Netty + Java SSL provider — fragile. |
| **`protobuf-maven-plugin`** | Plugin Maven (groupId `org.xolstice.maven.plugins`) qui invoque `protoc` + `protoc-gen-grpc-java` lors de la phase `generate-sources`. Output dans `target/generated-sources/protobuf/{java, grpc-java}/`. Mêmes `.proto` que Python M8 + Go M9 + TS M10. |
| **`EfeClient`** | La classe publique qui wrappe le gRPC `ManagedChannel` + options (auth, OTel, retry, mTLS). Construite via `EfeClient.builder()...build()`. Implémente `AutoCloseable` pour try-with-resources. |
| **`Verdict`** | Le type publique des résultats de classification. **Java 17 record** — immutabilité par construction + builder generated automatiquement par le compilateur. 10 fields cross-language. |
| **`ClientOptions`** | Le **builder pattern** Java canonique (vs Go functional options ou TS options-object). Construit via `ClientOptions.builder().endpoint("...").auth(...).build()`. |
| **`CallCredentials`** | Interface gRPC Java pour injecter dynamiquement des credentials par appel. On l'utilise pour le JWT bearer (vs un interceptor statique qui figerait le token). |
| **`GrpcSslContexts`** | Helper class Netty-grpc pour construire un `SslContext` pour mTLS client (`forClient().keyManager(cert, key).trustManager(ca).build()`). Le SDK encapsule via la classe `MtlsBuilder`. |
| **`InProcessServerBuilder`** | Helper gRPC Java pour spawner un serveur in-memory transporté via un nom de canal symbolique (équivalent du `bufconn` Go). Pas de socket TCP — pure JVM, plus rapide. Utilisé par les tests d'intégration JUnit 5. |
| **`MockEfeClient`** | Classe Java de test (subpackage `ai.efe.sdk.mock`), drop-in replacement pour `EfeClient` que les consumers utilisent dans JUnit. API : `setVerdict(v)`, `setError(e)`, `getCalls()`, `reset()`. |
| **JPMS** | **Java Platform Module System** (depuis Java 9). Le SDK ship un `module-info.java` qui déclare explicitement les packages exportés + les modules requis. Améliore l'encapsulation + supporte `jlink` pour images custom. |
| **shaded JAR** | JAR qui embarque ses dépendances (typically renommées via `maven-shade-plugin`) pour éviter les "JAR hell" de versions conflictuelles. `grpc-netty-shaded` pré-shade Netty ; on N'A PAS besoin de re-shade le SDK lui-même. |
| **GPG signed JAR** | Pour Maven Central, chaque artefact (JAR, sources JAR, javadoc JAR, POM) doit être signé par une clé GPG. Le plugin `maven-gpg-plugin` automatise le `.asc` signature file generation. La clé publique doit être uploaded sur un keyserver public (e.g., `keys.openpgp.org`). |
| **Sonatype Central Portal** | Le nouveau moyen (depuis 2024) de publier sur Maven Central. Remplace l'ancien OSSRH (Nexus Staging). On stage des artefacts + ils transitent automatiquement vers le synch repository. Le SDK utilisera le plugin `central-publishing-maven-plugin`. |
| **`@Nullable` / `@NonNull`** | Annotations utilisables (`org.jspecify`, `org.checkerframework`, `javax.annotation`) pour signaler la nullité statique aux IDE + analyzers. Le SDK adopte JSpecify (canonical Java 2024+). |
| **Spring Boot starter** | Pattern où un module `efe-sdk-spring-boot-starter` expose une `@AutoConfiguration` qui produce des beans `EfeClient` from properties. M11 livre 1 example qui DEMONSTRE ce pattern (mais ne livre PAS le starter publié séparément — reporté D27+). |

### §0.3 Notation

* Les blocs `text-with-tabs` sans surlignage = texte canonique copié
  tel quel dans le code Java / pom.xml.
* Les blocs ```java = Java pseudo-code didactique.
* Les blocs ```xml = Maven POM fragments.
* Les blocs ```bash = commandes shell exécutables.
* Tableaux pin tests = id `DT-J.<sprint>.<n>` à matérialiser dans
  `scripts/tests/test_d26_java_sdk_extension.py`.

---

## §1. Mission de l'extension M11 (Φ-B)

**Phrase canonique** :

> Doter EFE d'un **SDK Java** publishable Maven Central, **enterprise-grade**
> (signed JARs, JPMS modulaire, javadoc complet), **observable** (OTel
> auto + Prometheus 5 collecteurs), et **inclusif** (3 exemples Spring
> Boot couvrant MVC + WebFlux + Actuator) — **en 1 sprint atomique
> D26** de 10 jours wallclock.

**Pourquoi maintenant (Φ-B post-M10)** :

1. **Demande consumer enterprise** — 40% des consumers EFE pre-D24
   évaluaient le SDK Python (M8) puis se rabattaient sur du REST proxy
   custom parce que leur stack tech était Java/JVM. Spring Boot reste
   le framework dominant côté serveur enterprise (~70% des nouveaux
   projets backend). Sans M11, EFE rate ce segment.

2. **Cohérence cross-language** — M8 (Python) + M9 (Go) + M10 (TS)
   livrent une surface SDK consistente. Java ferme la boucle des **4
   langages stratégiques** identifiés dans `PRODUCT_COMPLETION_ARCHITECTURE.md`.
   Le pin test `test_m11_verdict_field_set_matches_m8_m9_m10` anchor
   cette cohérence (mêmes 10 fields).

3. **Maven Central pipeline** — D26 livre une chaîne de publication
   complète (signed JARs + sources + javadoc + GPG + central-portal-staging).
   Cette infrastructure est réutilisable pour TOUS les futurs modules
   Java EFE (efe-sdk-spring-boot-starter en D27+, efe-cli-java en
   D28+, etc.).

4. **Spring Boot integration patterns** — Les 3 exemples canonisent
   l'usage du SDK dans les 3 patterns Spring Boot dominants : MVC
   (synchrone, controller @RestController), WebFlux (asynchrone,
   `@RestController` returning `Mono<Verdict>`), Actuator (observability
   metrics auto-exposed). Les ops apprennent en 30 min comment
   intégrer dans leur stack.

5. **Cohérence avec PRODUCT_COMPLETION** —
   `PRODUCT_COMPLETION_ARCHITECTURE.md` §6.4 liste D26 comme un
   sprint Φ-B avant D27 (release-engineering + SLO consolidation).
   M11 est le DERNIER SDK greenfield majeur ; M12+ sera de la
   maintenance + de l'évolution incrémentale.

**Non-objectifs** (explicitement OUT du sprint D26) :

* **`efe-sdk-spring-boot-starter` publishable** — la auto-configuration
  + properties binding sera livrée en D27 comme module séparé
  (`ai.efe:efe-sdk-spring-boot-starter`). En D26, on livre uniquement
  un example qui démontre comment écrire son propre starter custom.
* **Reactive Streams (Reactor) backbone integré** — Java SDK D26 expose
  `CompletableFuture` natif pour l'async + un example WebFlux qui
  wrap dans `Mono.fromFuture(...)`. Pas de dépendance à Reactor en
  core ; l'app peut wrapper si besoin.
* **GraalVM native image** — Java SDK ne ship pas de configuration
  native-image en D26. Reporté D28+ (la chaîne `grpc-netty-shaded`
  + Netty + reflection est non-trivial à GraalVM-iser, +200 LOC de
  config).
* **Module path Maven renommage post-GA** — Une fois v1.0.0 publié
  sous `ai.efe:efe-sdk-java`, l'identité (groupId + artifactId) est
  immuable. Tout bump majeur (v2.0.0) requiert un nouveau artifactId
  ou un sub-path import migration.
* **Java < 17 support** — Cible : Java 17 LTS minimum. Pas de Java 11
  / 8. Les consumers en Java 11 devront updater (Java 17 a 5 ans
  d'âge en 2026).
* **Android SDK** — Le SDK Java cible JVM stack server-side. Pas
  Android (qui exige `protoc-gen-grpc-java-lite`, des fixtures
  spécifiques, et un workflow CI séparé). Reporté D29+.

---

## §2. Position dans la topologie EFE post-M10

```
┌───────────────────────────────────────────────────────────────────────┐
│              EFE Topology — post-M10 livré + audit pass 1             │
└───────────────────────────────────────────────────────────────────────┘

  Layer L4 — Consumer surface
  ┌───────────────────────────────────────────────────────────────────┐
  │  efe-tool CLI (Python)        efe-sdk-go (Go)                     │
  │  ↑ M8 livré D19/D20/D25       ↑ M9 livré D23 + audit GREEN        │
  │                                                                   │
  │  @efe/sdk (TS Node + Browser)                ??? (Java)           │
  │  ↑ M10 livré D24 + audit GREEN     ←  M11 GAP ici                 │
  └───────────┬───────────────────────────┬─────────────────┬─────────┘
              │ gRPC                      │ gRPC + grpc-web │ gRPC
              ▼                           ▼                 ▼
  Layer L3 — Wire contract (efe.v1)
  ┌───────────────────────────────────────────────────────────────────┐
  │   protos/efe/v1/*.proto — types.proto, efe.proto, catalog.proto,  │
  │   admin.proto, mode2.proto, mode3.proto                           │
  └───────────────────────────────────────────────────────────────────┘
              │
              ▼
  Layer L2 — Server (Lettre E single-tenant + mTLS + JWT scopes)
  ┌───────────────────────────────────────────────────────────────────┐
  │  efe-server (Rust) — exposes :50051 gRPC + :50052 gRPC-Web         │
  └───────────────────────────────────────────────────────────────────┘
              │
              ▼
  Layer L1 — Compute kernel (Rust + LibTorch + cones)
  ┌───────────────────────────────────────────────────────────────────┐
  │  efe-core + efe-cones + efe-fusion (M3..M7)                        │
  └───────────────────────────────────────────────────────────────────┘
```

**Où M11 s'insère** :

* **Layer L4 (consumer surface)** — comble le `??? (Java)` gap. Après
  M11, les **4 langages stratégiques** (Python, Go, TS, Java) sont
  couverts.
* **Layer L3 (wire contract)** — M11 consomme **les mêmes** `protos/efe/v1/*.proto`
  que M8/M9/M10. Aucune extension proto requise. Les pin tests
  `test_d26_java_sdk_protos_in_sync` matchent les SHA-256 des protos
  contre `clients/java/efe-sdk/efe-sdk-core/src/main/proto/PROTOS_HASH.txt`.
* **Layer L2 (server)** — Lettre E expose le port `:50051` gRPC natif.
  Java SDK consomme via `grpc-netty-shaded` (HTTP/2). Pas de besoin
  d'Envoy translator (contrairement au TS browser via grpc-web).
* **Layer L1 (compute)** — pas d'impact M11.

**Couplage M11 ↔ M8/M9/M10** : 0 dépendance fonctionnelle. Les 4
SDKs sont indépendants ; ils partagent UNIQUEMENT le wire contract
proto + les invariants cross-language (10 sentinels, 10 Verdict
fields, 5 metric names). Cohérence validée par pin tests Python qui
charge les 4 SDKs et compare les surfaces.

---

## §3. État AS-IS au 2026-04-30 (greenfield)

### §3.1 Ce qui existe déjà

* `clients/python/efe_tool/` — SDK Python (M8 livré + audit GREEN).
* `clients/go/efe-sdk/` — SDK Go (M9 livré + audit GREEN).
* `clients/typescript/@efe/sdk/` — SDK TypeScript (M10 livré + audit pass 1 GREEN).
* `protos/efe/v1/*.proto` — wire contract stable depuis D17 (modulo
  ajouts compatible-additive D18..D22).
* `docs/EFE_PYTHON_SDK_EXTENSION_ARCHITECTURE.md` (2 359 lignes) —
  référence M8.
* `docs/EFE_GO_SDK_EXTENSION_ARCHITECTURE.md` (2 257 lignes) —
  référence M9.
* `docs/EFE_TYPESCRIPT_SDK_EXTENSION_ARCHITECTURE.md` (3 345 lignes) —
  référence M10.

### §3.2 Ce qui n'existe PAS encore

* `clients/java/` — répertoire entier inexistant. **AS-IS = 0 fichier**.
* `pom.xml` (Maven) — inexistant n'importe où dans le repo.
* CI workflows `.github/workflows/sdk-java-test.yml` +
  `sdk-java-smoke.yml` + `sdk-java-publish.yml` — inexistants.
* Pin tests Python `scripts/tests/test_d26_java_sdk_extension.py` —
  inexistants.
* CHANGELOG entrée M11 — inexistante.

### §3.3 Capacités de l'environnement de dev

* **Java JDK** : on suppose 17+ installé. Si absent, le CI
  `actions/setup-java@v4` provisionne. Localement, le dev install
  via `sdkman` ou OS-specific (`brew install openjdk@17` sur macOS).
* **Maven** : version ≥ 3.9 requis (pour central-publishing-maven-plugin
  + JPMS support clean). `brew install maven` sur macOS, ou via
  sdkman.
* **`protoc`** : non requis localement — le `protobuf-maven-plugin`
  télécharge le binary protoc canonique au build time depuis
  Maven Central.
* **GPG** : pour signer les JARs en publish. Pas requis en dev pour
  build/test, uniquement en publish. `brew install gnupg`.
* **Java 21** : optionnel — utilisé pour la matrice CI (vérifier que
  le SDK reste compatible avec Java 17 LTS).

### §3.4 Vérification AS-IS

Commande de vérification au 2026-04-30 :

```bash
ls -la clients/
# ATTENDU : drwxr-xr-x ... go/  python/  typescript/
# (PAS de java/)

find clients -name 'pom.xml' 2>/dev/null
# ATTENDU : (vide)

mvn --version
# ATTENDU : Apache Maven ≥ 3.9 + Java ≥ 17
```

→ **Greenfield confirmé**. M11 part de zéro.

---

## §4. Définition opérationnelle de M11 v1.0 GA

### §4.1 Scope fonctionnel

M11 est **GA-ready** quand toutes les capabilities suivantes sont
livrées + tests verts + pin tested :

| Capability | Surface | Test plan |
|---|---|---|
| **Construct + close client** | `EfeClient.builder().endpoint("...").build()` + `client.close()` (try-with-resources) | JUnit 5 unit |
| **Classify (sync)** | `Verdict v = client.classify("text")` | JUnit integration vs `InProcessServer` |
| **Classify (async)** | `CompletableFuture<Verdict> f = client.classifyAsync("text")` | JUnit integration |
| **Classify (stream)** | `Stream<StreamItem> = client.classifyStream(texts)` | JUnit integration |
| **Healthcheck** | `client.healthcheck()` (throws on not-serving) | JUnit integration |
| **JWT auth** | `ClientOptions.builder().auth(JwtAuth.of("token")).build()` | JUnit spec — header asserted |
| **mTLS auth** | `ClientOptions.builder().auth(MtlsAuth.fromFiles(certPath, keyPath, caPath)).build()` | JUnit spec — `SslContext` produced |
| **Retry + backoff** | `ClientOptions.builder().retry(RetryConfig.exponential(3, ...)).build()` | JUnit spec — 3 attempts on UNAVAILABLE |
| **OTel auto** | `ClientOptions.builder().otel(openTelemetry).build()` | JUnit spec — span shape |
| **Prometheus metrics** | `ClientOptions.builder().prometheus(promRegistry).build()` — 5 collectors canoniques | JUnit spec — names match `efe_sdk_*` |
| **3 Spring Boot examples** | MVC + WebFlux + Actuator | Each example builds + runs |
| **Mock for unit tests** | `MockEfeClient` class + `setVerdict/setError/getCalls` | JUnit |
| **Proto codegen** | `mvn generate-sources` produces `target/generated-sources/protobuf/{java,grpc-java}/` idempotently | CI step |
| **JAR shading** | `grpc-netty-shaded` already pre-shaded ; SDK JAR doesn't re-shade | mvn dependency:tree assertion |
| **Maven Central pipeline** | Signed JARs + sources + javadoc + central-publishing-maven-plugin OK | mvn deploy --dry-run |
| **JPMS module-info** | `module ai.efe.sdk { exports ai.efe.sdk; ... }` + `mvn package` produces modular JAR | CI step |

### §4.2 Hors scope (D26)

* **Spring Boot starter publié** — Reporté D27. En D26, l'example
  `spring-boot-actuator/` démontre le pattern `@AutoConfiguration` mais
  reste un example utilisateur, pas un publishable artifact séparé.
* **Reactor full integration** — Pas d'API qui retourne `Mono<Verdict>`
  / `Flux<StreamItem>` natif. L'example WebFlux fait `Mono.fromFuture(client.classifyAsync(text))`
  pour bridger.
* **GraalVM native image config** — Non-trivial à wire (reflection
  hints pour Netty + protobuf). Reporté D28+.
* **Android target** — Cible JVM server-side uniquement. Pas
  `grpc-android` ni `protoc-gen-grpc-java-lite`.
* **JPMS multi-release JAR** — Cible Java 17+ uniquement. Pas de
  multi-release classes Java 11 / 8 fallback.
* **Custom serialization (Avro/Kryo)** — Wire format reste binary
  proto. Pas d'alternatives JSON/XML/Avro pour le RPC.

### §4.3 Acceptance criteria

```text
M11 v1.0 GA = TOUS les items suivants ✓ :

[A] Surface API publique
  ✓ EfeClient class (try-with-resources friendly via AutoCloseable)
  ✓ EfeClient.Builder static class (canonical Java builder)
  ✓ Verdict record — 10 fields cross-lang pinnés
  ✓ ClientOptions builder
  ✓ 10 sentinel exception classes (extends EfeException)
  ✓ EfeException base + isRetryable static method + errorCode static method
  ✓ ExponentialBackoff class
  ✓ MockEfeClient (in mock sub-package)

[B] Build / proto codegen
  ✓ pom.xml parent + 2 modules (efe-sdk-core + efe-sdk-mock)
  ✓ mvn clean package OK from clean state
  ✓ protobuf-maven-plugin generates target/generated-sources/
  ✓ PROTOS_HASH.txt anti-drift pin tested

[C] Tests
  ✓ JUnit 5 unit tests : ≥ 30 passants
  ✓ JUnit 5 integration tests vs InProcessServer : ≥ 10 passants
  ✓ JaCoCo coverage > 80% lines

[D] Examples (3 sub-projects)
  ✓ spring-boot-mvc : mvn spring-boot:run + curl /classify OK
  ✓ spring-boot-webflux : idem (Mono response)
  ✓ spring-boot-actuator : /actuator/metrics expose efe_sdk_* counters

[E] CI/CD
  ✓ sdk-java-test.yml — matrix Java 17/21 × Linux/macOS/Windows
  ✓ sdk-java-smoke.yml — daily live vs staging
  ✓ sdk-java-publish.yml — auto on tag, signed JARs

[F] Publication
  ✓ mvn deploy --dry-run validates POM + signed JARs (.asc files)
  ✓ Sources JAR + Javadoc JAR generated alongside main JAR
  ✓ central-publishing-maven-plugin staging validation
  ✓ groupId+artifactId immutable post-v1.0.0
```

### §4.4 SLO finaux

| ID | Cible | Mesure |
|----|-------|--------|
| `SLO-JAVA-1` | `EfeClient.builder().build()` cold start < 100 ms | JUnit microbenchmark (post JIT warmup) |
| `SLO-JAVA-2` | `classify` p99 ≤ server p99 + 50 ms overhead | JUnit integration |
| `SLO-JAVA-3` | Core JAR < 500 KB ; with grpc-netty-shaded < 5 MB | mvn package size assertion |
| `SLO-JAVA-4` | Java 17+ supported | tests matrix Java 17 + 21 |
| `SLO-JAVA-5` | Coverage ≥ 80% lines | JaCoCo |
| `SLO-JAVA-6` | Maven Central publishable | central-publishing-maven-plugin staging OK |
| `SLO-JAVA-7` | Javadoc clean (0 warning) | mvn javadoc:javadoc |
| `SLO-JAVA-8` | JPMS module-info clean | mvn package --enable-preview |

---

## §5. Principes directeurs (idiomes Java)

### §5.1 Six guiding principles

1. **Type safety + null safety end-to-end** — Java 17+ records pour
   les POJOs immuables, JSpecify `@Nullable` annotations sur les API
   nullables, `Optional<T>` pour les retour optionnels (pas de `null`
   silently retourné).

2. **AutoCloseable + try-with-resources** — `EfeClient` implements
   `AutoCloseable`. Idiom canonique :
   ```java
   try (var client = EfeClient.builder().endpoint("...").build()) {
       Verdict v = client.classify("text");
   }
   ```

3. **Builder pattern partout** — Pour `EfeClient`, `ClientOptions`,
   `RetryConfig` : pattern Java canonique. Évite les constructors
   poly-arg et permet d'évoluer la surface sans bumps majeurs.

4. **Both sync + async API** — `classify(text)` returns `Verdict`
   (blocking) ; `classifyAsync(text)` returns `CompletableFuture<Verdict>`.
   Les deux compilées vers le même bytecode utilisent les mêmes
   gRPC stubs (`BlockingStub` + `FutureStub`).

5. **Soft deps for observability** — OTel + Prom sont des dépendances
   `<scope>provided</scope>` (équivalent peerDep TS). Le SDK détecte
   leur présence via `Class.forName(...)` + reflection lazy. Si
   absent, no-op providers (zero overhead).

6. **JAR size discipline** — Core JAR (sans transitive Netty etc.)
   doit rester < 500 KB. Le SDK n'embarque PAS Netty (`grpc-netty-shaded`
   est une dep transitive du consumer). Le shading est laissé à
   `grpc-netty-shaded` lui-même.

### §5.2 Anti-patterns explicites

| Anti-pattern | Pourquoi NON | Alternative Java |
|---|---|---|
| Public mutable POJOs (setters/getters) | Mutable state, équal/hashcode broken | `record Verdict(...)` immuable |
| `null` retourné comme valeur "absente" | NullPointerException latent | `Optional<T>` pour le optionnel |
| Checked exceptions (`throws IOException`) sur l'API publique | Pollue la signature des callers | Unchecked `RuntimeException` subclasses |
| Constructor à 10 args positionnels | Lisibilité, refactor difficile | Builder pattern |
| Static singleton pour le client | Tests difficiles, lifecycle non géré | Constructed instance, AutoCloseable |
| `Thread.sleep` dans le retry | Bloque le caller thread pour la durée du backoff | `CompletableFuture` chained delay |
| Logging via `System.out.println` | Pas de framework, pas de levels | `slf4j` API + bind par l'app |
| `Object.toString()` qui leak des secrets | JWT visible dans les stack traces | `toString()` redacted, jamais inclut auth |
| `Synchronized` blocks sur des HashMaps | Lock contention, non scalable | `ConcurrentHashMap` natif |
| `extends Thread` au lieu de `implements Runnable` / Executor | Anti-pattern depuis Java 5 | `ExecutorService` + `submit(...)` |

### §5.3 Idiomes Java canoniques choisis

* **`record Verdict(String label, double confidence, ...)`** — Java
  17 record produit automatiquement constructor + accessors +
  equals/hashCode/toString immuables.
* **`EfeClient.Builder`** static nested class — pattern canonique
  Java pour API config.
* **`AutoCloseable`** + `try-with-resources` — gestion lifecycle
  ressources.
* **`CompletableFuture<T>`** pour async — composable, idiomatic
  Java 8+.
* **`Stream<T>`** + `Collectors` pour transformations batch (avec
  `parallelStream()` quand utile).
* **`@Nullable` JSpecify** — annotation canonique 2024+ (vs
  `@javax.annotation.Nullable` deprecated, `@org.checkerframework.*`
  trop verbose).
* **`Optional<T>`** pour les retours optionnels.
* **`Duration`** + `Instant` pour les timeouts/deadlines (vs `long
  millis` legacy).
* **slf4j-api** (sans binding hardcoded) pour le logging — l'app
  binde Logback/Log4j2/etc.

### §5.4 Decision : Maven vs Gradle

**Décision** : Maven.

**Rationale** :
* **Maven** : standard de facto pour les libs publiées Maven Central.
  POM declarative + plugin ecosystem mature pour la signature, le
  staging, le javadoc, le sources JAR. La majorité des libs Java
  enterprise sont en Maven.
* **Gradle** : excellent pour les apps internes, le multi-project
  build complexe. Moins canonical pour les libs publiées Maven
  Central — la communauté pousse à fournir le `gradle-publish-plugin`
  qui interopère avec Maven Central, mais rajoute une couche.
* Pour M11, la priorité est la **publication** facile + reproductible.
  Maven gagne.

**Pin test** : `test_d26_uses_maven` vérifie l'existence des `pom.xml`
+ l'absence de `build.gradle*`.

### §5.5 Decision : `grpc-netty-shaded` vs `grpc-netty` vs `grpc-okhttp`

**Décision** : `io.grpc:grpc-netty-shaded` (canonical pour la prod
Java).

**Rationale** :
* **`grpc-netty-shaded`** : embarque Netty + boringssl + ALPN auto-config.
  Zero dependency conflict avec d'autres versions de Netty dans
  l'app consumer. Recommandé par grpc-java team pour la prod.
* **`grpc-netty`** : non-shaded, requiert que l'app fournisse Netty
  ALPN compatible. Fragile en présence d'autres libs (Spring Boot,
  Apache HttpClient, etc.) qui pull différentes versions Netty.
* **`grpc-okhttp`** : transport via OkHttp, plus léger mais moins
  performant (HTTP/1.1 frame-by-frame ; HTTP/2 partiellement supporté).
  Bon pour Android, sous-optimal pour serveur enterprise.

**Pin test** : `test_d26_uses_grpc_netty_shaded` vérifie le `<dependency>`
canonique dans `efe-sdk-core/pom.xml`.

### §5.6 Decision : JUnit 5 (Jupiter) vs JUnit 4 vs TestNG

**Décision** : JUnit 5 (Jupiter).

**Rationale** :
* **JUnit 5 (Jupiter)** : standard Java testing 2024+. `@DisplayName`,
  `@ParameterizedTest`, `@Nested`, `@RepeatedTest` modernisent les
  tests. AssertJ + Mockito s'intègrent natively. Le user a explicitement
  demandé JUnit 5.
* **JUnit 4** : legacy, pas de paramétrage natif fluent, deprecated
  en faveur de JUnit 5.
* **TestNG** : excellent mais plus de niche entreprise.

---

## §6. Invariants structurels (layout Maven multi-module)

### §6.1 Filesystem layout (frozen)

```
clients/java/efe-sdk/                              ← parent project
├── pom.xml                                        ← parent POM (packaging=pom)
├── README.md
├── LICENSE                                         ← Apache-2.0
├── CHANGELOG.md
├── .gitignore
├── .mvn/
│   └── jvm.config                                  ← --enable-preview etc.
├── mvnw + mvnw.cmd                                 ← Maven Wrapper (3.9.x)
│
├── efe-sdk-core/                                   ← module 1 : main SDK
│   ├── pom.xml
│   ├── src/main/java/ai/efe/sdk/
│   │   ├── EfeClient.java                          ← public API
│   │   ├── ClientOptions.java                      ← Builder
│   │   ├── Verdict.java                            ← record
│   │   ├── StreamItem.java                         ← record
│   │   ├── Version.java                            ← public const
│   │   ├── exception/
│   │   │   ├── EfeException.java                   ← base RuntimeException
│   │   │   ├── EfeUnauthenticatedException.java
│   │   │   ├── EfePermissionDeniedException.java
│   │   │   ├── EfeCatalogNotFoundException.java
│   │   │   ├── EfeCatalogInvalidException.java
│   │   │   ├── EfeTimeoutException.java
│   │   │   ├── EfeUnavailableException.java
│   │   │   ├── EfeRateLimitedException.java
│   │   │   ├── EfeInvalidArgumentException.java
│   │   │   ├── EfeInternalException.java
│   │   │   └── EfeClosedException.java
│   │   ├── auth/
│   │   │   ├── AuthConfig.java                     ← sealed interface
│   │   │   ├── JwtAuth.java                        ← record (sealed permits)
│   │   │   ├── MtlsAuth.java                       ← record
│   │   │   ├── NoAuth.java                         ← singleton
│   │   │   └── JwtCallCredentials.java             ← gRPC CallCredentials impl
│   │   ├── retry/
│   │   │   ├── RetryConfig.java
│   │   │   ├── RetryBackoff.java                   ← interface
│   │   │   ├── ExponentialBackoff.java
│   │   │   └── RetryClientInterceptor.java
│   │   ├── otel/
│   │   │   └── OtelClientInterceptor.java
│   │   ├── prometheus/
│   │   │   ├── SdkMetrics.java
│   │   │   └── PrometheusClientInterceptor.java
│   │   └── internal/
│   │       ├── ErrorMapper.java                    ← Status → exception
│   │       ├── BlockingClient.java                 ← gRPC stub wrapper
│   │       └── ChannelFactory.java                 ← ManagedChannel builder
│   ├── src/main/java/module-info.java              ← JPMS module
│   ├── src/main/proto/                             ← copy/symlink protos/efe/v1/
│   │   └── efe/v1/*.proto
│   ├── src/main/resources/
│   │   └── META-INF/
│   │       └── services/                            ← SPI hooks if needed
│   ├── src/test/java/ai/efe/sdk/
│   │   ├── EfeClientTest.java
│   │   ├── VerdictTest.java
│   │   ├── ExceptionTest.java
│   │   ├── ClientOptionsTest.java
│   │   ├── retry/
│   │   │   ├── ExponentialBackoffTest.java
│   │   │   └── RetryClientInterceptorTest.java
│   │   ├── auth/
│   │   │   └── JwtCallCredentialsTest.java
│   │   ├── prometheus/
│   │   │   └── SdkMetricsTest.java
│   │   └── integration/
│   │       ├── ClassifyIntegrationTest.java
│   │       ├── HealthcheckIntegrationTest.java
│   │       ├── StreamIntegrationTest.java
│   │       └── RetryIntegrationTest.java
│   └── src/test/resources/
│       ├── certs/                                   ← test mTLS material
│       └── logback-test.xml
│
├── efe-sdk-mock/                                   ← module 2 : test fixtures
│   ├── pom.xml
│   └── src/main/java/ai/efe/sdk/mock/
│       ├── MockEfeClient.java
│       ├── MockEfeServer.java                      ← InProcessServerBuilder wrapper
│       └── VerdictFactory.java
│
├── examples/                                        ← NOT part of parent build
│   ├── spring-boot-mvc/                            ← own pom.xml, own version
│   │   ├── pom.xml
│   │   ├── src/main/java/com/example/efe/
│   │   │   ├── Application.java
│   │   │   ├── config/EfeConfig.java
│   │   │   └── controller/ClassifyController.java
│   │   └── src/main/resources/
│   │       └── application.yml
│   ├── spring-boot-webflux/
│   │   ├── pom.xml
│   │   └── src/main/java/...
│   └── spring-boot-actuator/
│       ├── pom.xml
│       └── src/main/java/...
│
└── scripts/
    ├── gen-proto.sh                                 ← invokes protobuf-maven-plugin
    ├── publish.sh                                   ← mvn deploy + GPG
    ├── lint.sh                                      ← mvn checkstyle:check + spotbugs:check
    └── e2e-bootstrap.sh                             ← spawn InProcessServer + run tests
```

### §6.2 Layout invariants

* **`efe-sdk-core/` ne dépend PAS de `efe-sdk-mock/`** — pin test
  `test_d26_core_does_not_depend_on_mock`. Le mock est strictly
  test-time.
* **`efe-sdk-mock/` dépend de `efe-sdk-core/`** — c'est le sens
  attendu (mock implémente les interfaces du core).
* **`examples/` ne fait PAS partie du build parent** — chacun a son
  propre POM (`<parent>` absent ou pointe directement sur Spring Boot).
  Évite que `mvn package` du parent build les exemples (qui pull
  Spring Boot ~80 MB transitive).
* **`src/main/proto/`** est le path canonique attendu par le
  `protobuf-maven-plugin`. Les `.proto` sont COPIÉS depuis `protos/efe/v1/`
  (ou symlinkés). Le pin test SHA-256 anchor le sync.
* **`module-info.java`** est commité. Définit l'export public du SDK
  + les dépendances JPMS.

### §6.3 `pom.xml` parent (extrait canonical)

```xml
<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>

  <groupId>ai.efe</groupId>
  <artifactId>efe-sdk-java-parent</artifactId>
  <version>0.4.0</version>
  <packaging>pom</packaging>

  <name>EFE SDK Java Parent</name>
  <description>Parent POM for the EFE Java SDK + mock test fixtures.</description>
  <url>https://github.com/efe-ai/efe</url>

  <licenses>
    <license>
      <name>Apache License, Version 2.0</name>
      <url>https://www.apache.org/licenses/LICENSE-2.0.txt</url>
      <distribution>repo</distribution>
    </license>
  </licenses>

  <developers>
    <developer>
      <id>rodrigue.pekar</id>
      <name>Rodrigue Pekar</name>
      <email>rodrigue.pekar@icloud.com</email>
      <organization>EFE Team</organization>
    </developer>
  </developers>

  <scm>
    <connection>scm:git:git://github.com/efe-ai/efe.git</connection>
    <developerConnection>scm:git:ssh://git@github.com/efe-ai/efe.git</developerConnection>
    <url>https://github.com/efe-ai/efe/tree/main/clients/java/efe-sdk</url>
    <tag>HEAD</tag>
  </scm>

  <modules>
    <module>efe-sdk-core</module>
    <module>efe-sdk-mock</module>
  </modules>

  <properties>
    <maven.compiler.release>17</maven.compiler.release>
    <project.build.sourceEncoding>UTF-8</project.build.sourceEncoding>
    <grpc.version>1.64.0</grpc.version>
    <protobuf.version>3.25.3</protobuf.version>
    <opentelemetry.version>1.38.0</opentelemetry.version>
    <prometheus.version>1.3.1</prometheus.version>
    <slf4j.version>2.0.13</slf4j.version>
    <junit.version>5.10.2</junit.version>
    <assertj.version>3.26.0</assertj.version>
    <mockito.version>5.12.0</mockito.version>
  </properties>

  <dependencyManagement>
    <dependencies>
      <dependency>
        <groupId>io.grpc</groupId>
        <artifactId>grpc-bom</artifactId>
        <version>${grpc.version}</version>
        <type>pom</type>
        <scope>import</scope>
      </dependency>
      <!-- ... -->
    </dependencies>
  </dependencyManagement>

  <build>
    <pluginManagement>
      <plugins>
        <plugin>
          <groupId>org.apache.maven.plugins</groupId>
          <artifactId>maven-compiler-plugin</artifactId>
          <version>3.13.0</version>
        </plugin>
        <plugin>
          <groupId>org.apache.maven.plugins</groupId>
          <artifactId>maven-source-plugin</artifactId>
          <version>3.3.1</version>
        </plugin>
        <plugin>
          <groupId>org.apache.maven.plugins</groupId>
          <artifactId>maven-javadoc-plugin</artifactId>
          <version>3.7.0</version>
        </plugin>
        <plugin>
          <groupId>org.apache.maven.plugins</groupId>
          <artifactId>maven-gpg-plugin</artifactId>
          <version>3.2.4</version>
        </plugin>
        <plugin>
          <groupId>org.sonatype.central</groupId>
          <artifactId>central-publishing-maven-plugin</artifactId>
          <version>0.5.0</version>
        </plugin>
        <plugin>
          <groupId>org.jacoco</groupId>
          <artifactId>jacoco-maven-plugin</artifactId>
          <version>0.8.12</version>
        </plugin>
        <plugin>
          <groupId>org.xolstice.maven.plugins</groupId>
          <artifactId>protobuf-maven-plugin</artifactId>
          <version>0.6.1</version>
        </plugin>
      </plugins>
    </pluginManagement>
  </build>
</project>
```

### §6.4 `efe-sdk-core/pom.xml` (extrait)

```xml
<dependencies>
  <dependency>
    <groupId>io.grpc</groupId>
    <artifactId>grpc-netty-shaded</artifactId>
    <version>${grpc.version}</version>
  </dependency>
  <dependency>
    <groupId>io.grpc</groupId>
    <artifactId>grpc-protobuf</artifactId>
  </dependency>
  <dependency>
    <groupId>io.grpc</groupId>
    <artifactId>grpc-stub</artifactId>
  </dependency>
  <dependency>
    <groupId>io.grpc</groupId>
    <artifactId>grpc-services</artifactId>
  </dependency>
  <dependency>
    <groupId>com.google.protobuf</groupId>
    <artifactId>protobuf-java</artifactId>
    <version>${protobuf.version}</version>
  </dependency>

  <!-- Soft deps : observability (provided scope = peerDep equivalent) -->
  <dependency>
    <groupId>io.opentelemetry</groupId>
    <artifactId>opentelemetry-api</artifactId>
    <version>${opentelemetry.version}</version>
    <scope>provided</scope>
    <optional>true</optional>
  </dependency>
  <dependency>
    <groupId>io.prometheus</groupId>
    <artifactId>prometheus-metrics-core</artifactId>
    <version>${prometheus.version}</version>
    <scope>provided</scope>
    <optional>true</optional>
  </dependency>
  <dependency>
    <groupId>org.slf4j</groupId>
    <artifactId>slf4j-api</artifactId>
    <version>${slf4j.version}</version>
  </dependency>

  <!-- JSpecify nullness annotations -->
  <dependency>
    <groupId>org.jspecify</groupId>
    <artifactId>jspecify</artifactId>
    <version>1.0.0</version>
  </dependency>

  <!-- Test scope -->
  <dependency>
    <groupId>org.junit.jupiter</groupId>
    <artifactId>junit-jupiter</artifactId>
    <version>${junit.version}</version>
    <scope>test</scope>
  </dependency>
  <dependency>
    <groupId>org.assertj</groupId>
    <artifactId>assertj-core</artifactId>
    <version>${assertj.version}</version>
    <scope>test</scope>
  </dependency>
  <dependency>
    <groupId>org.mockito</groupId>
    <artifactId>mockito-core</artifactId>
    <version>${mockito.version}</version>
    <scope>test</scope>
  </dependency>
  <dependency>
    <groupId>io.grpc</groupId>
    <artifactId>grpc-testing</artifactId>
    <scope>test</scope>
  </dependency>
</dependencies>

<build>
  <extensions>
    <extension>
      <groupId>kr.motd.maven</groupId>
      <artifactId>os-maven-plugin</artifactId>
      <version>1.7.1</version>
    </extension>
  </extensions>
  <plugins>
    <plugin>
      <groupId>org.xolstice.maven.plugins</groupId>
      <artifactId>protobuf-maven-plugin</artifactId>
      <configuration>
        <protocArtifact>com.google.protobuf:protoc:${protobuf.version}:exe:${os.detected.classifier}</protocArtifact>
        <pluginId>grpc-java</pluginId>
        <pluginArtifact>io.grpc:protoc-gen-grpc-java:${grpc.version}:exe:${os.detected.classifier}</pluginArtifact>
      </configuration>
      <executions>
        <execution>
          <goals>
            <goal>compile</goal>
            <goal>compile-custom</goal>
          </goals>
        </execution>
      </executions>
    </plugin>
  </plugins>
</build>
```

### §6.5 Decision : sealed interfaces + records (Java 17+)

Le SDK exploite les sealed interfaces (Java 17 stable) :

```java
public sealed interface AuthConfig
        permits JwtAuth, MtlsAuth, NoAuth {}

public record JwtAuth(String token) implements AuthConfig { ... }
public record MtlsAuth(Path certPath, Path keyPath, Path caPath) implements AuthConfig { ... }
public final class NoAuth implements AuthConfig {
    public static final NoAuth INSTANCE = new NoAuth();
    private NoAuth() {}
}
```

**Avantages** :
* `switch (authConfig)` exhaustif au compile time (pattern matching
  Java 21+).
* IDE autocomplete + javadoc clean.
* Records auto-génèrent `equals`/`hashCode`/`toString`.

**Pin test** : `test_d26_auth_config_sealed` vérifie `sealed interface`
+ `permits` + 3 implementations.

---

## §7. Vue d'ensemble — sprint D26 unique

### §7.1 Récapitulatif du livrable

| Axe | Spec | Volumétrie |
|---|---|---|
| Module Maven publishable | `ai.efe:efe-sdk-java:0.4.0` private (Φ-B) | 1 module + 1 mock submodule |
| Source Java (core) | `efe-sdk-core/src/main/java/` | ~3 500 LOC |
| Generated (proto stubs) | `target/generated-sources/protobuf/{java,grpc-java}/` | ~2 500 LOC (auto) |
| Source Java (mock) | `efe-sdk-mock/src/main/java/` | ~400 LOC |
| Tests JUnit 5 (unit + integration) | `efe-sdk-core/src/test/java/` | ~1 500 LOC |
| Examples Spring Boot | 3 sub-projects | ~700 LOC |
| Scripts shell | gen-proto, publish, lint, e2e-bootstrap | ~280 LOC |
| CI workflows | sdk-java-test + sdk-java-smoke + sdk-java-publish | ~450 LOC YAML |
| Pin tests Python | ~32 tests anti-régression | ~750 LOC |
| Documentation | README + Javadoc inline | ~300 LOC |
| **Total** | **~10 000 LOC** (~3 900 hand-coded Java + 2 500 generated + 1 500 tests + 700 examples + 1 480 build/CI/docs) | |

### §7.2 Phasage J1 → J10

| Jour | Goal | Output |
|---|---|---|
| **J1** | Bootstrap Maven multi-module + parent POM + protobuf-maven-plugin | parent `pom.xml` + `efe-sdk-core/pom.xml` + `efe-sdk-mock/pom.xml` ; `mvn validate` OK ; `mvn generate-sources` produces `target/generated-sources/` |
| **J2** | Core types : Verdict (record) + 10 EfeException sentinels + ClientOptions Builder + Version | `src/main/java/ai/efe/sdk/{Verdict,Version,ClientOptions}.java` + `exception/` + 6 unit tests JUnit 5 |
| **J3** | EfeClient class + ChannelFactory + happy-path | `EfeClient.java` (sync API) + `ChannelFactory.java` ; `MockEfeServer` (InProcessServerBuilder) + 1 integration test "happy path" |
| **J4** | Auth (JWT CallCredentials + MtlsBuilder) + RetryClientInterceptor + ExponentialBackoff | `auth/` package + `retry/` package + tests |
| **J5** | OTel + Prometheus + mock module | `otel/OtelClientInterceptor.java` + `prometheus/SdkMetrics.java` + `efe-sdk-mock/` module avec `MockEfeClient` + `VerdictFactory` |
| **J6** | Async API (CompletableFuture) + classifyStream | `EfeClient.classifyAsync` + `EfeClient.classifyStream` (worker pool ExecutorService + dispatcher pattern à la M9-A3) |
| **J7** | 3 Spring Boot examples : MVC + WebFlux + Actuator | `examples/{spring-boot-mvc,spring-boot-webflux,spring-boot-actuator}/` ; chaque example `mvn spring-boot:run` + curl OK |
| **J8** | Tests JUnit massifs + JaCoCo coverage | ≥ 30 unit + ≥ 10 integration ; coverage > 80% lines |
| **J9** | Pin tests Python + scripts/ + CI workflows | `scripts/tests/test_d26_java_sdk_extension.py` ; 3 workflows YAML ; signed JARs validation |
| **J10** | Audit pass 1 + CHANGELOG + Maven Central staging dry-run | `mvn deploy -DskipTests --dry-run` validates POM + signed JARs ; CHANGELOG entry M11 |

### §7.3 Risques sprint-level

| Risque | Probabilité | Impact | Mitigation |
|---|:---:|:---:|---|
| `protobuf-maven-plugin` toolchain instable / OS-specific | M | M | Pin version `0.6.1` ; `os-maven-plugin` extension détecte le bon classifier ; fallback hand-coded stubs (à la M9 stratégie) |
| Maven Central staging credentials manquantes | M | H | Provision en J1 via secret repo (USERNAME + PASSWORD + GPG key) ; document dans README |
| GPG signature CI flakiness | M | M | Skip signing in PR builds ; sign only sur tag publish workflow |
| `grpc-netty-shaded` JAR explose le bundle (~5 MB) | L | L | Documenté dans SLO-JAVA-3 ; consumers conscients ; alternative `grpc-okhttp` mentionnée pour Android |
| Spring Boot 3.x incompatibility | L | M | Pin Spring Boot 3.3.x (LTS) ; tests CI valident |
| JPMS module-info conflicts | M | M | Use `requires transitive` strategically ; tests CI valident `mvn package` produces modular JAR |
| Java 17 + 21 matrix mismatch | L | L | CI matrix tests both ; SDK targets 17 (release flag) — Java 21 runtime should still load |

---

## §8. Surface publique cible (API Java publishable)

### §8.1 Public API (`ai.efe.sdk` package)

```java
package ai.efe.sdk;

// Public types (28 public exports — equivalent to TS M10 surface)

// 1. Main client
public final class EfeClient implements AutoCloseable {
    public static Builder builder() { ... }

    public Verdict classify(String text) throws EfeException;
    public CompletableFuture<Verdict> classifyAsync(String text);
    public Stream<StreamItem> classifyStream(Iterable<String> texts);
    public void healthcheck() throws EfeException;
    @Override public void close();
    public boolean isClosed();

    public static final class Builder {
        public Builder endpoint(String endpoint) { ... }
        public Builder auth(AuthConfig auth) { ... }
        public Builder retry(RetryConfig retry) { ... }
        public Builder otel(io.opentelemetry.api.OpenTelemetry otel) { ... }
        public Builder prometheus(io.prometheus.metrics.model.registry.PrometheusRegistry reg) { ... }
        public Builder deadline(java.time.Duration deadline) { ... }
        public Builder userAgent(String ua) { ... }
        public Builder maxReceivedMessageSize(int bytes) { ... }
        public EfeClient build();
    }
}

// 2. Verdict (record)
public record Verdict(
    String label,
    double confidence,
    boolean reject,
    int rounds,
    double alignmentDelta,
    String catalog,
    String zone,
    String swarmId,
    String requestId,
    long elapsedMs
) {
    public boolean isTrusted() { return !reject && "trusted".equals(zone); }
    public boolean isWarn() { return !reject && "warn".equals(zone); }
    public boolean isReject() { return reject || "reject".equals(zone); }
}

// 3. StreamItem (record)
public record StreamItem(int index, @Nullable Verdict verdict, @Nullable Throwable error) {}

// 4. ClientOptions (Builder pattern, internal — exposed via EfeClient.Builder)

// 5. AuthConfig + 3 implementations
public sealed interface AuthConfig permits JwtAuth, MtlsAuth, NoAuth {}
public record JwtAuth(String token) implements AuthConfig {
    public static JwtAuth of(String token) { return new JwtAuth(token); }
}
public record MtlsAuth(Path certPath, Path keyPath, Path caPath) implements AuthConfig {
    public static MtlsAuth fromFiles(Path cert, Path key, Path ca) { return new MtlsAuth(cert, key, ca); }
}
public final class NoAuth implements AuthConfig {
    public static final NoAuth INSTANCE = new NoAuth();
    private NoAuth() {}
}

// 6. RetryConfig + ExponentialBackoff
public record RetryConfig(int maxAttempts, RetryBackoff backoff) {
    public static RetryConfig exponential(int maxAttempts, Duration initial, Duration max) {
        return new RetryConfig(maxAttempts, new ExponentialBackoff(initial, max, 2.0));
    }
}
public interface RetryBackoff {
    Duration next(int attempt);
}
public final class ExponentialBackoff implements RetryBackoff {
    public ExponentialBackoff(Duration initial, Duration max, double multiplier) { ... }
    @Override public Duration next(int attempt) { ... }
}

// 7. Exception hierarchy
package ai.efe.sdk.exception;
public class EfeException extends RuntimeException { ... }
public final class EfeUnauthenticatedException extends EfeException { ... }
public final class EfePermissionDeniedException extends EfeException { ... }
public final class EfeCatalogNotFoundException extends EfeException { ... }
public final class EfeCatalogInvalidException extends EfeException { ... }
public final class EfeTimeoutException extends EfeException { ... }
public final class EfeUnavailableException extends EfeException { ... }
public final class EfeRateLimitedException extends EfeException { ... }
public final class EfeInvalidArgumentException extends EfeException { ... }
public final class EfeInternalException extends EfeException { ... }
public final class EfeClosedException extends EfeException { ... }

// 8. Static helpers
public final class EfeException {
    public static boolean isRetryable(Throwable err) { ... }
    public static String errorCode(Throwable err) { ... }
}

// 9. Version
public final class Version {
    public static final String VERSION = "0.4.0";
    private Version() {}
}
```

### §8.2 Mock package (`ai.efe.sdk.mock`)

```java
package ai.efe.sdk.mock;

public final class MockEfeClient implements EfeClient.Mockable {
    public void setVerdict(Verdict v) { ... }
    public void setError(Throwable err) { ... }
    public List<String> getCalls() { ... }
    public int getCallCount() { ... }
    public void reset() { ... }
    // implements all EfeClient public methods
}

public final class MockEfeServer implements AutoCloseable {
    public static MockEfeServer create() throws IOException { ... }
    public String getEndpoint() { ... }
    public void onClassify(Function<ExecuteRequest, ExecuteResponse> handler) { ... }
    @Override public void close() { ... }
}

public final class VerdictFactory {
    public static Verdict newVerdict() { ... }
    public static Verdict newVerdict(VerdictOverrides overrides) { ... }
    public record VerdictOverrides(...) { ... }
}
```

### §8.3 Total public surface

| Export | Type | Stable | Pin tested |
|---|---|---|---|
| `EfeClient` | final class (AutoCloseable) | yes | ✓ DT-J-1 |
| `EfeClient.Builder` | static nested class | yes | ✓ DT-J-1 |
| `Verdict` | record (10 fields) | yes — major bump to add field | ✓ DT-J-2 |
| `StreamItem` | record | yes | ✓ DT-J-2 |
| `AuthConfig` | sealed interface | yes | ✓ DT-J-3 |
| `JwtAuth` / `MtlsAuth` / `NoAuth` | record / record / singleton | yes | ✓ DT-J-3 |
| `RetryConfig` | record | yes | ✓ DT-J-3 |
| `RetryBackoff` | interface | yes | ✓ DT-J-3 |
| `ExponentialBackoff` | final class | yes | ✓ DT-J-3 |
| `EfeException` | base RuntimeException | yes | ✓ DT-J-4 |
| 10 sentinel subclasses | final class extends EfeException | yes | ✓ DT-J-4 |
| `EfeException.isRetryable(err)` | static method | yes | ✓ DT-J-4 |
| `EfeException.errorCode(err)` | static method | yes | ✓ DT-J-4 |
| `Version.VERSION` | constant string | yes | ✓ DT-J-5 |
| `MockEfeClient` | final class (mock package) | yes | ✓ DT-J-6 |
| `MockEfeServer` | final class | yes | ✓ DT-J-6 |
| `VerdictFactory.newVerdict(...)` | static method | yes | ✓ DT-J-6 |

**Total** : 28+ public exports ; 6 pin test groups DT-J-1..6.

---

## §9. Sprint D26 — module Maven entier

### §9.1 Definition of Done — sprint-level

```text
Sprint D26 = DONE quand :

  ✓ mvn clean install OK from clean state
  ✓ mvn generate-sources idempotent (PROTOS_HASH.txt anchored)
  ✓ mvn package produces 4 JARs/module :
    - efe-sdk-core-0.4.0.jar (main)
    - efe-sdk-core-0.4.0-sources.jar
    - efe-sdk-core-0.4.0-javadoc.jar
    - efe-sdk-core-0.4.0.pom (after deploy phase)
  ✓ mvn test (JUnit5 unit + integration) GREEN, > 80% coverage (JaCoCo)
  ✓ mvn javadoc:javadoc 0 warnings
  ✓ mvn spotbugs:check + checkstyle:check 0 errors
  ✓ mvn package produces JPMS-modular JAR (module-info.class in JAR root)
  ✓ Examples : MVC + WebFlux + Actuator tous compile + run
  ✓ Pin tests Python (~32) GREEN
  ✓ CI workflows green sur PR test
  ✓ mvn deploy --dry-run validates Maven Central staging
  ✓ All 4 artifacts signed (efe-sdk-core-0.4.0.jar.asc etc.)
  ✓ CHANGELOG.md entry M11 D26 [Unreleased]
  ✓ Architecture doc (CE FICHIER) committed avant code
```

### §9.2 Output artefacts cibles (publishable)

```
target/efe-sdk-core/
├── efe-sdk-core-0.4.0.jar              (main, ~480 KB)
├── efe-sdk-core-0.4.0.jar.asc          (GPG signature)
├── efe-sdk-core-0.4.0-sources.jar      (sources, ~280 KB)
├── efe-sdk-core-0.4.0-sources.jar.asc
├── efe-sdk-core-0.4.0-javadoc.jar      (javadoc, ~720 KB)
├── efe-sdk-core-0.4.0-javadoc.jar.asc
├── efe-sdk-core-0.4.0.pom              (POM)
└── efe-sdk-core-0.4.0.pom.asc

target/efe-sdk-mock/
├── efe-sdk-mock-0.4.0.jar              (mock, ~120 KB)
├── efe-sdk-mock-0.4.0.jar.asc
├── efe-sdk-mock-0.4.0-sources.jar
├── efe-sdk-mock-0.4.0-sources.jar.asc
├── efe-sdk-mock-0.4.0-javadoc.jar
├── efe-sdk-mock-0.4.0-javadoc.jar.asc
├── efe-sdk-mock-0.4.0.pom
└── efe-sdk-mock-0.4.0.pom.asc
```

---

## §10. Sub-système 1 — `EfeClient` + Builder + AutoCloseable

### §10.1 EfeClient — full surface

```java
package ai.efe.sdk;

import io.grpc.ManagedChannel;
import io.grpc.StatusRuntimeException;
import org.jspecify.annotations.NonNull;
import org.jspecify.annotations.Nullable;

import java.time.Duration;
import java.util.concurrent.CompletableFuture;
import java.util.stream.Stream;

import ai.efe.sdk.exception.*;
import ai.efe.sdk.internal.ChannelFactory;
import ai.efe.sdk.internal.ErrorMapper;

/**
 * EfeClient — main entry point of the EFE Java SDK.
 *
 * <p>Construct via the builder pattern :
 * <pre>{@code
 * try (var client = EfeClient.builder()
 *         .endpoint("staging.efe.ai:50051")
 *         .auth(JwtAuth.of(System.getenv("EFE_JWT_TOKEN")))
 *         .deadline(Duration.ofSeconds(10))
 *         .build()) {
 *     Verdict verdict = client.classify("I want a refund");
 *     if (verdict.isTrusted()) {
 *         // ...
 *     }
 * }
 * }</pre>
 *
 * <p>Lifecycle :
 * <ul>
 *   <li>{@code builder().build()} → ready (lazy gRPC dial — no I/O yet).
 *   <li>{@code classify}, {@code classifyAsync}, {@code healthcheck} —
 *       concurrent OK.
 *   <li>{@code close()} → idempotent. Subsequent calls throw
 *       {@link EfeClosedException}.
 * </ul>
 *
 * <p>Concurrency : instances are SAFE for concurrent use across
 * threads. Internally uses a single {@code ManagedChannel} that
 * multiplexes HTTP/2 streams.
 *
 * <p>Pin tested by {@code test_d26_java_sdk_efe_client_surface}.
 *
 * @since 0.4.0
 */
public final class EfeClient implements AutoCloseable {
    private final @NonNull ManagedChannel channel;
    private final @NonNull SwarmServiceGrpc.SwarmServiceBlockingStub blockingStub;
    private final @NonNull SwarmServiceGrpc.SwarmServiceFutureStub futureStub;
    private final @NonNull ClientOptions opts;
    private volatile boolean closed = false;

    private EfeClient(Builder builder) {
        this.opts = builder.options();
        this.channel = ChannelFactory.create(opts);
        this.blockingStub = SwarmServiceGrpc.newBlockingStub(channel);
        this.futureStub = SwarmServiceGrpc.newFutureStub(channel);
    }

    public static Builder builder() {
        return new Builder();
    }

    /**
     * Send one classification request synchronously.
     *
     * @param text input text (non-null, non-blank)
     * @return verdict
     * @throws EfeInvalidArgumentException if text is null or blank
     * @throws EfeUnauthenticatedException on auth failure
     * @throws EfeTimeoutException on deadline exceeded
     * @throws EfeUnavailableException on transient unavailability
     * @throws EfeRateLimitedException on quota exceeded
     * @throws EfeClosedException if {@link #close()} was already called
     */
    public Verdict classify(@NonNull String text) {
        if (closed) throw new EfeClosedException("client is closed");
        if (text == null || text.isBlank()) {
            throw new EfeInvalidArgumentException("text must be non-blank");
        }
        try {
            ExecuteRequest req = ExecuteRequest.newBuilder().setRawPrompt(text).build();
            long start = System.nanoTime();
            ExecuteResponse resp = blockingStub
                .withDeadlineAfter(opts.deadline().toMillis(), TimeUnit.MILLISECONDS)
                .execute(req);
            long elapsedMs = (System.nanoTime() - start) / 1_000_000L;
            return verdictFromExecuteResponse(resp, elapsedMs);
        } catch (StatusRuntimeException e) {
            throw ErrorMapper.fromGrpcStatus(e);
        }
    }

    /**
     * Send one classification request asynchronously.
     *
     * @return CompletableFuture that completes with the Verdict, or
     *         exceptionally with one of the {@link EfeException} subclasses.
     */
    public CompletableFuture<Verdict> classifyAsync(@NonNull String text) {
        if (closed) {
            return CompletableFuture.failedFuture(
                new EfeClosedException("client is closed"));
        }
        if (text == null || text.isBlank()) {
            return CompletableFuture.failedFuture(
                new EfeInvalidArgumentException("text must be non-blank"));
        }
        ExecuteRequest req = ExecuteRequest.newBuilder().setRawPrompt(text).build();
        long start = System.nanoTime();
        ListenableFuture<ExecuteResponse> grpcFuture = futureStub
            .withDeadlineAfter(opts.deadline().toMillis(), TimeUnit.MILLISECONDS)
            .execute(req);
        return toCompletableFuture(grpcFuture)
            .thenApply(resp -> verdictFromExecuteResponse(
                resp, (System.nanoTime() - start) / 1_000_000L))
            .exceptionally(t -> {
                if (t.getCause() instanceof StatusRuntimeException sre) {
                    throw ErrorMapper.fromGrpcStatus(sre);
                }
                throw (t instanceof RuntimeException rte) ? rte : new RuntimeException(t);
            });
    }

    /**
     * Concurrent batch classification with input-order index preservation.
     *
     * <p>Architecture mirrors the M9-A3 Go SDK + M10-A5/A6 TS SDK fix :
     * single dispatcher reads {@code texts}, assigns monotonic indices,
     * fans out to N worker threads (default 8). The orchestrator
     * collects results into a {@code Stream} that emits in
     * non-deterministic order — the {@link StreamItem#index()} reflects
     * the input position.
     *
     * @param texts input source (non-null)
     * @return Stream of StreamItems
     */
    public Stream<StreamItem> classifyStream(@NonNull Iterable<String> texts) {
        // Implementation uses ExecutorService + BlockingQueue.
        // See StreamImpl for details.
        return StreamImpl.run(this, texts, opts.streamConcurrency());
    }

    /**
     * Probe the server's gRPC standard Health endpoint.
     *
     * @throws EfeUnavailableException on NOT_SERVING or unreachable
     * @throws EfeTimeoutException on deadline exceeded
     */
    public void healthcheck() {
        // Use grpc.health.v1.Health stub.
        // ...
    }

    /**
     * Release the underlying gRPC connection. Idempotent.
     *
     * <p>After close, subsequent classify/healthcheck calls throw
     * {@link EfeClosedException}.
     */
    @Override
    public void close() {
        if (closed) return;
        closed = true;
        channel.shutdown();
        try {
            if (!channel.awaitTermination(5, TimeUnit.SECONDS)) {
                channel.shutdownNow();
            }
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            channel.shutdownNow();
        }
    }

    public boolean isClosed() {
        return closed;
    }

    @Override
    public String toString() {
        return "EfeClient{endpoint=" + opts.endpoint() + " closed=" + closed + "}";
        // NEVER include JWT token.
    }

    // ─── Builder ───
    public static final class Builder {
        private @Nullable String endpoint;
        private @NonNull AuthConfig auth = NoAuth.INSTANCE;
        private @NonNull RetryConfig retry = RetryConfig.exponential(3,
            Duration.ofMillis(100), Duration.ofSeconds(30));
        private @NonNull Duration deadline = Duration.ofSeconds(10);
        private @NonNull String userAgent = "efe-sdk-java/" + Version.VERSION;
        private int maxReceivedMessageSize = 16 * 1024 * 1024;
        private @Nullable Object otel;       // OpenTelemetry — type-erased to allow soft dep
        private @Nullable Object prometheus; // PrometheusRegistry — idem

        private Builder() {}

        public Builder endpoint(@NonNull String endpoint) {
            this.endpoint = Objects.requireNonNull(endpoint);
            return this;
        }
        public Builder auth(@NonNull AuthConfig auth) { ... }
        public Builder retry(@NonNull RetryConfig retry) { ... }
        public Builder deadline(@NonNull Duration deadline) { ... }
        public Builder userAgent(@NonNull String ua) { ... }
        public Builder maxReceivedMessageSize(int bytes) { ... }
        public Builder otel(@NonNull io.opentelemetry.api.OpenTelemetry otel) { ... }
        public Builder prometheus(@NonNull Object reg) { ... }

        public EfeClient build() {
            if (endpoint == null) {
                throw new EfeInvalidArgumentException("endpoint is required");
            }
            // ... validate cross-field
            return new EfeClient(this);
        }

        ClientOptions options() { return new ClientOptions(...); }
    }
}
```

### §10.2 ClientOptions internal record

```java
record ClientOptions(
    String endpoint,
    AuthConfig auth,
    RetryConfig retry,
    Duration deadline,
    String userAgent,
    int maxReceivedMessageSize,
    int streamConcurrency,
    @Nullable Object otel,
    @Nullable Object prometheus
) {}
```

Internal — pas exposé via le public API. Construit par le Builder.

### §10.3 Verdict record

```java
package ai.efe.sdk;

/**
 * Verdict — the public verdict type returned by {@link EfeClient#classify}.
 *
 * <p>Java 17 record — immutable by construction.
 *
 * <p>Cross-language pinned : 10 fields match Python M8 + Go M9 + TS M10
 * EXACTLY (camelCase Java vs snake_case wire JSON via the proto-default mapping).
 */
public record Verdict(
    @NonNull String label,
    double confidence,
    boolean reject,
    int rounds,
    double alignmentDelta,
    @NonNull String catalog,
    @NonNull String zone,
    @NonNull String swarmId,
    @NonNull String requestId,
    long elapsedMs
) {

    /** Compact constructor — validates non-null invariants. */
    public Verdict {
        Objects.requireNonNull(label, "label");
        Objects.requireNonNull(catalog, "catalog");
        Objects.requireNonNull(zone, "zone");
        Objects.requireNonNull(swarmId, "swarmId");
        Objects.requireNonNull(requestId, "requestId");
        if (confidence < 0.0 || confidence > 1.0) {
            throw new IllegalArgumentException("confidence must be in [0,1]");
        }
    }

    public boolean isTrusted() { return !reject && "trusted".equals(zone); }
    public boolean isWarn() { return !reject && "warn".equals(zone); }
    public boolean isReject() { return reject || "reject".equals(zone); }

    /** {@inheritDoc} — NEVER includes auth material. */
    @Override
    public String toString() {
        return "Verdict[label=" + label
            + " confidence=" + String.format("%.3f", confidence)
            + " zone=" + zone
            + " rounds=" + rounds
            + (reject ? " reject=true" : "")
            + (requestId.isEmpty() ? "" : " req=" + requestId)
            + " elapsed=" + elapsedMs + "ms]";
    }
}
```

---

## §11. Sub-système 2 — gRPC code generation pipeline

### §11.1 `protobuf-maven-plugin` configuration

Plugin canonical : `org.xolstice.maven.plugins:protobuf-maven-plugin:0.6.1`.

```xml
<plugin>
  <groupId>org.xolstice.maven.plugins</groupId>
  <artifactId>protobuf-maven-plugin</artifactId>
  <configuration>
    <!-- Resolve protoc binary from Maven Central (no host install needed) -->
    <protocArtifact>com.google.protobuf:protoc:${protobuf.version}:exe:${os.detected.classifier}</protocArtifact>
    <!-- gRPC Java codegen plugin -->
    <pluginId>grpc-java</pluginId>
    <pluginArtifact>io.grpc:protoc-gen-grpc-java:${grpc.version}:exe:${os.detected.classifier}</pluginArtifact>
    <!-- Source path -->
    <protoSourceRoot>${project.basedir}/src/main/proto</protoSourceRoot>
    <!-- Output paths -->
    <outputDirectory>${project.build.directory}/generated-sources/protobuf/java</outputDirectory>
    <outputDirectoryGrpc>${project.build.directory}/generated-sources/protobuf/grpc-java</outputDirectoryGrpc>
  </configuration>
  <executions>
    <execution>
      <goals>
        <goal>compile</goal>          <!-- protobuf-java messages -->
        <goal>compile-custom</goal>   <!-- protoc-gen-grpc-java service stubs -->
      </goals>
    </execution>
  </executions>
</plugin>
```

### §11.2 Generated output

Pour chaque `.proto` dans `src/main/proto/efe/v1/` :
* `target/generated-sources/protobuf/java/efe/v1/<Message>.java` — proto messages classes (e.g., `ExecuteRequest.java`, `ExecuteResponse.java`)
* `target/generated-sources/protobuf/grpc-java/efe/v1/<Service>Grpc.java` — gRPC service stubs (e.g., `SwarmServiceGrpc.java` with `BlockingStub`, `FutureStub`, `Stub`)

Maven `build-helper-maven-plugin` ajoute ces paths à `compileSourceRoots` automatiquement.

### §11.3 OS detection

Le `protoc` artifact dépend de l'OS+arch. `os-maven-plugin` extension détecte automatiquement et expose `${os.detected.classifier}` :
* `linux-x86_64`
* `linux-aarch_64`
* `osx-x86_64`
* `osx-aarch_64`
* `windows-x86_64`

### §11.4 Drift detection

Pin test Python `test_d26_java_sdk_protos_in_sync` :

```python
def test_d26_java_sdk_protos_in_sync():
    """`PROTOS_HASH.txt` matches actual SHA-256 of protos/."""
    hash_file = SDK_ROOT / 'efe-sdk-core/src/main/proto/PROTOS_HASH.txt'
    assert hash_file.is_file(), "regenerate via `mvn generate-sources`"
    expected = parse_hashes(hash_file.read_text())
    for proto_name in ('types.proto', 'efe.proto', 'catalog.proto', 'admin.proto'):
        actual = sha256(REPO_ROOT / 'protos/efe/v1' / proto_name)
        assert expected[proto_name] == actual
```

### §11.5 `gen-proto.sh` wrapper

```bash
#!/usr/bin/env bash
# Sprint D26 — invoke mvn generate-sources + update PROTOS_HASH.txt.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

REPO_ROOT="$(cd ../../../.. && pwd)"
PROTOS_DIR="$REPO_ROOT/protos"
CORE_PROTO_DIR="efe-sdk-core/src/main/proto"

# 1. Sync protos/ → src/main/proto/ (copy, not symlink — Maven plugin
# requires a real path).
mkdir -p "$CORE_PROTO_DIR/efe/v1"
cp "$PROTOS_DIR/efe/v1/"*.proto "$CORE_PROTO_DIR/efe/v1/"

# 2. Run mvn.
./mvnw -pl efe-sdk-core generate-sources

# 3. Update PROTOS_HASH.txt.
{
  echo "# Generated by scripts/gen-proto.sh — DO NOT EDIT."
  for f in "$CORE_PROTO_DIR/efe/v1"/*.proto; do
    h=$(shasum -a 256 "$f" | awk '{print $1}')
    echo "$h  $(basename $f)"
  done
} > "$CORE_PROTO_DIR/PROTOS_HASH.txt"

echo "✅ protoc + grpc-java generation complete."
```

### §11.6 Decision : `protobuf-maven-plugin` vs `protoc-jar`

**Décision** : `protobuf-maven-plugin` (xolstice).

**Rationale** :
* **`protobuf-maven-plugin` (xolstice 0.6.1)** : community standard,
  bien maintenu, support gRPC out-of-the-box via `compile-custom`,
  download protoc depuis Maven Central (pas de host install requis).
* **`protoc-jar`** : alternative qui embed protoc dans une dep
  Maven, mais moins flexible pour gRPC.
* **`com.google.protobuf:protoc-gen-grpc-java`** : plugin natif
  Google, mais requiert son propre wiring Maven verbose.

**Pin test** : `test_d26_uses_xolstice_protobuf_plugin`.

---

## §12. Sub-système 3 — Errors + retry + ClientInterceptor chain

### §12.1 Exception hierarchy

```java
package ai.efe.sdk.exception;

/**
 * Base unchecked exception for all SDK-generated errors.
 *
 * <p>Use {@code instanceof EfeException} to detect any SDK error
 * generically ; use {@code instanceof EfeXxxException} to discriminate
 * the specific sentinel.
 *
 * <p>Cross-language pinned : 10 sentinels match Python M8 + Go M9 +
 * TS M10. Java naming convention uses the {@code Exception} suffix
 * (vs Go {@code ErrXxx} or TS {@code ErrXxx}).
 */
public class EfeException extends RuntimeException {
    private final String code;

    public EfeException(String code, String message) {
        super(message);
        this.code = code;
    }
    public EfeException(String code, String message, Throwable cause) {
        super(message, cause);
        this.code = code;
    }

    public String code() { return code; }

    /** Returns true iff err belongs to the retry-eligible subset. */
    public static boolean isRetryable(Throwable err) {
        return err instanceof EfeUnavailableException
            || err instanceof EfeTimeoutException
            || err instanceof EfeRateLimitedException;
    }

    /** Returns the canonical short code (used for metrics labels). */
    public static String errorCode(Throwable err) {
        if (err == null) return "";
        if (err instanceof EfeException ee) return ee.code;
        return "unknown";
    }
}

// 10 sentinel subclasses (one example shown — pattern repeated).
public final class EfeUnauthenticatedException extends EfeException {
    public EfeUnauthenticatedException() {
        this("unauthenticated");
    }
    public EfeUnauthenticatedException(String message) {
        super("unauthenticated", "efe: " + message);
    }
    public EfeUnauthenticatedException(String message, Throwable cause) {
        super("unauthenticated", "efe: " + message, cause);
    }
}
```

### §12.2 ErrorMapper — gRPC Status → sentinel

```java
package ai.efe.sdk.internal;

import io.grpc.Status;
import io.grpc.StatusRuntimeException;
import ai.efe.sdk.exception.*;

public final class ErrorMapper {
    private ErrorMapper() {}

    public static EfeException fromGrpcStatus(StatusRuntimeException e) {
        Status.Code code = e.getStatus().getCode();
        String message = e.getStatus().getDescription();
        if (message == null) message = "(no description)";
        return switch (code) {
            case UNAUTHENTICATED -> new EfeUnauthenticatedException(message, e);
            case PERMISSION_DENIED -> new EfePermissionDeniedException(message, e);
            case NOT_FOUND -> new EfeCatalogNotFoundException(message, e);
            case FAILED_PRECONDITION -> new EfeCatalogInvalidException(message, e);
            case DEADLINE_EXCEEDED, CANCELLED -> new EfeTimeoutException(message, e);
            case UNAVAILABLE -> new EfeUnavailableException(message, e);
            case RESOURCE_EXHAUSTED -> new EfeRateLimitedException(message, e);
            case INVALID_ARGUMENT, OUT_OF_RANGE ->
                new EfeInvalidArgumentException(message, e);
            case INTERNAL, UNKNOWN, DATA_LOSS -> new EfeInternalException(message, e);
            default -> new EfeInternalException(message, e);
        };
    }
}
```

### §12.3 Retry interceptor

```java
package ai.efe.sdk.retry;

import io.grpc.*;
import io.grpc.MethodDescriptor;

public final class RetryClientInterceptor implements ClientInterceptor {
    private final RetryConfig config;

    public RetryClientInterceptor(RetryConfig config) {
        this.config = config;
    }

    @Override
    public <ReqT, RespT> ClientCall<ReqT, RespT> interceptCall(
            MethodDescriptor<ReqT, RespT> method,
            CallOptions callOptions,
            Channel next) {
        return new RetryingClientCall<>(method, callOptions, next, config);
    }
}

/** Implements the retry loop via ClientCall.Listener decoration. */
final class RetryingClientCall<ReqT, RespT> extends ForwardingClientCall<ReqT, RespT> {
    // ...
    @Override
    public void start(ClientCall.Listener<RespT> responseListener, Metadata headers) {
        // First attempt + listener that on UNAVAILABLE/DEADLINE/RESOURCE_EXHAUSTED
        // retries with exponential backoff via a ScheduledExecutorService.
        // ...
    }
}
```

### §12.4 Interceptor chain order

```
Caller → retry → prom → otel → auth → ManagedChannel → Server
```

Order canonical (mêmes que Go M9 + TS M10) :
1. **retry (outermost)** : retries are a separate observation.
2. **prom** : metrics observe POST-retry timing.
3. **otel** : span captures the actual on-wire request.
4. **auth (innermost)** : token attached at the boundary.

### §12.5 RetryConfig record

```java
public record RetryConfig(int maxAttempts, RetryBackoff backoff) {
    public RetryConfig {
        if (maxAttempts < 1) throw new IllegalArgumentException("maxAttempts ≥ 1");
        Objects.requireNonNull(backoff);
    }

    public static RetryConfig exponential(int maxAttempts, Duration initial, Duration max) {
        return new RetryConfig(maxAttempts, new ExponentialBackoff(initial, max, 2.0));
    }
    public static RetryConfig disabled() {
        return new RetryConfig(1, attempt -> Duration.ZERO);
    }
}

public final class ExponentialBackoff implements RetryBackoff {
    private final Duration initial;
    private final Duration max;
    private final double multiplier;
    private final RandomGenerator random;

    public ExponentialBackoff(Duration initial, Duration max, double multiplier) {
        this(initial, max, multiplier, RandomGenerator.getDefault());
    }
    ExponentialBackoff(Duration initial, Duration max, double multiplier, RandomGenerator rng) {
        // ... validate
        this.random = rng;
    }

    @Override
    public Duration next(int attempt) {
        long base = (long) Math.min(
            initial.toMillis() * Math.pow(multiplier, attempt - 1),
            max.toMillis());
        // Full jitter — uniform [0, base]
        long delay = (long) (random.nextDouble() * base);
        return Duration.ofMillis(delay);
    }
}
```

---

## §13. Sub-système 4 — OpenTelemetry auto-instrumentation

### §13.1 Soft dep (provided scope)

`pom.xml` declarative :

```xml
<dependency>
    <groupId>io.opentelemetry</groupId>
    <artifactId>opentelemetry-api</artifactId>
    <version>${opentelemetry.version}</version>
    <scope>provided</scope>
    <optional>true</optional>
</dependency>
```

### §13.2 OtelClientInterceptor

```java
package ai.efe.sdk.otel;

import io.grpc.*;
import io.opentelemetry.api.OpenTelemetry;
import io.opentelemetry.api.trace.Span;
import io.opentelemetry.api.trace.SpanKind;
import io.opentelemetry.api.trace.Tracer;
import io.opentelemetry.context.Scope;

public final class OtelClientInterceptor implements ClientInterceptor {
    private final Tracer tracer;

    public OtelClientInterceptor(OpenTelemetry openTelemetry) {
        this.tracer = openTelemetry.getTracer("ai.efe.sdk", Version.VERSION);
    }

    @Override
    public <ReqT, RespT> ClientCall<ReqT, RespT> interceptCall(
            MethodDescriptor<ReqT, RespT> method,
            CallOptions callOptions,
            Channel next) {
        Span span = tracer.spanBuilder("efe/" + method.getFullMethodName())
            .setSpanKind(SpanKind.CLIENT)
            .setAttribute("rpc.system", "grpc")
            .setAttribute("rpc.service", method.getServiceName())
            .setAttribute("rpc.method", method.getBareMethodName())
            .startSpan();
        try (Scope ignored = span.makeCurrent()) {
            return new ForwardingClientCall.SimpleForwardingClientCall<>(
                next.newCall(method, callOptions)) {
                @Override
                public void start(Listener<RespT> responseListener, Metadata headers) {
                    super.start(new ForwardingClientCallListener.SimpleForwardingClientCallListener<>(
                        responseListener) {
                        @Override
                        public void onClose(Status status, Metadata trailers) {
                            if (!status.isOk()) {
                                span.setStatus(io.opentelemetry.api.trace.StatusCode.ERROR,
                                    status.getDescription() != null ? status.getDescription() : "");
                            }
                            span.end();
                            super.onClose(status, trailers);
                        }
                    }, headers);
                }
            };
        }
    }
}
```

### §13.3 No-op fallback

Si l'app ne fournit pas un `OpenTelemetry` au builder, le SDK ne wire
PAS l'interceptor (`opts.otel() == null` → skip). Pas d'overhead, pas
de `OpenTelemetry.noop()` allocation.

---

## §14. Sub-système 5 — Prometheus metrics

### §14.1 5 collecteurs canoniques (cross-language)

Mêmes noms que Python M8 + Go M9 + TS M10 :

```java
package ai.efe.sdk.prometheus;

import io.prometheus.metrics.core.metrics.Counter;
import io.prometheus.metrics.core.metrics.Gauge;
import io.prometheus.metrics.core.metrics.Histogram;
import io.prometheus.metrics.model.registry.PrometheusRegistry;

public final class SdkMetrics {
    public static final String CLASSIFY_TOTAL = "efe_sdk_classify_total";
    public static final String CLASSIFY_LATENCY = "efe_sdk_classify_latency_seconds";
    public static final String CLASSIFY_ERRORS_TOTAL = "efe_sdk_classify_errors_total";
    public static final String IN_FLIGHT = "efe_sdk_in_flight";
    public static final String RETRY_ATTEMPTS_TOTAL = "efe_sdk_retry_attempts_total";

    private final Counter classifyTotal;
    private final Histogram classifyLatency;
    private final Counter classifyErrors;
    private final Gauge inFlight;
    private final Counter retryAttempts;

    public SdkMetrics(PrometheusRegistry registry) {
        this.classifyTotal = Counter.builder()
            .name(CLASSIFY_TOTAL)
            .help("Total efe_sdk Classify calls, partitioned by outcome.")
            .labelNames("outcome")
            .register(registry);
        this.classifyLatency = Histogram.builder()
            .name(CLASSIFY_LATENCY)
            .help("Classify wall-clock latency in seconds, partitioned by outcome.")
            .labelNames("outcome")
            .classicLinearBuckets(0.005, 0.01, 12)
            .register(registry);
        this.classifyErrors = Counter.builder()
            .name(CLASSIFY_ERRORS_TOTAL)
            .help("Total efe_sdk Classify errors, partitioned by sentinel code.")
            .labelNames("sentinel")
            .register(registry);
        this.inFlight = Gauge.builder()
            .name(IN_FLIGHT)
            .help("Currently in-flight Classify calls.")
            .register(registry);
        this.retryAttempts = Counter.builder()
            .name(RETRY_ATTEMPTS_TOTAL)
            .help("Total retry attempts, partitioned by attempt number.")
            .labelNames("attempt")
            .register(registry);
    }

    // ... per-call observation methods
}
```

### §14.2 PrometheusClientInterceptor

ClientInterceptor qui wraps each call : ++ inFlight on start, observe
latency on end, ++ classifyTotal{outcome=success|error}, etc.

### §14.3 Pin test cross-language

```python
CANONICAL_METRIC_NAMES_JAVA = (
    'efe_sdk_classify_total',
    'efe_sdk_classify_latency_seconds',
    'efe_sdk_classify_errors_total',
    'efe_sdk_in_flight',
    'efe_sdk_retry_attempts_total',
)

def test_d26_java_sdk_metric_names_match_other_sdks():
    """Java SDK metric names must match Python/Go/TS SDKs exactly."""
    src = (CORE / 'src/main/java/ai/efe/sdk/prometheus/SdkMetrics.java').read_text()
    for name in CANONICAL_METRIC_NAMES_JAVA:
        assert name in src
```

---

## §15. Sub-système 6 — Auth (JWT CallCredentials + mTLS)

### §15.1 JWT CallCredentials

```java
package ai.efe.sdk.auth;

import io.grpc.CallCredentials;
import io.grpc.Metadata;
import io.grpc.Status;
import java.util.concurrent.Executor;

/**
 * gRPC CallCredentials that injects "Authorization: Bearer <jwt>"
 * on every outgoing RPC.
 *
 * <p>Captured at construction ; never logged ; redacted by
 * {@code toString()}.
 */
public final class JwtCallCredentials extends CallCredentials {
    private static final Metadata.Key<String> AUTHORIZATION =
        Metadata.Key.of("authorization", Metadata.ASCII_STRING_MARSHALLER);

    private final String bearerHeader;

    public JwtCallCredentials(String token) {
        if (token == null || token.isBlank()) {
            throw new IllegalArgumentException("JWT token must be non-blank");
        }
        this.bearerHeader = "Bearer " + token;
    }

    @Override
    public void applyRequestMetadata(RequestInfo requestInfo, Executor appExecutor,
                                       MetadataApplier applier) {
        appExecutor.execute(() -> {
            try {
                Metadata headers = new Metadata();
                headers.put(AUTHORIZATION, bearerHeader);
                applier.apply(headers);
            } catch (Throwable t) {
                applier.fail(Status.UNAUTHENTICATED.withCause(t));
            }
        });
    }

    @Override
    public String toString() {
        return "JwtCallCredentials{<redacted>}";
    }
}
```

### §15.2 mTLS via GrpcSslContexts

```java
package ai.efe.sdk.auth;

import io.grpc.netty.shaded.io.grpc.netty.GrpcSslContexts;
import io.grpc.netty.shaded.io.netty.handler.ssl.SslContext;

public final class MtlsBuilder {
    private MtlsBuilder() {}

    public static SslContext build(MtlsAuth auth) {
        try {
            return GrpcSslContexts.forClient()
                .keyManager(auth.certPath().toFile(), auth.keyPath().toFile())
                .trustManager(auth.caPath().toFile())
                .protocols("TLSv1.3")
                .build();
        } catch (Exception e) {
            throw new EfeInvalidArgumentException("mTLS: cannot build SslContext: " + e.getMessage(), e);
        }
    }
}
```

### §15.3 Auth wiring in ChannelFactory

```java
package ai.efe.sdk.internal;

import io.grpc.ManagedChannel;
import io.grpc.netty.shaded.io.grpc.netty.NettyChannelBuilder;

public final class ChannelFactory {
    public static ManagedChannel create(ClientOptions opts) {
        var builder = NettyChannelBuilder.forTarget(opts.endpoint());
        switch (opts.auth()) {
            case JwtAuth jwt -> {
                builder.useTransportSecurity()
                    .build()
                    .withCallCredentials(new JwtCallCredentials(jwt.token()));
            }
            case MtlsAuth mtls -> {
                builder.sslContext(MtlsBuilder.build(mtls));
            }
            case NoAuth none -> {
                builder.usePlaintext();
            }
        }
        // Apply max recv size, user agent, interceptors...
        return builder
            .userAgent(opts.userAgent())
            .maxInboundMessageSize(opts.maxReceivedMessageSize())
            .intercept(buildInterceptors(opts))
            .build();
    }
}
```

---

## §16. Sub-système 7 — 3 Spring Boot examples

### §16.1 spring-boot-mvc

```java
@SpringBootApplication
public class Application {
    public static void main(String[] args) { SpringApplication.run(Application.class, args); }
}

@Configuration
public class EfeConfig {
    @Bean(destroyMethod = "close")
    public EfeClient efeClient(@Value("${efe.endpoint}") String endpoint,
                                 @Value("${efe.jwt}") String jwt) {
        return EfeClient.builder()
            .endpoint(endpoint)
            .auth(JwtAuth.of(jwt))
            .deadline(Duration.ofSeconds(10))
            .build();
    }
}

@RestController
@RequestMapping("/api")
public class ClassifyController {
    private final EfeClient efe;

    public ClassifyController(EfeClient efe) { this.efe = efe; }

    @PostMapping("/classify")
    public ResponseEntity<Verdict> classify(@RequestBody ClassifyRequest req) {
        try {
            Verdict v = efe.classify(req.text());
            return ResponseEntity.ok(v);
        } catch (EfeUnauthenticatedException e) {
            return ResponseEntity.status(401).build();
        } catch (EfeRateLimitedException e) {
            return ResponseEntity.status(429).build();
        } catch (EfeException e) {
            return ResponseEntity.status(500).build();
        }
    }

    @GetMapping("/healthz")
    public ResponseEntity<Map<String, String>> healthz() {
        try {
            efe.healthcheck();
            return ResponseEntity.ok(Map.of("status", "serving"));
        } catch (EfeException e) {
            return ResponseEntity.status(503).body(Map.of("status", "not_serving"));
        }
    }

    public record ClassifyRequest(String text) {}
}
```

### §16.2 spring-boot-webflux

Idem mais avec `Mono<Verdict>` :

```java
@RestController
public class ClassifyController {
    private final EfeClient efe;

    @PostMapping("/classify")
    public Mono<Verdict> classify(@RequestBody ClassifyRequest req) {
        return Mono.fromFuture(efe.classifyAsync(req.text()))
            .onErrorMap(EfeException.class, this::toResponseStatusException);
    }
}
```

### §16.3 spring-boot-actuator

Démontre l'auto-configuration + Prometheus metrics expose via
`/actuator/prometheus`.

```java
@Configuration
@AutoConfiguration  // Spring Boot 3 pattern
public class EfeAutoConfiguration {

    @Bean
    @ConditionalOnMissingBean
    public PrometheusRegistry prometheusRegistry() {
        return new PrometheusRegistry();
    }

    @Bean(destroyMethod = "close")
    public EfeClient efeClient(EfeProperties props,
                                 OpenTelemetry openTelemetry,
                                 PrometheusRegistry prometheusRegistry) {
        return EfeClient.builder()
            .endpoint(props.endpoint())
            .auth(JwtAuth.of(props.jwt()))
            .otel(openTelemetry)
            .prometheus(prometheusRegistry)
            .build();
    }
}

@ConfigurationProperties("efe")
public record EfeProperties(String endpoint, String jwt) {}
```

`application.yml` :

```yaml
efe:
  endpoint: "staging.efe.ai:50051"
  jwt: "${EFE_JWT_TOKEN}"

management:
  endpoints.web.exposure.include: prometheus,health,info
  endpoint.prometheus.access: read_only
```

---

## §17. Sub-système 8 — Mock infrastructure

### §17.1 InProcessServerBuilder (équivalent bufconn)

`io.grpc:grpc-inprocess` + `grpc-testing` fournissent
`InProcessServerBuilder` + `InProcessChannelBuilder` qui spawn un
serveur gRPC in-memory transporté par un nom de canal symbolique.
Pas de socket TCP — pure JVM.

### §17.2 MockEfeServer

```java
package ai.efe.sdk.mock;

import io.grpc.Server;
import io.grpc.inprocess.InProcessServerBuilder;
import efe.v1.SwarmServiceGrpc;
import efe.v1.Efe.ExecuteRequest;
import efe.v1.Efe.ExecuteResponse;

public final class MockEfeServer implements AutoCloseable {
    private final Server server;
    private final String name;
    private @Nullable Function<ExecuteRequest, ExecuteResponse> classifyHandler;

    public static MockEfeServer create() throws IOException {
        return new MockEfeServer("mock-" + UUID.randomUUID());
    }

    private MockEfeServer(String name) throws IOException {
        this.name = name;
        this.server = InProcessServerBuilder
            .forName(name)
            .directExecutor()
            .addService(new MockSwarmService())
            .build()
            .start();
    }

    public String getEndpoint() {
        return "in-process:" + name;  // Used by EfeClient.builder().endpoint(...)
    }

    public void onClassify(Function<ExecuteRequest, ExecuteResponse> handler) {
        this.classifyHandler = handler;
    }

    @Override
    public void close() {
        server.shutdownNow();
        try {
            server.awaitTermination(5, TimeUnit.SECONDS);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        }
    }

    private final class MockSwarmService extends SwarmServiceGrpc.SwarmServiceImplBase {
        @Override
        public void execute(ExecuteRequest request, StreamObserver<ExecuteResponse> responseObserver) {
            if (classifyHandler == null) {
                responseObserver.onError(Status.UNIMPLEMENTED.asException());
                return;
            }
            try {
                responseObserver.onNext(classifyHandler.apply(request));
                responseObserver.onCompleted();
            } catch (Throwable t) {
                responseObserver.onError(Status.INTERNAL.withDescription(t.getMessage()).asException());
            }
        }
    }
}
```

### §17.3 ChannelFactory.create handling in-process

```java
public static ManagedChannel create(ClientOptions opts) {
    String endpoint = opts.endpoint();
    if (endpoint.startsWith("in-process:")) {
        String name = endpoint.substring("in-process:".length());
        return InProcessChannelBuilder.forName(name).directExecutor().build();
    }
    // Standard NettyChannel path...
}
```

### §17.4 MockEfeClient

```java
public final class MockEfeClient {
    private @Nullable Verdict verdict;
    private @Nullable Throwable error;
    private final List<String> calls = Collections.synchronizedList(new ArrayList<>());
    private boolean closed = false;

    public void setVerdict(Verdict v) { this.verdict = v; }
    public void setError(Throwable err) { this.error = err; }
    public List<String> getCalls() { return List.copyOf(calls); }
    public int getCallCount() { return calls.size(); }
    public void reset() {
        verdict = null; error = null; calls.clear(); closed = false;
    }

    public Verdict classify(String text) {
        if (closed) throw new EfeClosedException("mock client is closed");
        calls.add(text);
        if (error != null) {
            if (error instanceof RuntimeException re) throw re;
            throw new RuntimeException(error);
        }
        return verdict != null ? verdict : VerdictFactory.newVerdict();
    }
}
```

---

## §18. Stratégie de tests

### §18.1 Pyramid

```
                          /\
                         /  \  Spring Boot examples manual run (smoke)
                        /----\
                       /      \
                      /  JUnit \  ~10 integration tests vs InProcessServer
                     / integration\
                    /------------- \
                   /               \
                  /  JUnit unit      \  ~30 unit tests (no I/O)
                 / (Vitest-equivalent) \
                /-----------------------\
               /                         \
              /  Pin tests Python         \  ~32 tests structure
             /----------------------------- \
```

### §18.2 JUnit 5 unit tests (≥ 30)

| Test class | Tests | Focus |
|---|:---:|---|
| `VerdictTest` | 5 | record immutability, isTrusted/isWarn/isReject, toString redacted |
| `ExceptionTest` | 7 | 10 sentinels distinct, isRetryable, errorCode, instanceof EfeException |
| `ClientOptionsTest` | 8 | Builder validation : empty endpoint, neg deadline, bad retry, JWT empty, mTLS partial paths |
| `retry/ExponentialBackoffTest` | 5 | defaults, monotonic growth, max cap, never negative, jitter variance |
| `auth/JwtCallCredentialsTest` | 3 | header injection, empty token rejection, toString redacted |
| `prometheus/SdkMetricsTest` | 3 | 5 metric names match canonical, registration order |
| `internal/ErrorMapperTest` | 4 | gRPC Status → sentinel mapping |

### §18.3 JUnit integration (≥ 10)

```java
class ClassifyIntegrationTest {

    private MockEfeServer server;
    private EfeClient client;

    @BeforeEach
    void setUp() throws IOException {
        server = MockEfeServer.create();
        client = EfeClient.builder().endpoint(server.getEndpoint()).build();
    }

    @AfterEach
    void tearDown() throws Exception {
        client.close();
        server.close();
    }

    @Test
    @DisplayName("classify returns server-side verdict with derived zone")
    void happyPath() {
        server.onClassify(req -> ExecuteResponse.newBuilder()
            .setOutput(DecodedOutput.newBuilder().setText("billing"))
            .setConfidence(0.91f)
            .setVerdict(Verdict.VERDICT_CONVERGED)
            .build());

        Verdict v = client.classify("I want a refund");

        assertThat(v.label()).isEqualTo("billing");
        assertThat(v.confidence()).isCloseTo(0.91, withinPercentage(1.0));
        assertThat(v.zone()).isEqualTo("trusted");
        assertThat(v.reject()).isFalse();
    }

    @Test
    void emptyTextThrowsInvalidArgument() {
        assertThatThrownBy(() -> client.classify("  "))
            .isInstanceOf(EfeInvalidArgumentException.class);
    }

    @Test
    void afterCloseThrowsEfeClosed() {
        client.close();
        assertThatThrownBy(() -> client.classify("foo"))
            .isInstanceOf(EfeClosedException.class);
    }

    @Test
    void retryOnUnavailable() {
        AtomicInteger calls = new AtomicInteger();
        server.onClassify(req -> {
            int n = calls.incrementAndGet();
            if (n < 3) throw Status.UNAVAILABLE.withDescription("transient").asRuntimeException();
            return ExecuteResponse.newBuilder()
                .setOutput(DecodedOutput.newBuilder().setText("ok"))
                .setConfidence(0.9f)
                .setVerdict(Verdict.VERDICT_CONVERGED)
                .build();
        });

        Verdict v = client.classify("hello");

        assertThat(calls.get()).isEqualTo(3);
        assertThat(v.label()).isEqualTo("ok");
    }

    // ... 6 more tests
}
```

### §18.4 JaCoCo coverage configuration

```xml
<plugin>
  <groupId>org.jacoco</groupId>
  <artifactId>jacoco-maven-plugin</artifactId>
  <executions>
    <execution>
      <goals><goal>prepare-agent</goal></goals>
    </execution>
    <execution>
      <id>report</id>
      <phase>verify</phase>
      <goals><goal>report</goal></goals>
    </execution>
    <execution>
      <id>check</id>
      <goals><goal>check</goal></goals>
      <configuration>
        <rules>
          <rule>
            <element>BUNDLE</element>
            <limits>
              <limit><counter>LINE</counter><value>COVEREDRATIO</value><minimum>0.80</minimum></limit>
              <limit><counter>BRANCH</counter><value>COVEREDRATIO</value><minimum>0.70</minimum></limit>
            </limits>
            <excludes>
              <exclude>ai.efe.sdk.internal.generated.*</exclude>
              <exclude>efe.v1.*</exclude>
            </excludes>
          </rule>
        </rules>
      </configuration>
    </execution>
  </executions>
</plugin>
```

### §18.5 Pin tests Python (≈ 32)

| Section | Tests |
|---|:---:|
| §1. Maven skeleton | 5 |
| §2. Version + Verdict cross-lang | 2 |
| §3. Public API surface (errors + options + retry) | 6 |
| §4. Verdict + StreamItem records | 2 |
| §5. Prometheus metrics canonical | 1 |
| §6. Generated proto stubs | 2 |
| §7. Mock module | 3 |
| §8. Spring Boot examples | 3 |
| §9. Scripts shell | 4 |
| §10. CI workflows | 3 |
| §11. Maven Central pipeline (pom + GPG plugins) | 1 |
| **Total** | **32** |

---

## §19. JPMS `module-info.java` + Javadoc strategy

### §19.1 module-info.java

```java
/**
 * EFE SDK Java — official Java/JVM SDK for the EFE inference platform.
 */
module ai.efe.sdk {
    // Public surface
    exports ai.efe.sdk;
    exports ai.efe.sdk.auth;
    exports ai.efe.sdk.exception;
    exports ai.efe.sdk.retry;
    // Internal — explicitly NOT exported
    // exports ai.efe.sdk.internal;  // intentionally NOT exported

    // Required dependencies
    requires transitive io.grpc.api;
    requires io.grpc.protobuf;
    requires io.grpc.stub;
    requires com.google.protobuf;
    requires org.slf4j;
    requires static org.jspecify;          // compile-time only

    // Optional (soft) deps
    requires static io.opentelemetry.api;  // peer dep
    requires static io.prometheus.metrics.core;  // peer dep
}
```

### §19.2 Javadoc generation

```xml
<plugin>
  <groupId>org.apache.maven.plugins</groupId>
  <artifactId>maven-javadoc-plugin</artifactId>
  <executions>
    <execution>
      <id>attach-javadocs</id>
      <goals><goal>jar</goal></goals>
      <configuration>
        <source>17</source>
        <doclint>all,-missing</doclint>
        <links>
          <link>https://docs.oracle.com/en/java/javase/17/docs/api/</link>
          <link>https://grpc.github.io/grpc-java/javadoc/</link>
        </links>
        <failOnError>true</failOnError>
        <failOnWarnings>true</failOnWarnings>
      </configuration>
    </execution>
  </executions>
</plugin>
```

---

## §20. JAR shading + dependency conflict isolation

### §20.1 Pourquoi `grpc-netty-shaded` plutôt que self-shading

Le SDK NE shade PAS lui-même. La raison :
1. **Tree-shake-ability** : un consumer qui n'utilise que `Verdict` + `EfeException`
   peut bénéficier d'un proguard / shrinker pour stripper les classes
   internal — facile sans shading.
2. **Debug** : les stack traces conservent les noms canoniques.
3. **`grpc-netty-shaded` fait déjà le job pour Netty** — la principale
   source de conflits (Netty 4.x vs 5.x, boringssl vs JDK SSL).

### §20.2 Dependency tree assertion

```xml
<plugin>
  <artifactId>maven-enforcer-plugin</artifactId>
  <executions>
    <execution>
      <id>enforce-no-netty-conflict</id>
      <goals><goal>enforce</goal></goals>
      <configuration>
        <rules>
          <bannedDependencies>
            <excludes>
              <!-- Refuse non-shaded Netty (use grpc-netty-shaded) -->
              <exclude>io.netty:netty-all</exclude>
              <exclude>io.netty:netty-handler</exclude>
            </excludes>
          </bannedDependencies>
        </rules>
      </configuration>
    </execution>
  </executions>
</plugin>
```

---

## §21. SLO mapping

| SLO | Méthode | Cible | Pin test |
|---|---|---|---|
| SLO-JAVA-1 cold start < 100 ms | JUnit microbench `EfeClient.builder().build()` | < 100 ms post JIT | DT-J-9 |
| SLO-JAVA-2 p99 ≤ server p99 + 50 ms | Integration test 1k iter | < 50 ms overhead | DT-J-10 |
| SLO-JAVA-3 core JAR < 500 KB | mvn package size assertion | ≤ 500 KB | DT-J-11 |
| SLO-JAVA-4 Java 17+ supported | matrix CI Java 17 + 21 | green | DT-J-12 |
| SLO-JAVA-5 coverage ≥ 80% | JaCoCo | ≥ 80% lines | DT-J-13 |
| SLO-JAVA-6 Maven Central publishable | central-publishing-maven-plugin staging | dry-run OK | DT-J-14 |
| SLO-JAVA-7 javadoc clean | mvn javadoc:javadoc | 0 warning | DT-J-15 |
| SLO-JAVA-8 JPMS module clean | mvn package modular | OK | DT-J-16 |

---

## §22. Risques transverses + mitigations

| Risque | P | I | Mitigation |
|---|:-:|:-:|---|
| GPG keys management dans CI | M | H | Provision via secrets ; documented dans publish.sh |
| Maven Central staging timeout 4h | L | M | central-publishing-maven-plugin auto-retries ; alert ops |
| `grpc-netty-shaded` major bump | M | M | Pin major `1.6x` ; matrix CI valide |
| Spring Boot 3.x breaking change | L | M | Pin Spring Boot 3.3.x LTS |
| JPMS module conflicts en consumer | M | M | `requires static` for soft deps ; doc canonical usage |
| Java 17 → 21 record difference | L | L | Tests CI valident |
| protobuf-maven-plugin Windows | L | L | Test CI Windows ; OS-specific classifier auto |
| `protoc-gen-grpc-java` version mismatch | L | M | Pin via `${grpc.version}` |

---

## §23. Volumétrie code attendue

| Composant | Hand-coded LOC | Generated LOC | Total |
|---|---:|---:|---:|
| `efe-sdk-core/src/main/java/` | 3 500 | — | 3 500 |
| `efe-sdk-core/target/generated-sources/` | — | 2 500 | 2 500 |
| `efe-sdk-mock/src/main/java/` | 400 | — | 400 |
| `efe-sdk-core/src/test/java/` (JUnit + integration) | 1 500 | — | 1 500 |
| Examples (3 sub-projects) | 700 | — | 700 |
| Scripts shell | 280 | — | 280 |
| CI workflows YAML | 450 | — | 450 |
| Pin tests Python | 750 | — | 750 |
| `pom.xml` files | 350 | — | 350 |
| README + CHANGELOG + LICENSE | 300 | — | 300 |
| **Total** | **~8 230 hand** | **~2 500 gen** | **~10 730 LOC** |

---

## §24. Plan d'implémentation J1-J10

### J1 — Bootstrap Maven multi-module

* Parent `pom.xml` (packaging=pom) + `<modules>` + `<dependencyManagement>` + `<pluginManagement>`.
* `efe-sdk-core/pom.xml` + `efe-sdk-mock/pom.xml` (skeleton).
* `mvn validate` OK.
* `protobuf-maven-plugin` config + `mvn generate-sources` produces output.
* Maven Wrapper (`mvnw`) committed for CI reproducibility.
* `git commit` initial : "chore(m11): scaffold java sdk maven multi-module".

### J2 — Core types : Verdict + 10 EfeException + ClientOptions Builder

* `Verdict.java` (record) + `StreamItem.java` (record).
* `EfeException.java` base + 10 subclasses.
* `ClientOptions` + `EfeClient.Builder` (initial).
* `Version.java` (`public static final String VERSION = "0.4.0";`).
* JUnit unit tests : 6 tests OK.

### J3 — EfeClient sync API + happy path

* `EfeClient.classify(text)` (blocking, BlockingStub).
* `ChannelFactory.create(opts)` (NettyChannelBuilder + interceptor chain).
* `MockEfeServer` (InProcessServerBuilder).
* 1 integration test "happy path" GREEN.

### J4 — Auth + retry + interceptors

* `auth/JwtCallCredentials` + `auth/MtlsBuilder`.
* `auth/AuthConfig` sealed interface + 3 records.
* `retry/ExponentialBackoff` + `retry/RetryClientInterceptor`.
* `internal/ErrorMapper` (gRPC Status → exception).
* Tests retry 3x on UNAVAILABLE + auth header asserted + mTLS SslContext built.

### J5 — OTel + Prometheus + mock module

* `otel/OtelClientInterceptor`.
* `prometheus/SdkMetrics` + `prometheus/PrometheusClientInterceptor`.
* `efe-sdk-mock/` module : `MockEfeClient` + `VerdictFactory`.
* Pin tests : 5 metric names canonical.

### J6 — Async API + classifyStream

* `EfeClient.classifyAsync(text)` (FutureStub + CompletableFuture conversion).
* `EfeClient.classifyStream(texts)` (worker pool ExecutorService + dispatcher).
* Integration test : 50 texts streamed, indices preserved.

### J7 — 3 Spring Boot examples

* `examples/spring-boot-mvc/` (Spring MVC + RestController).
* `examples/spring-boot-webflux/` (WebFlux + Mono.fromFuture).
* `examples/spring-boot-actuator/` (auto-config + Prometheus expose).
* Each `mvn spring-boot:run` + curl OK.

### J8 — Tests massifs + JaCoCo

* Compléter JUnit unit ≥ 30.
* Compléter integration ≥ 10.
* JaCoCo coverage > 80% lines.
* Fix flakiness.

### J9 — Pin tests Python + scripts/ + CI workflows

* `scripts/tests/test_d26_java_sdk_extension.py` (~32 tests).
* `scripts/{gen-proto,publish,lint,e2e-bootstrap}.sh`.
* `.github/workflows/sdk-java-test.yml`.
* `.github/workflows/sdk-java-smoke.yml`.
* `.github/workflows/sdk-java-publish.yml` (gated on tag, GPG signing).
* `pytest` GREEN.

### J10 — Audit pass 1 + CHANGELOG + Maven staging dry-run

* Audit chirurgical (à la M9 + M10).
* Apply fixes (~10 expected).
* Add audit-fix pin tests.
* `mvn deploy -DskipTests --dry-run` OK + signed JARs validation.
* CHANGELOG M11 D26 [Unreleased].
* PR + CI green.

---

## §25. Versioning + publication Maven Central

### §25.1 Semver alignment

* `0.4.0` = D26 ship (private, GitHub Packages, scope `ai.efe`).
* `0.4.x` = patch privés.
* `0.5.0` = D27+ extension (efe-sdk-spring-boot-starter).
* `1.0.0` = GA public, Maven Central via Sonatype Central Portal.

### §25.2 Private publish (Φ-B) — GitHub Packages

```bash
# Pre-flight : NPM-equivalent for Java
# ~/.m2/settings.xml :
# <servers>
#   <server>
#     <id>github</id>
#     <username>efe-ai-bot</username>
#     <password>${env.GITHUB_TOKEN}</password>
#   </server>
# </servers>

cd clients/java/efe-sdk
./mvnw clean
./mvnw -DskipTests deploy -DaltDeploymentRepository=github::default::https://maven.pkg.github.com/efe-ai/efe
```

### §25.3 Public publish (GA) — Maven Central

```bash
# Pre-flight :
#  1. Sonatype Central Portal account at https://central.sonatype.com
#  2. GPG key generated + uploaded to keys.openpgp.org
#  3. ~/.m2/settings.xml has <server id="central"> with portal token

cd clients/java/efe-sdk
./mvnw clean
./mvnw -DskipTests deploy -P release
# `release` profile activates :
#   - maven-source-plugin (sources jar)
#   - maven-javadoc-plugin (javadoc jar)
#   - maven-gpg-plugin (sign all artifacts)
#   - central-publishing-maven-plugin (stage + auto-release)
```

### §25.4 `release` profile (parent pom.xml)

```xml
<profiles>
  <profile>
    <id>release</id>
    <build>
      <plugins>
        <plugin>
          <groupId>org.apache.maven.plugins</groupId>
          <artifactId>maven-source-plugin</artifactId>
          <executions>
            <execution>
              <id>attach-sources</id>
              <goals><goal>jar-no-fork</goal></goals>
            </execution>
          </executions>
        </plugin>
        <plugin>
          <groupId>org.apache.maven.plugins</groupId>
          <artifactId>maven-javadoc-plugin</artifactId>
          <executions>
            <execution>
              <id>attach-javadocs</id>
              <goals><goal>jar</goal></goals>
            </execution>
          </executions>
        </plugin>
        <plugin>
          <groupId>org.apache.maven.plugins</groupId>
          <artifactId>maven-gpg-plugin</artifactId>
          <executions>
            <execution>
              <id>sign-artifacts</id>
              <phase>verify</phase>
              <goals><goal>sign</goal></goals>
            </execution>
          </executions>
        </plugin>
        <plugin>
          <groupId>org.sonatype.central</groupId>
          <artifactId>central-publishing-maven-plugin</artifactId>
          <extensions>true</extensions>
          <configuration>
            <publishingServerId>central</publishingServerId>
            <autoPublish>true</autoPublish>
            <waitUntil>published</waitUntil>
          </configuration>
        </plugin>
      </plugins>
    </build>
  </profile>
</profiles>
```

### §25.5 Consumer install

Pendant Φ-B :

```xml
<repositories>
  <repository>
    <id>github-efe-ai</id>
    <url>https://maven.pkg.github.com/efe-ai/efe</url>
  </repository>
</repositories>

<dependency>
  <groupId>ai.efe</groupId>
  <artifactId>efe-sdk-java</artifactId>
  <version>0.4.0</version>
</dependency>
```

Post-GA :

```xml
<dependency>
  <groupId>ai.efe</groupId>
  <artifactId>efe-sdk-java</artifactId>
  <version>1.0.0</version>
</dependency>
```

---

## §26. Compliance + sécurité

### §26.1 GPG signing pipeline

* GPG key 4096 bits RSA, dédiée au release pipeline.
* Public key uploaded sur `keys.openpgp.org`.
* Private key + passphrase stockés dans GitHub Secrets :
  * `GPG_PRIVATE_KEY` : ASCII-armored private key.
  * `GPG_PASSPHRASE` : passphrase de la clé.
* `sdk-java-publish.yml` import la clé via `crazy-max/ghaction-import-gpg@v6`.

### §26.2 Supply chain hardening

* `dependency-check-maven` (OWASP) scans transitives → CVE detection.
* `mvn dependency:tree` audit manuel.
* Dependabot Maven daily scans.
* Reproducible builds via `${project.build.outputTimestamp}` propriété.

### §26.3 Secret redaction

* `EfeClient.toString()` redacts auth.
* `JwtCallCredentials.toString()` retourne `"<redacted>"`.
* slf4j logging — JWT JAMAIS dans les logs.
* mTLS material : `Path` references seulement, pas de bytes en mémoire log-able.

### §26.4 SBOM (Software Bill of Materials)

`cyclonedx-maven-plugin` génère un SBOM CycloneDX au format JSON pour
chaque release :

```xml
<plugin>
  <groupId>org.cyclonedx</groupId>
  <artifactId>cyclonedx-maven-plugin</artifactId>
  <version>2.8.0</version>
  <executions>
    <execution>
      <phase>package</phase>
      <goals><goal>makeAggregateBom</goal></goals>
    </execution>
  </executions>
</plugin>
```

SBOM uploadé en artifact GitHub Release.

---

## §27. Annexes — `pom.xml` parent canonical (extrait)

[Voir §6.3 et §6.4 pour les fragments. Le fichier complet est ~250
lignes XML, distribué entre le parent + 2 modules + le release
profile.]

---

## §28. Annexes — surface API Java complète

[Voir §8.1 et §8.2 pour la déclaration. Documentation Javadoc ~800
lignes générée par `mvn javadoc:javadoc` lors du build.]

---

## §29. Annexes — Maven plugins config + scripts

### §29.1 `gen-proto.sh` — voir §11.5

### §29.2 `publish.sh`

```bash
#!/usr/bin/env bash
# Sprint D26 — publish efe-sdk-java via Maven.
#
# Two modes :
#   PUBLISH=private (default) — deploy to GitHub Packages (Φ-B private).
#   PUBLISH=public            — deploy to Maven Central (GA, requires v1.x.x).

set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

PUBLISH="${PUBLISH:-private}"
VERSION=$(./mvnw -q -Dexec.executable=echo -Dexec.args='${project.version}' --non-recursive exec:exec)

# Pre-flight.
[[ -z "$(git status --porcelain)" ]] || { echo "FATAL: dirty tree"; exit 3; }

# Build + test.
./mvnw clean install

case "$PUBLISH" in
  private)
    ./mvnw -DskipTests deploy \
        -DaltDeploymentRepository=github::default::https://maven.pkg.github.com/efe-ai/efe
    git tag -s "v$VERSION" -m "efe-sdk-java v$VERSION (private)"
    git push origin "v$VERSION"
    ;;
  public)
    [[ "$VERSION" =~ ^1\. ]] || { echo "FATAL: public requires v1.x.x"; exit 5; }
    ./mvnw -DskipTests -P release deploy
    git tag -s "v$VERSION" -m "efe-sdk-java v$VERSION (PUBLIC)"
    git push origin "v$VERSION"
    ;;
  *)
    echo "FATAL: unknown PUBLISH=$PUBLISH"; exit 1
    ;;
esac
```

### §29.3 `lint.sh`

```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

# Checkstyle + SpotBugs + JaCoCo coverage check.
./mvnw checkstyle:check
./mvnw spotbugs:check
./mvnw verify -DskipTests=false  # triggers jacoco:check
./mvnw javadoc:javadoc -DfailOnError=true -DfailOnWarnings=true
```

---

## §30. Annexes — RACI + budget + Gantt

### §30.1 RACI

| Activity | R | A | C | I |
|---|:-:|:-:|:-:|:-:|
| Architecture doc (this) | rodrigue | rodrigue | — | team |
| Bootstrap Maven multi-module | rodrigue | rodrigue | — | team |
| Core types + sentinels | rodrigue | rodrigue | — | team |
| EfeClient + ChannelFactory | rodrigue | rodrigue | — | team |
| 3 Spring Boot examples | rodrigue | rodrigue | — | team |
| Pin tests Python | rodrigue | rodrigue | — | team |
| CI workflows + GPG | rodrigue | rodrigue | devops | team |
| Maven Central account + GPG keys | rodrigue | platform-eng | rodrigue | team |
| Publish dry-run + GA | rodrigue | rodrigue | platform-eng | team |

### §30.2 Budget temps

10 jours wallclock × 1 dev = 80 h.

| Phase | Heures | Cumulé |
|---|---:|---:|
| J1 Bootstrap | 8 | 8 |
| J2 Core types | 8 | 16 |
| J3 EfeClient sync + happy path | 10 | 26 |
| J4 Auth+retry+interceptors | 10 | 36 |
| J5 OTel+Prom+mock | 8 | 44 |
| J6 Async + classifyStream | 8 | 52 |
| J7 3 Spring Boot examples | 8 | 60 |
| J8 Tests massifs + JaCoCo | 8 | 68 |
| J9 Pin+scripts+CI+GPG | 8 | 76 |
| J10 Audit + Maven staging | 4 | 80 |

### §30.3 Gantt

```
J1 │██ Bootstrap (Maven multi-module)
J2 │  █ Core types
J3 │   █ EfeClient sync
J4 │    █ Auth+retry
J5 │     █ OTel+Prom+mock
J6 │      █ Async+stream
J7 │       █ Examples
J8 │        █ Tests massifs
J9 │         █ Pin+CI+GPG
J10│          █ Audit+staging
```

---

## §31. Mises à jour du document

| Date | Version | Auteur | Diff |
|---|---|---|---|
| 2026-04-30 | v1.0.0 | rodrigue | Création initiale (greenfield) |

---

**Fin du document EFE_JAVA_SDK_EXTENSION_ARCHITECTURE.md (M11 D26).**
