# Sprint D14 — Architecture détaillée : Postgres-backed catalog registry

> **Statut** : architecture détaillée, prête à implémenter.
> **Phase** : ultra-thinking. Précision chirurgicale suprême.
> **Position** : suit D12-bis (gRPC handler) et son audit interne
> (`docs/audits/d12bis_audit_report.md` — 7 bugs trouvés, 6 fixés
> dont C1 quotas non injectés et C2 Helm/test drift).
> **Pré-requis** : avoir lu
> [`d12_catalog_admin_versioning_architecture.md`](d12_catalog_admin_versioning_architecture.md)
> (1 673 lignes, le trait `CatalogRegistry` + `InMemoryRegistry`),
> [`d12bis_catalog_grpc_handler_architecture.md`](d12bis_catalog_grpc_handler_architecture.md)
> (2 736 lignes, le handler gRPC),
> [`d12bis_audit_report.md`](../audits/d12bis_audit_report.md)
> (audit + fixes C1+C2+H1+M1+M2+L1),
> [`d11_per_tenant_isolation_architecture.md`](d11_per_tenant_isolation_architecture.md)
> (zero-trust per-tenant invariant — D14 doit l'enforcer en SQL),
> [`d13_observability_architecture.md`](d13_observability_architecture.md)
> (instruments + spans + events à étendre pour Postgres).
>
> **Mission** : remplacer `InMemoryRegistry` par
> `PostgresCatalogRegistry` derrière le **même trait
> `CatalogRegistry`**. Le runtime n'a qu'une branche à modifier
> (`runtime.rs::build` ligne 124) : `CatalogBackend::Postgres` cesse
> de retourner `Err`, instancie le nouveau backend. Le trait n'est
> pas touché ; **les 6 RPC du handler n'ont rien à apprendre**. Le
> reste du sprint est : schéma SQL + advisory locks + sqlx migrate
> + connection pool + observabilité étendue + tests testcontainers
> + Helm wiring + threat model + runbook ops.
>
> **Pain point fondamental que D14 résout** :
> `restart efe-server = perte totale de tous les catalogues`.
> Aujourd'hui, un upgrade rolling Helm, un crash, un OOM-kill
> → tous les catalogues enregistrés via `RegisterCatalog`
> disparaissent. D14 transforme `efe-server` d'un **binaire
> éphémère** en un **service stateful durable** sans changer la
> surface gRPC ni rebriefer un seul client.

---

## Table des matières

1. [Contexte & mission D14](#1-contexte--mission-d14)
2. [État des lieux post-D12-bis — pré-conditions remplies](#2-état-des-lieux-post-d12-bis--pré-conditions-remplies)
3. [Décision majeure #1 — choix de la couche d'accès SQL](#3-décision-majeure-1--choix-de-la-couche-daccès-sql)
4. [Décision majeure #2 — schéma : single table vs partitioned per tenant](#4-décision-majeure-2--schéma--single-table-vs-partitioned-per-tenant)
5. [Décision majeure #3 — version monotone sous contention](#5-décision-majeure-3--version-monotone-sous-contention)
6. [Schéma SQL détaillé](#6-schéma-sql-détaillé)
7. [Mapping `CatalogRegistry` trait → SQL](#7-mapping-catalogregistry-trait--sql)
8. [Connection pool : sizing, lifecycle, leak detection](#8-connection-pool--sizing-lifecycle-leak-detection)
9. [Migrations : sqlx migrate, forward-only](#9-migrations--sqlx-migrate-forward-only)
10. [Tests : testcontainers + integration suite](#10-tests--testcontainers--integration-suite)
11. [Performance budget vs `InMemoryRegistry`](#11-performance-budget-vs-inmemoryregistry)
12. [Observabilité D13 — instruments ajoutés](#12-observabilité-d13--instruments-ajoutés)
13. [Auth + tenant isolation D11 — défense en profondeur SQL](#13-auth--tenant-isolation-d11--défense-en-profondeur-sql)
14. [Backup / restore / point-in-time recovery](#14-backup--restore--point-in-time-recovery)
15. [Sécurité — TLS, secrets, RLS, SQL injection](#15-sécurité--tls-secrets-rls-sql-injection)
16. [Helm wiring — postgres connection block](#16-helm-wiring--postgres-connection-block)
17. [Runtime wire-up — `Server::build` branche Postgres](#17-runtime-wire-up--serverbuild-branche-postgres)
18. [Threat model — 8 vecteurs](#18-threat-model--8-vecteurs)
19. [Decisions log — récap des décisions clés](#19-decisions-log--récap-des-décisions-clés)
20. [Risques + mitigations](#20-risques--mitigations)
21. [Plan wallclock — 7 j](#21-plan-wallclock--7-j)
22. [SLO ajoutés en D14](#22-slo-ajoutés-en-d14)
23. [Anti-régression D14](#23-anti-régression-d14)
24. [Hand-off vers D15](#24-hand-off-vers-d15)
25. [Annexes](#25-annexes)

---

## 1. Contexte & mission D14

### 1.1 État post-D13/D12-bis

Au gel de v0.1.13.1 (post-D12-bis-audit) :

* **6 RPC catalog gRPC opérationnels** (`RegisterCatalog`,
  `EvolveCatalog`, `GetCatalog`, `ListCatalogs`,
  `ListCatalogVersions`, `DeleteCatalog`) — handler dans
  `crates/efe-server/src/services/catalog/mod.rs`, audit RBAC
  read/write, isolation D11, observabilité D13.
* **Trait `CatalogRegistry`** stable :
  ```rust
  #[async_trait]
  pub trait CatalogRegistry: Send + Sync {
      async fn register(&self, tenant: &str, name: &str,
                        content: CatalogContent,
                        metadata: CatalogMetadata)
                        -> Result<RegisteredCatalog, CatalogError>;
      async fn evolve(&self, from: &CatalogKey, op: EvolveOp,
                      metadata_override: CatalogMetadata)
                      -> Result<RegisteredCatalog, CatalogError>;
      async fn get(&self, key: &CatalogKey, include_compiled: bool)
                   -> Result<RegisteredCatalog, CatalogError>;
      async fn list_catalogs(&self, tenant: &str, page_size: u32,
                             page_token: Option<&str>)
                             -> Result<CatalogPage, CatalogError>;
      async fn list_versions(&self, tenant: &str, name: &str)
                             -> Result<Vec<CatalogKey>, CatalogError>;
      async fn delete(&self, key: &CatalogKey, force: bool,
                      principal: &str)
                      -> Result<(), CatalogError>;
  }
  ```
* **`InMemoryRegistry`** en `crates/efe-server/src/catalog/memory.rs`
  — implémentation in-process, `Arc<RwLock<BTreeMap<…>>>`, validée
  par 31 tests unitaires + 28 tests d'intégration tonic loopback.
  Quotas configurables (`CatalogPolicy` injecté depuis
  `config.catalog.policy` via le fix C1 de l'audit D12-bis).
* **`runtime.rs::Server::build`** ligne 124 :
  ```rust
  CatalogBackend::Postgres => {
      return Err(anyhow::anyhow!(
          "EFE_CATALOG_BACKEND=postgres is reserved for D14 ; \
           set EFE_CATALOG_BACKEND=memory until then."
      ));
  }
  ```
  → **fail-fast au boot** quand `EFE_CATALOG_BACKEND=postgres`.
  D14 lève cette restriction.
* **D11 zero-trust per-tenant** : enforce dans le handler via
  `assert_tenant_match(auth_tenant, claimed_tenant)`. D14 doit
  préserver cet invariant et le **doubler en SQL** (defense in
  depth — toute requête a `WHERE tenant = $1`, jamais de
  cross-tenant SELECT possible même si le handler avait un bug).
* **D13 observabilité** : 8 instruments Prometheus
  (`efe_catalog_*_duration_seconds`, `efe_audit_events_total`,
  `efe_catalog_versions_per_name`), 16 spans, 5 typed event
  builders. D14 ajoute 4 instruments PG-spécifiques (cf. § 12).

### 1.2 Le problème opérationnel

Aujourd'hui, le backend `Memory` est un `Arc<RwLock<BTreeMap<…>>>`
in-process. Conséquences :

* **Restart = perte totale.** Helm rolling upgrade, OOM, panic,
  SIGTERM → toutes les `(tenant, name, version)` registrations
  partent en fumée.
* **Pas de réplication.** Si un opérateur monte 3 réplicas pour la
  HA, **chaque réplica a son propre store** — `RegisterCatalog` sur
  réplica A est invisible sur réplicas B et C. Le load balancer
  envoie aléatoirement, le client voit un comportement non
  déterministe.
* **Pas de partage entre processus.** Plusieurs binaires
  `efe-server` sur la même machine (e.g. blue/green deploy) ont des
  registres divergents.
* **Pas de migration de données** entre versions du binaire. Bumper
  `Mode1Config` ou la struct `CatalogContent` invalide silencieusement
  les catalogues existants en RAM.
* **Pas de backup.** Aucun moyen de snapshot l'état du registry
  pour audit ou rollback.

D14 **résout les 5 problèmes simultanément** en remplaçant le
backend in-memory par **PostgreSQL** (ou compatible — Aurora,
CloudSQL, Crunchy, Neon).

### 1.3 Mission D14 (énoncé chirurgical)

D14 livre **une seule nouvelle impl** du trait `CatalogRegistry` :
`PostgresCatalogRegistry`. Le périmètre est strictement délimité :

**Doit livrer :**

1. `crates/efe-server/src/catalog/postgres.rs` : impl complète
   du trait `CatalogRegistry` sur sqlx + Postgres.
2. `crates/efe-server/migrations/` : 5 migrations sqlx (DDL
   forward-only).
3. `crates/efe-server/src/catalog/postgres_pool.rs` : connection
   pool wrapper (sizing, leak detection, health probe).
4. `crates/efe-server/src/runtime.rs` : ouvrir la branche
   `CatalogBackend::Postgres` (cesser le `Err`).
5. `crates/efe-server/src/config.rs` : bloc `CatalogPostgresConfig`
   + 11 nouvelles env vars `EFE_PG_*`.
6. `crates/efe-server/tests/d14_postgres_catalog_integration.rs` :
   ~30 tests testcontainers (happy paths, concurrency,
   isolation D11, error mapping, migration application).
7. `deploy/helm/efe/values.yaml` : bloc `catalog.postgres`
   (host, port, database, ssl mode, secret ref).
8. `deploy/helm/efe/templates/configmap.yaml` : 11 env vars.
9. `deploy/helm/efe/templates/secret-postgres.yaml` (optionnel) :
   Secret pour le password ; pris en charge par
   `existingSecretName` ou external-secrets.
10. `docs/bench/d14_postgres_catalog_backend_architecture.md`
    (ce document).
11. `docs/audits/d14_audit_report.md` (skeleton ; rempli post-impl).
12. 4 nouveaux instruments D13 (gauge pool, histogram query, counter
    transaction failures, gauge migration version).
13. CHANGELOG.md entrée [Unreleased] § "Sprint D14".
14. BENCHMARKS.md § 16 (D14, v0.1.14).
15. `bench_baselines/v0.1.14/baseline.json` avec mesures.

**NE doit PAS livrer :**

* Changements au trait `CatalogRegistry` (interface stable D12).
* Modifications du handler gRPC (D12-bis livré).
* Modifications des conversions proto↔domain (D12-bis livré).
* Modifications du schéma proto `efe.v1.catalog.*` (D12 livré).
* Replication / HA Postgres setup côté EFE (responsabilité ops via
  pgBackRest, postgres operator, RDS Multi-AZ, etc. — out of scope).
* Compaction / archival des vieilles versions (D15).

### 1.4 Hors-périmètre

Pour éviter l'embonpoint :

| Hors scope D14 | Justification | Sprint cible |
|----------------|---------------|--------------|
| Compaction des vieilles versions (e.g. drop des versions ≥ 30 j non référencées) | Politique métier, pas implémentation | **D15** |
| HA setup Postgres (Patroni, primary/standby) | Responsabilité ops | external |
| Online schema migration zero-downtime au-delà du forward-only | Rare en MVP, complexité haute | **D16** |
| Multi-region active/active | Conflict-free version assignment requires CRDT or external sequencer | **D17+** |
| Read replicas for `Get` / `List` | Optimisation, pas correctness | **D15** |
| Native sharding par tenant | Single-table v1 est largement suffisant pour scale moyen | **D18+** |
| Schema versioning beyond `parent_key` chain | YAGNI pour v0.1.14 | future |
| Audit log à l'extérieur de la table catalog (e.g. Loki dédié) | D13 audit déjà couvert via `tracing` + `efe_audit_events_total` | n/a |

---

## 2. État des lieux post-D12-bis — pré-conditions remplies

### 2.1 Invariants stables (NE PAS modifier)

D14 doit honorer chaque invariant ci-dessous sans exception :

| Invariant | Source | Impact si violé |
|-----------|--------|-----------------|
| Trait `CatalogRegistry` ne change pas | D12 trait + 6 RPC handler | Le handler casse → tous les RPC catalog cassent |
| `(tenant, name)` cardinality monotone : version `N+1` strictement après `N` | D12 contract test `monotonic_versions` | Audit history corrompue ; rollback impossible |
| `register` est atomique : soit toute la `(tenant, name, version)` row + content + metadata est commit, soit rien | D12 design | Catalogues partiels ; lectures fantômes |
| `delete force=false` refuse si la version est référencée par une `parent_key` postérieure | D12 contract test `delete_referenced_without_force_returns_failed_precondition` | Audit chain coupée ; impossibilité de rollback |
| `tenant` ne fuit jamais entre RPC | D11 invariant | Confidentialité multi-tenant cassée — show-stopper |
| `principal_present` label sur `efe_audit_events_total` | D13 fix C-1 | Alerte `D13ObsAuditPrincipalUnknown` aveugle |
| Erreur `Internal` ne leak pas le contenu vers le client | D12-bis fix audit T-12bis-2 | Info disclosure (DB IPs / paths) |
| Quotas `CatalogPolicy` configurables via `config.catalog.policy` | D12-bis audit fix C1 | Helm config muette → DoS opérateur |

### 2.2 Surface trait ne change pas

**Le trait `CatalogRegistry` est invariant entre D12 (in-memory) et
D14 (Postgres).** C'est le pivot architectural : le sprint D14 a un
seul livrable côté handler — *ouvrir la branche `Postgres` du
runtime*. Tout le reste est *derrière* le trait.

```rust
// runtime.rs::Server::build (D12-bis post-fix C1) — D14 modifie UNE branche

let catalog_policy = CatalogPolicy::from(&config.catalog.policy);
let catalog_audit  = Arc::new(CatalogAuditEmitter::new());
let catalog_registry: Arc<dyn CatalogRegistry> = match self.catalog_registry {
    Some(r) => r,
    None => match config.catalog.backend {
        CatalogBackend::Memory => Arc::new(
            InMemoryRegistry::with_policy_and_audit(
                catalog_policy.clone(),
                CatalogAuditEmitter::new(),
            ),
        ),
        // === D14 NEW : ouvrir la branche Postgres ===
        CatalogBackend::Postgres => Arc::new(
            PostgresCatalogRegistry::connect(
                &config.catalog.postgres,
                catalog_policy.clone(),
                CatalogAuditEmitter::new(),
            ).await?,
        ),
        // === plus de fail-fast ===
    },
};
```

L'invariance du trait permet **3 garanties testabilité fortes** :

1. Les **51 tests unitaires** + **28 tests d'intégration** existants
   s'appliquent sans modification. On ajoute une autre série pour
   le backend Postgres ; on ne *supprime* rien.
2. Le **trait contract test** (cf. annexe C de l'architecture D12)
   permet de pinner les invariants pour les *deux* impls. Si
   `PostgresCatalogRegistry::register` viole `monotonic_versions`,
   le contract test casse.
3. Tests d'intégration D12-bis (`tests/d12bis_catalog_integration.rs`)
   continuent de tourner avec `EFE_CATALOG_BACKEND=memory` (le
   défaut). Une seconde suite `tests/d14_postgres_*.rs` les répète
   avec `EFE_CATALOG_BACKEND=postgres` + testcontainers.

### 2.3 Gaps que D14 ferme

| Gap | Symptôme actuel | Fix D14 |
|-----|-----------------|---------|
| **Persistence** | Restart = perte | Tables Postgres durables |
| **Multi-réplica** | 3 répliques ⇒ 3 stores divergents | Toutes les répliques tapent le même cluster PG |
| **Backup** | Impossible | `pg_basebackup` + WAL archiving + `pg_dump` |
| **Migration data inter-version** | Catalogues invalidés silencieusement | `sqlx migrate` versioned, fail-fast au boot si schema obsolète |
| **Audit trail durable** | Logs stdout/Loki seulement (ephemeral si Loki down) | Table `audit_events` SQL append-only (option) — out of scope strict mais le schéma est préparé |
| **Concurrent writers** | Lock RwLock in-process — ne survit pas à 2 répliques | `pg_advisory_xact_lock` per `(tenant, name)` |

---

## 3. Décision majeure #1 — choix de la couche d'accès SQL

### 3.1 Critères d'évaluation

| Critère | Poids | Pourquoi |
|---------|:-----:|----------|
| Async-native (tokio) | 5/5 | Tonic + tokio runtime ; sync ORM = wrap futur fragile |
| Compile-time query check | 4/5 | Catch SQL typos en CI, pas en prod |
| Connection pool intégré | 4/5 | Pas de bb8/deadpool wrapper |
| Migrations toolchain | 4/5 | Forward-only, version tracked, repeatable |
| Maturité écosystème | 5/5 | Doit être stable ≥ 2 ans, pas de breaking change majeur dans 6 mois |
| Footprint (binary size + deps) | 3/5 | EFE binary actuellement ~80 MB ; chaque ajout pèse |
| Type safety pour rows ↔ structs | 4/5 | Manual unmarshalling = source de bugs |
| Postgres feature support (advisory locks, JSONB, BYTEA, ARRAY, INTERVAL) | 5/5 | D14 utilise tous ces types |
| Async transaction nesting | 4/5 | `register` + `evolve` font transaction → savepoints utiles |
| Error introspection (UNIQUE_VIOLATION code 23505) | 5/5 | Mappage `CatalogError::Conflict` |

### 3.2 Comparaison

#### Option A : `sqlx`

**Pour :**
- Async-native, première classe tokio.
- Compile-time query check via `query!()` macro (avec offline mode
  pour CI : `sqlx prepare` snapshot).
- `PgPool` intégré, configurable (max_connections, max_lifetime,
  acquire_timeout, idle_timeout).
- `sqlx migrate` built-in (forward-only par défaut, optional
  reversible).
- Type mapping rich : `BIGINT` ↔ `i64`, `JSONB` ↔ `serde_json::Value`,
  `BYTEA` ↔ `Vec<u8>`, `TIMESTAMPTZ` ↔ `chrono::DateTime` (ou
  `time::OffsetDateTime`).
- Error introspection précise via `sqlx::Error::Database(_)` →
  `db.code() == Some("23505")` pour UNIQUE_VIOLATION.
- Maturité : v0.7.x stable, utilisé par actix, axum, leptos, des
  centaines de crates.
- Pas de macro proc reflection lourde au runtime — query! est tout
  fait en const-eval.

**Contre :**
- Pas de "ORM" feature complete (pas de `Catalog::find().with_metadata()`).
  Mais c'est intentionnel : on veut SQL explicite pour D14 (audit,
  performance).
- Macro `query!()` exige une connexion live à PG au build OU
  `cargo sqlx prepare` snapshot dans `.sqlx/` — friction CI.
- Migration tooling un peu rough (pas d'undo automatique — mais
  on choisit forward-only de toute façon).

#### Option B : `diesel`

**Pour :**
- Maturité legendary (depuis 2015).
- Type system très puissant (DSL compile-time, joins type-checked).
- Migration tooling robuste (`diesel migration`).

**Contre :**
- **Sync only** au cœur ; `diesel-async` existe mais c'est un
  wrapper qui n'est pas first-class. Risque drift.
- Macro `table!{}` forte friction sur les schemas dynamiques.
- Compile time élevé (templates lourdes).
- Connection pool externe requis (bb8 ou r2d2).

#### Option C : `sea-orm`

**Pour :**
- Async natif (sur sqlx en interne).
- Active record + entity macros.

**Contre :**
- Encore une couche de macros au-dessus de sqlx → temps de compile,
  binary size.
- Maturité moins éprouvée que sqlx ou diesel.
- Ergonomics ORM ne se traduit pas en gain pour D14 (on a 6 RPC
  rigides, pas de queries dynamiques).

#### Option D : `tokio-postgres` raw

**Pour :**
- Minimaliste, dépendance fine.
- Direct control.

**Contre :**
- Tout à la main : pool (deadpool/bb8), migrations
  (refinery/barrel), type mapping verbose.
- Pas de query check compile-time.
- Verbose, source de bugs au volume D14.

### 3.3 Décision : `sqlx`

| Critère | Poids | sqlx | diesel | sea-orm | tokio-postgres |
|---------|:-----:|:----:|:------:|:-------:|:--------------:|
| Async-native | 5 | ✅ 5 | ❌ 1 | ✅ 5 | ✅ 5 |
| Compile query check | 4 | ✅ 5 | ✅ 5 | 🟡 3 | ❌ 0 |
| Pool intégré | 4 | ✅ 5 | ❌ 0 | ✅ 5 | ❌ 0 |
| Migrations | 4 | ✅ 4 | ✅ 5 | ✅ 4 | ❌ 0 |
| Maturité | 5 | ✅ 5 | ✅ 5 | 🟡 3 | ✅ 5 |
| Footprint | 3 | ✅ 4 | 🟡 3 | 🟡 3 | ✅ 5 |
| Type safety | 4 | ✅ 5 | ✅ 5 | ✅ 5 | 🟡 3 |
| PG feature support | 5 | ✅ 5 | ✅ 5 | ✅ 5 | ✅ 5 |
| Tx nesting | 4 | ✅ 4 | ✅ 5 | ✅ 4 | ✅ 4 |
| Error introspection | 5 | ✅ 5 | ✅ 5 | ✅ 5 | ✅ 5 |
| **Total pondéré (max 215)** | | **194** | 137 | 169 | 137 |

→ **Décision : sqlx**, avec features `runtime-tokio`, `tls-rustls`,
`postgres`, `chrono`, `json`, `migrate`, `macros`. Pas de feature
`offline` pinned — à la place, on commit `.sqlx/` dans le repo (CI
runs `cargo sqlx prepare --check` pour détecter le drift).

**Cargo.toml additions** :

```toml
[dependencies]
# ... existing ...
sqlx = { version = "0.7", default-features = false, features = [
    "runtime-tokio",
    "tls-rustls",
    "postgres",
    "chrono",
    "json",
    "migrate",
    "macros",
    "uuid",  # for audit event ids if we go that route
] }
```

---

## 4. Décision majeure #2 — schéma : single table vs partitioned per tenant

### 4.1 Trade-offs

#### Option A : single table avec `(tenant, name, version)` PK composite

**Pour :**
- Simple, 1 schema, 1 migration.
- Indexes performants : `(tenant, name, version DESC)` composite.
- `pg_advisory_xact_lock(hashtext(tenant || '/' || name))` per-key
  scale linéairement.
- Backup/restore trivial.
- Foreign keys possibles pour le `parent_key` chain.

**Contre :**
- Si tenant X a 10⁹ catalogs et tenant Y a 10² catalogs, le tenant Y
  paie le coût des indexes du tenant X.
- VACUUM frequency dictée par le tenant le plus actif.
- DDL changes (e.g. adding a column) lock the table — affects all
  tenants.

#### Option B : LIST partitioning par `tenant`

```sql
CREATE TABLE catalogs (...) PARTITION BY LIST (tenant);
CREATE TABLE catalogs_acme PARTITION OF catalogs FOR VALUES IN ('acme');
CREATE TABLE catalogs_globex PARTITION OF catalogs FOR VALUES IN ('globex');
-- etc.
```

**Pour :**
- Isolation physique tenant-par-tenant.
- VACUUM par partition.
- DDL par partition (lock plus fin).

**Contre :**
- Requiert la **création dynamique** de partitions à chaque nouveau
  tenant onboardé. Pas trivial : on ne peut pas `CREATE TABLE` au
  vol depuis un trait `CatalogRegistry::register` sans quote-injection
  exposure.
- Postgres ≥ 14 a `CREATE TABLE … PARTITION OF` mais doit être
  exécuté hors transaction normale.
- Un tenant peut avoir 0 catalogues et payer le coût d'une partition
  vide.
- pg_advisory_xact_lock cross-partition se comporte différemment
  (acquérir le lock sur tenant X, puis tenant Y, déadlock potentiel).
- Pas de bénéfice perf tangible avant ~10⁵+ catalogs/tenant.

#### Option C : table-par-tenant (e.g. `acme_catalogs`)

**Contre majeur :**
- Quote injection si on construit `'CREATE TABLE ' || tenant_id || '_catalogs'`.
- Schema explosion : 1 000 tenants = 1 000 tables.
- search_path dynamique = overhead par requête.
- Backup/restore complexe.

**Rejeté immédiatement.**

### 4.2 Décision : single table v1 + composite PK

→ **Décision : single table `catalogs` + composite primary key**.

```sql
CREATE TABLE catalogs (
    tenant         TEXT     NOT NULL,
    name           TEXT     NOT NULL,
    version        INTEGER  NOT NULL,
    -- ...
    PRIMARY KEY (tenant, name, version)
);
```

**Justification :**

1. v0.1.14 cible des tenants de l'ordre de 10² (PME, équipes
   produit), pas de hyper-scale.
2. La complexité ops du LIST partitioning n'est pas amortie avant
   ~10⁵+ catalogs/tenant — bien au-delà de la cible v0.1.14.
3. Pour basculer en LIST partitioning plus tard (D17+), un script
   migrate-to-partitioned existe (cf. § 4.3) et reste forward-only.
4. La cardinalité par défaut est protégée par `CatalogPolicy`
   (max_names_per_tenant = 100, max_versions_per_name = 1000) — le
   plafond théorique pour 10³ tenants est ~10⁸ rows, soit ~50 GB
   compressé Postgres. Manageable single-table.

### 4.3 Future migration vers LIST partitioning

Documenté pour D17+ mais **non livré en D14** :

```sql
-- Étape 1 : créer la table partitionnée parallèle.
CREATE TABLE catalogs_partitioned (
    LIKE catalogs INCLUDING ALL
) PARTITION BY LIST (tenant);

-- Étape 2 : créer une partition par tenant existant.
DO $$
DECLARE t TEXT;
BEGIN
    FOR t IN SELECT DISTINCT tenant FROM catalogs LOOP
        EXECUTE format(
            'CREATE TABLE catalogs_%I PARTITION OF catalogs_partitioned FOR VALUES IN (%L)',
            t, t
        );
    END LOOP;
END$$;

-- Étape 3 : copier (en transaction).
INSERT INTO catalogs_partitioned SELECT * FROM catalogs;

-- Étape 4 : swap.
ALTER TABLE catalogs RENAME TO catalogs_legacy;
ALTER TABLE catalogs_partitioned RENAME TO catalogs;

-- Étape 5 : drop legacy (off-peak).
DROP TABLE catalogs_legacy;
```

→ tracking ouvert dans le sprint closure D14 comme **DT-D14.1**.

---

## 5. Décision majeure #3 — version monotone sous contention

### 5.1 Problème

Le trait `CatalogRegistry::register` doit assigner la version
**`max(existing) + 1`** par `(tenant, name)`, **sans race**. Si deux
clients concurrents register sur la même clé, **les deux doivent
réussir** mais avec des versions distinctes (e.g. `v1` et `v2`),
**pas tous les deux à `v1`**.

**SLO D12-CT-6 (D12)** : *les concurrent registers sur la même
`(tenant, name)` reçoivent toutes des versions distinctes monotones,
sans dropped registration*.

L'`InMemoryRegistry` résout ça avec `RwLock<Store>` : la write lock
pendant `max + 1 + insert` sérialise. En SQL, on n'a pas ce luxe —
les transactions concurrentes voient toutes la même `max(version)`
avant de tenter un insert.

### 5.2 Options évaluées

#### Option A : `SERIALIZABLE` isolation level

```rust
let mut tx = pool.begin().await?;
sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
    .execute(&mut tx).await?;
// ... SELECT MAX + INSERT ...
tx.commit().await?;
```

**Pour :** sémantique claire, postgres garantit serialisable.
**Contre :**
- Coût élevé (predicate locking).
- Retry obligatoire sur `serialization_failure` (40001).
- Sous fortes concurrence, throughput tank.

#### Option B : `REPEATABLE READ` + `SELECT MAX FOR UPDATE`

```sql
SELECT MAX(version) FROM catalogs
WHERE tenant = $1 AND name = $2
FOR UPDATE;
```

**Contre :** `FOR UPDATE` sur un `MAX()` lock-row mais sur quelle
ligne ? Postgres lock... pas évident, prone à deadlocks. Souvent
bug-prone.

#### Option C : `SEQUENCE` per `(tenant, name)`

```sql
CREATE SEQUENCE catalogs_acme_billing_seq;
SELECT nextval('catalogs_acme_billing_seq');
```

**Contre :** explosion sequences (10⁵ tenants × 10² names =
10⁷ sequences). Création dynamique = quote injection. Rejeté.

#### Option D : `INSERT … ON CONFLICT DO NOTHING RETURNING` + retry

```sql
INSERT INTO catalogs (tenant, name, version, …)
VALUES ($1, $2, (SELECT COALESCE(MAX(version), 0) + 1 FROM catalogs WHERE tenant=$1 AND name=$2), …)
ON CONFLICT (tenant, name, version) DO NOTHING
RETURNING version;
```

**Contre :** sous concurrence haute, taux de retry monte. Pas
déterministe, pas borné.

#### Option E : `pg_advisory_xact_lock` per `(tenant, name)`

```rust
// Hash (tenant || '/' || name) en bigint pour le lock id.
// Postgres exige int8 (8 octets) pour le advisory lock id.
let lock_id = hash_tenant_name(tenant, name);
let mut tx = pool.begin().await?;
sqlx::query("SELECT pg_advisory_xact_lock($1)")
    .bind(lock_id).execute(&mut tx).await?;
// L'advisory lock vit le temps de la transaction.
let max_v: Option<i32> = sqlx::query_scalar(
    "SELECT MAX(version) FROM catalogs WHERE tenant = $1 AND name = $2"
).bind(tenant).bind(name).fetch_one(&mut tx).await?;
let next_v = max_v.map(|v| v + 1).unwrap_or(1);
sqlx::query("INSERT INTO catalogs (tenant, name, version, …) VALUES (…)")
    .execute(&mut tx).await?;
tx.commit().await?;  // libère le lock automatiquement.
```

**Pour :**
- **Sémantique exacte** : pas deux registrations concurrentes pour
  la même `(tenant, name)` ; deux `(tenant, name)` distincts ne se
  bloquent pas.
- **Pas de retry** ; le second waiter attend, puis voit la nouvelle
  `max`.
- **Bounded contention** : O(1) advisory lock, pas de scan.
- Lock automatiquement libéré à `COMMIT` ou `ROLLBACK` (pas de
  manuel cleanup).
- Postgres-natif, robuste, utilisé en production par tous les ORMs
  qui font de la séquence custom.

**Contre :**
- `lock_id` est un `int8` ; on hash `(tenant, name)` → risque de
  collision sur 2⁶⁴. Probabilité absurde mais documentée.
- L'advisory lock est dans le namespace du serveur PG ; si on a 2
  bases logiques sur le même cluster qui hashent les mêmes IDs, on
  bloque cross-base. **Mitigation** : l'EFE a sa propre database, pas
  de partage.

### 5.3 Décision : option E (advisory locks)

→ **Décision : `pg_advisory_xact_lock(hash_id_for(tenant, name))`**

**Hash function spec** :

```rust
/// Stable 64-bit hash of (tenant, name) for use as a Postgres
/// advisory lock id. Uses xxh64 for low collision rate and high
/// throughput. The seed is a constant tied to the EFE namespace
/// so unrelated apps on the same cluster don't collide.
const ADVISORY_LOCK_SEED: u64 = 0xefe_0000_d014_0000;

fn lock_id_for(tenant: &str, name: &str) -> i64 {
    let mut buf = Vec::with_capacity(tenant.len() + 1 + name.len());
    buf.extend_from_slice(tenant.as_bytes());
    buf.push(b'/');
    buf.extend_from_slice(name.as_bytes());
    let h = xxhash_rust::xxh64::xxh64(&buf, ADVISORY_LOCK_SEED);
    h as i64  // bit-cast — sign doesn't matter for the lock semantics
}
```

**Properties** :

| Property | Value |
|----------|-------|
| Collision probability between 2 random `(tenant, name)` pairs | 2⁻⁶⁴ ≈ 5 × 10⁻²⁰ |
| Collision probability among 10⁶ pairs (birthday) | ~3 × 10⁻⁸ |
| Hash latency | ~5 ns / 32 B input (xxh64 is fast) |
| Lock acquire latency on uncontended cluster | ~100 µs |
| Lock acquire latency under contention (8 concurrent) | depends on tx duration ; bounded by sum of tx lengths |

**Limitation documentée** : si les 2⁶⁴ ID space est partagé avec
une autre app sur le même cluster, on peut avoir des locks fantômes
inter-app. **Mitigation** : EFE owns its database (cf. § 16 Helm —
on crée un user `efe_catalog` avec sa propre DB `efe_catalog_db`).

**Contract test** (cf. § 10.4) :

```rust
#[tokio::test]
async fn advisory_lock_serialises_concurrent_register_same_key() {
    let pg = TestPostgres::start().await;
    let r = PostgresCatalogRegistry::connect(&pg.config(), …).await.unwrap();
    let r1 = r.clone();
    let r2 = r.clone();
    let (a, b) = tokio::join!(
        async { r1.register("acme", "billing", content_a(), meta()).await },
        async { r2.register("acme", "billing", content_b(), meta()).await },
    );
    let h1 = a.unwrap();
    let h2 = b.unwrap();
    let mut versions = vec![h1.key.version, h2.key.version];
    versions.sort();
    assert_eq!(versions, vec![1, 2], "expected v1 and v2, got {versions:?}");
}
```

---

## 6. Schéma SQL détaillé

### 6.1 Tables

```sql
-- Migration 0001_init.sql
-- ----------------------------------------------------------------------------
-- Sprint D14 — Postgres catalog backend, initial schema.
--
-- Authoritative schema for the `efe.v1.catalog.CatalogService` six RPCs.
-- One table per logical entity ; foreign keys enforce parent_key chain
-- (audit trail forward-only).
-- ----------------------------------------------------------------------------

-- Schema migration tracking is owned by sqlx_migrations (auto-created).

-- =========================================================================
-- Table : catalogs
-- =========================================================================
-- One row per (tenant, name, version). Append-only ; deletes are rare and
-- guarded by `force` flag at the registry level.
--
-- The composite PK (tenant, name, version) :
--   - guarantees no two catalogs share the same canonical key,
--   - is dense for efficient (tenant, name)-prefix scans (list_versions),
--   - lets the (tenant) prefix scan support list_catalogs.
CREATE TABLE catalogs (
    tenant            TEXT      NOT NULL,
    name              TEXT      NOT NULL,
    version           INTEGER   NOT NULL,

    -- Content : full CatalogContent serialised as JSONB.
    -- JSONB picked over JSON for indexability and decompression cost ;
    -- TEXT was rejected because the content has nested arrays
    -- (`classes[*].canonical_sentences[*]`) and we want optional
    -- queryability for D15 audit dashboards.
    content_json      JSONB     NOT NULL,

    -- Metadata fields — stored as separate columns rather than embedded
    -- in `content_json` so we can index / filter on them without a JSONB
    -- expression index.
    created_at_unix_ms BIGINT   NOT NULL,
    created_by        TEXT      NOT NULL,        -- JWT sub claim ; "" for tests
    description       TEXT      NOT NULL DEFAULT '',
    parent_key        TEXT      NOT NULL DEFAULT '',  -- empty for v1 of a chain
    evolution_op      TEXT      NOT NULL,        -- "register" | "append_class" | …
    labels            JSONB     NOT NULL DEFAULT '{}'::jsonb,  -- BTreeMap<String,String>
    content_sha256    TEXT      NOT NULL,        -- hex SHA-256 of content_json

    -- Compiled anchors (optional — populated only for catalogs that have
    -- gone through the embedding pool). NULL = not yet compiled.
    compiled_anchors  BYTEA,    -- concatenated f32 little-endian rows
    compiled_classes  JSONB,    -- per-class blob offsets + counts
    embedder_fingerprint TEXT,  -- stamp used at compile time

    -- Audit trail.
    db_inserted_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    db_updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),

    PRIMARY KEY (tenant, name, version),
    -- The chain integrity is enforced by parent_key matching an existing
    -- canonical key when non-empty.
    CONSTRAINT catalogs_parent_chain
        CHECK (
            parent_key = ''
            OR parent_key ~ '^[a-z0-9_-]+/[a-z0-9_-]+@v\d+$'
        ),
    -- Defensive : version must match the canonical key parsing rules.
    CONSTRAINT catalogs_version_positive CHECK (version >= 1),
    -- Defensive : tenant + name must be non-empty.
    CONSTRAINT catalogs_tenant_nonempty CHECK (length(tenant) >= 1),
    CONSTRAINT catalogs_name_nonempty CHECK (length(name) >= 1)
);

COMMENT ON TABLE catalogs IS
    'Sprint D14 — versioned catalogs. Composite PK (tenant, name, version) ;
     append-only modulo `delete` at the trait level. Parent chain integrity
     guarded by check constraint (regex on parent_key).';

-- =========================================================================
-- Indexes
-- =========================================================================

-- Reverse-version index for `latest_version` lookups in list_catalogs.
-- Also accelerates the SELECT MAX(version) inside the advisory-locked
-- register transaction.
CREATE INDEX catalogs_tenant_name_version_desc_idx
    ON catalogs (tenant, name, version DESC);

-- Tenant-scoped scan for list_catalogs page iteration. The PK already
-- starts with tenant so a separate index would be redundant — but a
-- (tenant, name) covering subset speeds up the latest-version aggregation.
-- We rely on the PK directly for v0.1.14 ; revisit if list_catalogs p99
-- regresses.

-- Reference check : "is this version referenced by a successor ?"
-- The `parent_key` column is varchar ; an index on it accelerates
-- "is this row pointed to by anyone ?" needed by `delete force=false`.
CREATE INDEX catalogs_parent_key_idx
    ON catalogs (parent_key)
    WHERE parent_key <> '';

-- Audit timestamp index : optional, for D15 archival queries.
-- Only created if D15 archival lands ; commented out in D14.
-- CREATE INDEX catalogs_db_inserted_at_idx ON catalogs (db_inserted_at);

-- =========================================================================
-- Trigger : keep `db_updated_at` fresh on UPDATE.
-- =========================================================================
-- We rarely UPDATE catalogs (registrations are append-only at the trait
-- level), but `delete force=true` cascades may set rows in flight. The
-- trigger gives us a stable "last touched" stamp for ops debugging.
CREATE OR REPLACE FUNCTION catalogs_set_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.db_updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER catalogs_updated_at_trg
    BEFORE UPDATE ON catalogs
    FOR EACH ROW
    EXECUTE FUNCTION catalogs_set_updated_at();
```

### 6.2 Indexes

| Index | Columns | Justification |
|-------|---------|---------------|
| `catalogs_pkey` (auto) | `(tenant, name, version)` | Primary key ; supports prefix scans and exact lookups |
| `catalogs_tenant_name_version_desc_idx` | `(tenant, name, version DESC)` | Speeds `MAX(version)` inside register tx + latest-version in list_catalogs |
| `catalogs_parent_key_idx` (partial) | `(parent_key) WHERE parent_key <> ''` | Speeds `delete force=false` reference check |

**Pas d'index sur `created_at_unix_ms`** : pas de query qui filtre
sur ce champ en D14 ; YAGNI.

**Pas d'index JSONB** : `content_json` n'est jamais filtré côté
SQL — l'app le sérialise/désérialise en bloc. Si D15 ajoute une
recherche full-text ou par class.name, on ajoutera un GIN sur le
chemin spécifique.

### 6.3 Contraintes

| Contrainte | Type | Garantie |
|------------|------|----------|
| `PRIMARY KEY (tenant, name, version)` | composite UNIQUE NOT NULL | Pas de duplicates de canonical key |
| `catalogs_parent_chain` CHECK | regex | Parent key bien formé ou vide |
| `catalogs_version_positive` CHECK | `version >= 1` | Pas de v0 ou versions négatives |
| `catalogs_tenant_nonempty` CHECK | `length(tenant) >= 1` | Tenant non vide |
| `catalogs_name_nonempty` CHECK | `length(name) >= 1` | Name non vide |

**Pas de FOREIGN KEY** sur `parent_key`. Raison : si on a une FK
auto-référentielle, le `delete force=true` cascade demande
`ON DELETE CASCADE` qui complique le check côté trait
(`delete force=false` doit retourner `Conflict`, pas un
`fk_violation`). On préfère gérer la chaîne au niveau applicatif :
le check `force=false` → `SELECT 1 FROM catalogs WHERE parent_key = $1 LIMIT 1`
avant le `DELETE`.

### 6.4 Migrations forward-only

Layout recommandé :

```
crates/efe-server/migrations/
├── 0001_init.sql                 — schéma principal (cf. § 6.1)
├── 0002_audit_events_optional.sql — table audit_events séparée (option D15-prep)
├── 0003_indexes_phase2.sql       — index supplémentaires si bench le justifie
├── 0004_compiled_anchors_partial.sql — partial index pour compiled_anchors NOT NULL
└── 0005_quota_views.sql          — vues matérialisées per-tenant counts
```

**Conventions** :

* **Forward-only**. Pas de `down.sql`. Si un changement est trop
  risqué pour être appliqué sur prod, on crée une nouvelle migration
  qui annule (e.g. `DROP INDEX foo`) — version contrôlée.
* **Idempotent quand possible**. `CREATE TABLE IF NOT EXISTS`,
  `CREATE INDEX IF NOT EXISTS` (Postgres ≥ 9.5).
* **Tests parity** : la suite testcontainers applique TOUTES les
  migrations à un PG vide puis run les tests. Si une migration casse
  l'état, le test fail.
* **Production playbook** : migration appliquée par `efe-server` au
  boot via `sqlx::migrate!()` macro. Si la migration échoue, le
  binaire fail-fast (pas de "best effort"). Cf. § 9.

---

## 7. Mapping `CatalogRegistry` trait → SQL

### 7.1 `register`

**Pseudo-code** :

```rust
async fn register(
    &self,
    tenant: &str,
    name: &str,
    content: CatalogContent,
    metadata: CatalogMetadata,
) -> Result<RegisteredCatalog, CatalogError> {
    // Step 0 — policy validation (cheap, applied first).
    self.policy.validate_content(&content)?;
    let mut tx = self.pool.begin().await.map_err(internal_err)?;

    // Step 1 — acquire advisory lock for this (tenant, name).
    let lock_id = lock_id_for(tenant, name);
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(lock_id)
        .execute(&mut *tx)
        .await
        .map_err(internal_err)?;

    // Step 2 — quota checks (after lock so the count is consistent).
    let n_names: i64 = sqlx::query_scalar(
        "SELECT count(DISTINCT name) FROM catalogs WHERE tenant = $1"
    ).bind(tenant).fetch_one(&mut *tx).await.map_err(internal_err)?;
    if (n_names as u32) >= self.policy.max_names_per_tenant {
        return Err(CatalogError::QuotaExceeded(format!(
            "tenant={tenant} reached max_names_per_tenant={}",
            self.policy.max_names_per_tenant
        )));
    }

    let n_versions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM catalogs WHERE tenant = $1 AND name = $2"
    ).bind(tenant).bind(name).fetch_one(&mut *tx).await.map_err(internal_err)?;
    if (n_versions as u32) >= self.policy.max_versions_per_name {
        return Err(CatalogError::QuotaExceeded(format!(
            "(tenant={tenant}, name={name}) reached max_versions_per_name={}",
            self.policy.max_versions_per_name
        )));
    }

    // Step 3 — compute next version under lock.
    let max_v: Option<i32> = sqlx::query_scalar(
        "SELECT MAX(version) FROM catalogs WHERE tenant = $1 AND name = $2"
    ).bind(tenant).bind(name).fetch_one(&mut *tx).await.map_err(internal_err)?;
    let next_v: i32 = max_v.map(|v| v + 1).unwrap_or(1);

    // Step 4 — compute content_sha256 deterministically.
    let content_json = serde_json::to_value(&content).map_err(internal_err_je)?;
    let sha = sha256_hex_of_json(&content_json);

    // Step 5 — INSERT.
    sqlx::query(r#"
        INSERT INTO catalogs (
            tenant, name, version,
            content_json,
            created_at_unix_ms, created_by, description,
            parent_key, evolution_op, labels, content_sha256
        ) VALUES (
            $1, $2, $3,
            $4,
            $5, $6, $7,
            $8, $9, $10, $11
        )
    "#)
    .bind(tenant).bind(name).bind(next_v)
    .bind(&content_json)
    .bind(metadata.created_at_unix_ms)
    .bind(&metadata.created_by)
    .bind(&metadata.description)
    .bind(&metadata.parent_key) // empty for register
    .bind("register")
    .bind(serde_json::to_value(&metadata.labels).map_err(internal_err_je)?)
    .bind(&sha)
    .execute(&mut *tx)
    .await
    .map_err(map_pg_error)?;

    // Step 6 — audit emit (tracing, NOT a SQL row in D14 ; D15 may
    // add an `audit_events` table).
    self.audit.emit_register(tenant, name, next_v as u32, &metadata.created_by);

    tx.commit().await.map_err(internal_err)?;

    Ok(RegisteredCatalog {
        key: CatalogKey { tenant: tenant.to_owned(), name: name.to_owned(), version: next_v as u32 },
        content,
        metadata: CatalogMetadata { content_sha256: ContentDigest(sha), ..metadata },
        compiled: None,
    })
}
```

**Error mapping** (`map_pg_error`) :

| Postgres `code` | `CatalogError` |
|-----------------|----------------|
| `23505` (unique_violation) | `Conflict("(tenant, name, version) already exists ; concurrent register won the race")` |
| `23514` (check_violation) | `InvalidArgument("schema constraint violated: {detail}")` |
| `23502` (not_null_violation) | `Internal("missing required field — schema bug")` |
| `40001` (serialization_failure) | `Internal("transaction conflict ; retry recommended")` |
| any other | `Internal(_)` (logged server-side, opaque message to client per audit fix T-12bis-2) |

**Note importante sur les quotas** : on les check *dans la
transaction sous le lock* pour que deux registers concurrents qui
remplissent simultanément ne dépassent pas la limite. L'overhead
est de 2 `count(*)` requêtes par register — manageable car
indexes le permettent (cf. § 6.2).

### 7.2 `evolve`

```rust
async fn evolve(
    &self,
    from: &CatalogKey,
    op: EvolveOp,
    metadata_override: CatalogMetadata,
) -> Result<RegisteredCatalog, CatalogError> {
    let mut tx = self.pool.begin().await.map_err(internal_err)?;
    let lock_id = lock_id_for(&from.tenant, &from.name);
    sqlx::query("SELECT pg_advisory_xact_lock($1)").bind(lock_id)
        .execute(&mut *tx).await.map_err(internal_err)?;

    // Step 1 — fetch the source version.
    let row: Option<CatalogRow> = sqlx::query_as!(CatalogRow, r#"
        SELECT tenant, name, version, content_json, …
        FROM catalogs
        WHERE tenant = $1 AND name = $2 AND version = $3
    "#, from.tenant, from.name, from.version as i32)
        .fetch_optional(&mut *tx)
        .await.map_err(internal_err)?;
    let src = row.ok_or_else(|| CatalogError::NotFound(format!(
        "{}/{}@v{}", from.tenant, from.name, from.version
    )))?;

    // Step 2 — apply the op to the source content.
    let mut new_content: CatalogContent = serde_json::from_value(src.content_json)
        .map_err(internal_err_je)?;
    apply_evolve_op(&mut new_content, &op)?; // returns InvalidArgument on bad op
    self.policy.validate_content(&new_content)?;

    // Step 3 — quota check on versions.
    let n_versions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM catalogs WHERE tenant=$1 AND name=$2"
    ).bind(&from.tenant).bind(&from.name).fetch_one(&mut *tx)
     .await.map_err(internal_err)?;
    if (n_versions as u32) >= self.policy.max_versions_per_name {
        return Err(CatalogError::QuotaExceeded(format!(
            "(tenant={}, name={}) reached max_versions_per_name={}",
            from.tenant, from.name, self.policy.max_versions_per_name
        )));
    }

    // Step 4 — next version = MAX(version) + 1, NOT src.version + 1.
    // This handles the case where multiple concurrent evolves race :
    // the second waiter sees v3 (because the first wrote v2 from v1)
    // and picks v4. Without this, two waiters from v1 would both try v2.
    let max_v: i32 = sqlx::query_scalar(
        "SELECT MAX(version) FROM catalogs WHERE tenant=$1 AND name=$2"
    ).bind(&from.tenant).bind(&from.name).fetch_one(&mut *tx)
     .await.map_err(internal_err)?;
    let next_v = max_v + 1;

    // Step 5 — compute content_sha256 + parent_key.
    let new_content_json = serde_json::to_value(&new_content).map_err(internal_err_je)?;
    let sha = sha256_hex_of_json(&new_content_json);
    let parent_key = format!("{}/{}@v{}", from.tenant, from.name, from.version);

    // Step 6 — INSERT.
    sqlx::query(r#"
        INSERT INTO catalogs (tenant, name, version, content_json, …)
        VALUES ($1, $2, $3, $4, …)
    "#)
    .bind(&from.tenant).bind(&from.name).bind(next_v)
    .bind(&new_content_json)
    .bind(metadata_override.created_at_unix_ms)
    .bind(&metadata_override.created_by)
    .bind(&metadata_override.description)
    .bind(&parent_key)
    .bind(op.audit_tag())
    .bind(serde_json::to_value(&metadata_override.labels).map_err(internal_err_je)?)
    .bind(&sha)
    .execute(&mut *tx).await.map_err(map_pg_error)?;

    self.audit.emit_evolve(&from.tenant, &from.name, next_v as u32, &op, &metadata_override.created_by);
    tx.commit().await.map_err(internal_err)?;

    Ok(RegisteredCatalog { … })
}
```

**Concurrency property** : si deux clients concurrents font
`evolve(from=v1)` simultanément, l'advisory lock les sérialise. Le
premier obtient `next_v = 2`, le second `next_v = 3` (parce qu'il
voit la nouvelle row du premier après le COMMIT ; postgres lit
sous SI naturel). Tous deux ont `parent_key = 'tenant/name@v1'` (du
moment qu'ils partent du même from). C'est le comportement spec
D12.

### 7.3 `get`

```rust
async fn get(
    &self,
    key: &CatalogKey,
    include_compiled: bool,
) -> Result<RegisteredCatalog, CatalogError> {
    let row: Option<CatalogRow> = sqlx::query_as!(CatalogRow, r#"
        SELECT tenant, name, version, content_json,
               created_at_unix_ms, created_by, description,
               parent_key, evolution_op, labels, content_sha256,
               compiled_anchors, compiled_classes, embedder_fingerprint
        FROM catalogs
        WHERE tenant = $1 AND name = $2 AND version = $3
    "#, key.tenant, key.name, key.version as i32)
        .fetch_optional(&self.pool)
        .await.map_err(internal_err)?;
    let r = row.ok_or_else(|| CatalogError::NotFound(format!(
        "{}/{}@v{}", key.tenant, key.name, key.version
    )))?;

    let content: CatalogContent = serde_json::from_value(r.content_json)
        .map_err(internal_err_je)?;
    let metadata = CatalogMetadata {
        created_at_unix_ms: r.created_at_unix_ms,
        created_by:        r.created_by,
        description:       r.description,
        parent_key:        r.parent_key,
        evolution_op:      r.evolution_op,
        labels:            serde_json::from_value(r.labels).map_err(internal_err_je)?,
        content_sha256:    ContentDigest(r.content_sha256),
    };
    let compiled = if include_compiled && r.compiled_anchors.is_some() {
        Some(deserialize_compiled(&r)?)
    } else {
        None
    };

    Ok(RegisteredCatalog { key: key.clone(), content, metadata, compiled })
}
```

**Pas de transaction** : un single SELECT n'a pas besoin d'une
transaction explicite ; sqlx fournit déjà l'isolation read-committed
par défaut.

**Note sur `include_compiled`** : si `compiled_anchors` est NULL en
DB, on retourne `compiled=None` même si `include_compiled=true` —
le client comprend que le catalog n'a pas encore été compilé.

### 7.4 `list_catalogs`

```rust
async fn list_catalogs(
    &self,
    tenant: &str,
    page_size: u32,
    page_token: Option<&str>,
) -> Result<CatalogPage, CatalogError> {
    let page_size_clamped = page_size.clamp(1, 500) as i64;
    let after_name: String = match page_token {
        None => String::new(),
        Some(t) => decode_page_token(t)?, // base64-encoded last name in previous page
    };

    // We aggregate latest_version + counts per (tenant, name).
    // GROUP BY (tenant, name), ORDER BY name, LIMIT page_size + 1
    // (the +1 is to detect "is there a next page ?" without a separate count).
    let rows = sqlx::query!(r#"
        SELECT
            name,
            MAX(version)::INTEGER AS "latest_version!: i32",
            (SELECT jsonb_array_length(content_json -> 'classes')
             FROM catalogs c2
             WHERE c2.tenant = $1 AND c2.name = c.name AND c2.version = MAX(c.version)
            ) AS "num_classes!: i32",
            (SELECT COALESCE(SUM(jsonb_array_length(class -> 'canonical_sentences')), 0)
             FROM catalogs c2,
                  jsonb_array_elements(c2.content_json -> 'classes') class
             WHERE c2.tenant = $1 AND c2.name = c.name AND c2.version = MAX(c.version)
            )::INTEGER AS "num_anchors!: i32"
        FROM catalogs c
        WHERE tenant = $1 AND name > $2
        GROUP BY tenant, name
        ORDER BY name ASC
        LIMIT $3
    "#, tenant, after_name, page_size_clamped + 1)
        .fetch_all(&self.pool)
        .await.map_err(internal_err)?;

    let mut catalogs: Vec<CatalogRefSummary> = rows.iter().take(page_size_clamped as usize)
        .map(|r| CatalogRefSummary {
            tenant: tenant.to_owned(),
            name:   r.name.clone(),
            latest_version: r.latest_version as u32,
            num_classes:    r.num_classes as u32,
            num_anchors:    r.num_anchors as u32,
        })
        .collect();
    let next_page_token = if rows.len() > page_size_clamped as usize {
        encode_page_token(&catalogs.last().unwrap().name)
    } else {
        String::new()
    };
    Ok(CatalogPage { catalogs, next_page_token })
}
```

**Note sur la query** : on agrège `MAX(version)` + comptes
JSONB-extraits du content. C'est lourd ; en v0.1.14 on accepte.
Si bench montre que `list_catalogs` p99 dépasse les budgets, deux
optimisations :

1. **Materialized view** (D14 ou D15) :
   ```sql
   CREATE MATERIALIZED VIEW catalogs_summary AS
   SELECT tenant, name, MAX(version) AS latest_version, …
   FROM catalogs GROUP BY tenant, name;
   REFRESH MATERIALIZED VIEW CONCURRENTLY catalogs_summary;
   ```
2. **Denormalised `latest` table** maintenue par trigger sur INSERT.

→ tracked **DT-D14.2** dans le sprint closure.

**Page token format** : `base64(last_name)`. Opaque pour le client,
décodable en interne. Validation : si le décodage échoue, on retourne
`InvalidArgument("malformed page_token")`.

### 7.5 `list_versions`

```rust
async fn list_versions(
    &self,
    tenant: &str,
    name: &str,
) -> Result<Vec<CatalogKey>, CatalogError> {
    let rows = sqlx::query!(r#"
        SELECT version FROM catalogs
        WHERE tenant = $1 AND name = $2
        ORDER BY version ASC
    "#, tenant, name)
        .fetch_all(&self.pool)
        .await.map_err(internal_err)?;
    if rows.is_empty() {
        return Err(CatalogError::NotFound(format!("{tenant}/{name}")));
    }
    Ok(rows.into_iter().map(|r| CatalogKey {
        tenant: tenant.to_owned(),
        name:   name.to_owned(),
        version: r.version as u32,
    }).collect())
}
```

**Note** : on retourne `NotFound` si aucune version, pas une liste
vide. Cohérent avec `InMemoryRegistry::list_versions` (test
`list_versions_returns_not_found_for_unknown_chain`).

### 7.6 `delete`

```rust
async fn delete(
    &self,
    key: &CatalogKey,
    force: bool,
    principal: &str,
) -> Result<(), CatalogError> {
    let mut tx = self.pool.begin().await.map_err(internal_err)?;
    let lock_id = lock_id_for(&key.tenant, &key.name);
    sqlx::query("SELECT pg_advisory_xact_lock($1)").bind(lock_id)
        .execute(&mut *tx).await.map_err(internal_err)?;

    let canonical = format!("{}/{}@v{}", key.tenant, key.name, key.version);

    if !force {
        // Reference check : is this version pointed to by a successor?
        let referenced: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM catalogs WHERE parent_key = $1)"
        ).bind(&canonical)
         .fetch_one(&mut *tx).await.map_err(internal_err)?;
        if referenced {
            return Err(CatalogError::Conflict(format!(
                "{canonical} is referenced by a successor ; use force=true to cascade"
            )));
        }
    }

    // Step 2 — collect all transitive descendants if force=true.
    // (recursive CTE chains parent_key.)
    let to_delete: Vec<i32> = if force {
        sqlx::query_scalar!(r#"
            WITH RECURSIVE chain(version) AS (
                SELECT version FROM catalogs
                WHERE tenant = $1 AND name = $2 AND version = $3
                UNION
                SELECT c.version FROM catalogs c
                JOIN chain ch ON c.parent_key =
                    $1 || '/' || $2 || '@v' || ch.version
                WHERE c.tenant = $1 AND c.name = $2
            )
            SELECT version FROM chain
        "#, key.tenant, key.name, key.version as i32)
        .fetch_all(&mut *tx).await.map_err(internal_err)?
    } else {
        vec![key.version as i32]
    };

    if to_delete.is_empty() {
        return Err(CatalogError::NotFound(canonical));
    }

    // Step 3 — DELETE rows.
    let rows_affected = sqlx::query(r#"
        DELETE FROM catalogs
        WHERE tenant = $1 AND name = $2 AND version = ANY($3::integer[])
    "#)
    .bind(&key.tenant).bind(&key.name).bind(&to_delete)
    .execute(&mut *tx).await.map_err(internal_err)?
    .rows_affected();

    if rows_affected == 0 {
        return Err(CatalogError::NotFound(canonical));
    }

    self.audit.emit_delete(&key.tenant, &key.name, key.version, force, principal);
    tx.commit().await.map_err(internal_err)?;
    Ok(())
}
```

**Edge cases** :

* `delete` d'une row inexistante → `NotFound`.
* `delete force=false` sur une row référencée → `Conflict`.
* `delete force=true` sur une row inexistante → `NotFound` (le
  CTE retourne vide).
* `delete force=true` qui cascade `N` rows → toutes supprimées en
  une seule transaction. Les locks d'autres `(tenant, name)` ne
  sont pas pris.

---

## 8. Connection pool : sizing, lifecycle, leak detection

### 8.1 Sizing rationale

**Règle** : le pool doit pouvoir absorber le **degré de
concurrence simultané** des handlers. Pour EFE :

* Un `efe-server` accepte jusqu'à `max_concurrent_streams = 1024`
  streams gRPC.
* Chaque stream peut faire 1 RPC catalog → 1 transaction PG
  pendante (register/evolve/delete).
* Si tous les streams concurrents font register, on a 1024 tx
  pendantes — bien plus que ce que PG peut soutenir.
* PG default `max_connections = 100`. Au-delà, `connect()` fail.

**Sizing pratique** :

| Type de déploiement | Pool size par réplique | Rationale |
|---------------------|:---------------------:|-----------|
| Tier 1 (single replica, dev/staging) | 8 | Latency-bound ; 8 cores × 1 tx/core = 8 |
| Tier 2 (3 replicas, prod) | 8 par réplique × 3 = 24 total | Tient sous PG max_connections=100 avec marge |
| Tier 3 (10+ replicas, hyper) | 4 par réplique avec PgBouncer en transaction-pooling | PgBouncer multiplexe |

**Defaults D14** : `max_connections = 8` par réplique.
Override via `EFE_PG_POOL_MAX_CONNECTIONS`.

### 8.2 Pool config

```rust
let pool = sqlx::postgres::PgPoolOptions::new()
    .max_connections(cfg.pool_max_connections)
    .min_connections(cfg.pool_min_connections)
    .acquire_timeout(Duration::from_millis(cfg.pool_acquire_timeout_ms))
    .max_lifetime(Some(Duration::from_secs(cfg.pool_max_lifetime_s)))
    .idle_timeout(Some(Duration::from_secs(cfg.pool_idle_timeout_s)))
    .test_before_acquire(true)  // ping each connection before handing it out
    .connect(&cfg.url).await?;
```

**Defaults** :

| Field | Default | Env var | Justification |
|-------|--------:|---------|---------------|
| `pool_max_connections` | 8 | `EFE_PG_POOL_MAX_CONNECTIONS` | cf. § 8.1 |
| `pool_min_connections` | 2 | `EFE_PG_POOL_MIN_CONNECTIONS` | Garder 2 connexions chaudes pour la latence p99 |
| `pool_acquire_timeout_ms` | 5000 | `EFE_PG_POOL_ACQUIRE_TIMEOUT_MS` | 5 s = ~3× p99 PG connect over WAN |
| `pool_max_lifetime_s` | 1800 (30 min) | `EFE_PG_POOL_MAX_LIFETIME_S` | Recycle connexions pour éviter PG plan cache poisoning |
| `pool_idle_timeout_s` | 600 (10 min) | `EFE_PG_POOL_IDLE_TIMEOUT_S` | Libérer les ressources si l'app est idle |
| `test_before_acquire` | true | `EFE_PG_POOL_TEST_BEFORE_ACQUIRE` | Robustesse en cas de PG restart en arrière-plan |

### 8.3 Connection leak detection

sqlx lui-même n'a pas de leak detection built-in. On instrumente
**au niveau handler** :

* Acquire timeout exceeded → log `target: "efe.catalog.postgres"
  level: ERROR` + counter `efe_pg_acquire_timeout_total{tenant}`.
* Pool size gauge : `efe_pg_pool_size{state="active"|"idle"}`
  exposé via `pool.size()` et `pool.num_idle()`.
* Per-RPC span attribute : `pool_acquire_ms` mesuré au début de
  chaque méthode du registry.

**Acquire timeout — failure mode escalation** :

```
PG cluster overload OR network partition
  → acquire timeout 5 s
  → registry method returns CatalogError::Internal("…")
  → handler maps to Status::Internal("internal server error")  (sanitized)
  → client sees retryable error
  → client retries (with backoff)
  → … if persistent, alert PagerDuty via D13 alert.
```

---

## 9. Migrations : sqlx migrate, forward-only

### 9.1 Layout

```
crates/efe-server/migrations/
├── 0001_init.sql                     ← § 6.1
├── 0002_audit_events_optional.sql    ← table séparée pour D15-prep
├── 0003_indexes_phase2.sql           ← post-bench tuning
└── README.md                         ← ops playbook
```

### 9.2 CI integration

**Compile-time check** : on commit `.sqlx/` avec les query metadata
généré par `cargo sqlx prepare`. CI run :

```yaml
- name: Verify sqlx query metadata
  run: |
    cd crates/efe-server
    cargo sqlx prepare --check
```

Drift (un dev a modifié une query mais oublié `prepare`) → CI fail.

**Migration application au boot** :

```rust
// runtime.rs::Server::build (extrait D14)
if matches!(config.catalog.backend, CatalogBackend::Postgres) {
    let url = config.catalog.postgres.url();
    let migrator = sqlx::migrate!("../migrations");
    let pool = build_pg_pool(&config.catalog.postgres).await?;
    migrator.run(&pool).await
        .map_err(|e| anyhow!("migration failed at boot: {e}"))?;
}
```

Si la migration échoue → fail-fast, le binaire n'expose pas le
service catalog. Pas de "best effort".

### 9.3 Production migration playbook

**Forward-only**, donc l'ordre canonique est :

1. **Dev** : développeur écrit `00NN_my_change.sql`, run
   `cargo sqlx prepare`, commit.
2. **CI** : passe la migration sur un PG vide via testcontainers.
3. **Staging** : déploiement Helm — `efe-server` migre au boot.
4. **Prod** : Helm rolling upgrade — chaque réplique tente migrate.
   La table `_sqlx_migrations` empêche la double-application.

**Si une migration prod échoue** :

- Le pod fail-fast → l'orchestrateur relance.
- Si le 2e essai échoue : l'opérateur intervient — soit revert le
  binaire (rollback Helm), soit corrige la migration et déploie
  une nouvelle migration "fix" (forward-only).
- **Jamais un `down.sql`** (ils sont source de bugs : la migration
  reverse n'a pas le même chemin testé que la forward).

**Lock contention** : `sqlx::migrate!()` utilise un advisory lock
spécifique pour serialiser les boot migrations entre répliques.
Pas de double-application possible même si N pods démarrent en
même temps.

---

## 10. Tests : testcontainers + integration suite

### 10.1 Test strategy

| Type de test | Cible | Outil | Fichier |
|--------------|-------|-------|---------|
| Unit | Helpers (lock_id, sha256, page_token) | `cargo test --lib` | `crates/efe-server/src/catalog/postgres.rs::tests` |
| Integration trait | Trait contract sur PG réel | `testcontainers` + `tokio::test` | `crates/efe-server/tests/d14_postgres_catalog_trait.rs` |
| Integration RPC | 6 RPC end-to-end via tonic + PG | `testcontainers` + `Server::builder` | `crates/efe-server/tests/d14_postgres_catalog_rpc.rs` |
| Concurrency | Advisory lock + monotone | `tokio::join!` × 8 | dans `d14_postgres_catalog_trait.rs` |
| Migration | Migrations apply on empty PG | `sqlx::migrate!()` | `crates/efe-server/tests/d14_migrations.rs` |
| Performance | Latency budget vs in-memory | Criterion bench | `crates/efe-server/benches/postgres_rpc.rs` |
| Helm rendering | values/configmap/secret | Python | `scripts/tests/test_d14_postgres_helm_render.py` |
| Runtime wire-up | runtime.rs branche Postgres ouverte | Python | `scripts/tests/test_d14_runtime_wireup.py` |

### 10.2 Unit tests (mock pool)

Pas de mock pool — sqlx ne mock pas facilement. Les tests unitaires
testent les **helpers purs** : `lock_id_for`, `sha256_hex_of_json`,
`encode_page_token` / `decode_page_token`, `apply_evolve_op`, etc.

### 10.3 Integration tests (testcontainers PG)

```rust
// crates/efe-server/tests/d14_postgres_catalog_trait.rs

use testcontainers::{clients, images::postgres::Postgres};

struct TestPg {
    container: testcontainers::Container<'static, Postgres>,
    config: CatalogPostgresConfig,
}

impl TestPg {
    async fn start() -> Self {
        // Use a process-static client so the container outlives the test.
        static CLIENT: once_cell::sync::Lazy<clients::Cli> =
            once_cell::sync::Lazy::new(clients::Cli::default);
        let image = Postgres::default()
            .with_db_name("efe_test")
            .with_user("efe")
            .with_password("efe");
        let container = CLIENT.run(image);
        let port = container.get_host_port_ipv4(5432);
        let config = CatalogPostgresConfig {
            url: format!("postgres://efe:efe@127.0.0.1:{}/efe_test", port),
            ssl_mode: SslMode::Prefer,
            pool_max_connections: 4,
            pool_min_connections: 1,
            pool_acquire_timeout_ms: 5000,
            pool_max_lifetime_s: 1800,
            pool_idle_timeout_s: 600,
            pool_test_before_acquire: true,
        };
        Self { container, config }
    }
}

#[tokio::test]
async fn register_creates_v1_persists_and_returns() {
    let pg = TestPg::start().await;
    let policy = CatalogPolicy::default();
    let audit  = AuditEmitter::new();
    let registry = PostgresCatalogRegistry::connect(&pg.config, policy, audit).await.unwrap();

    let metadata = sample_metadata("alice@acme");
    let result = registry.register("acme", "billing", sample_content(), metadata).await.unwrap();
    assert_eq!(result.key.version, 1);

    // Re-fetch via get to confirm persistence.
    let fetched = registry.get(&result.key, false).await.unwrap();
    assert_eq!(fetched.content.classes.len(), result.content.classes.len());
}
```

**Caractéristiques importantes** :

* Une instance PG par `TestPg::start()` (cher mais hermétique).
  Pour la perf, on regroupe les tests qui partagent un état (e.g.
  list_catalogs après plusieurs registers) en une seule fonction.
* `clients::Cli` est `Lazy::new` pour éviter la recréation Docker.
* La registry instancie sa propre migration sub-flow ; chaque
  test démarre avec un schéma propre.

**Suite minimale D14 (~30 tests)** :

| # | Section | Test |
|--:|---------|------|
| 1 | Happy paths (6) | register/evolve/get/list/list_versions/delete |
| 2 | Mappings d'erreur (6) | NotFound, InvalidArgument, Conflict, QuotaExceeded, Internal sanitization, parent chain check |
| 3 | Concurrency (4) | concurrent register same key → distinct versions ; concurrent evolve same from_key → distinct versions ; concurrent register different keys → both succeed parallel ; advisory lock release on rollback |
| 4 | Persistence (3) | restart registry instance → previous data survives ; multi-pool same DB sees same data ; backup/restore round-trip |
| 5 | Quotas (4) | max_names ; max_versions ; max_classes ; max_sentences |
| 6 | D11 isolation (3) | cross-tenant get → NotFound (since tenant filter in WHERE) ; cross-tenant list → empty ; cross-tenant delete → NotFound |
| 7 | Migrations (2) | apply on empty PG ; idempotent on already-migrated PG |
| 8 | Page token (2) | round-trip ; malformed → InvalidArgument |

### 10.4 Concurrency tests

```rust
#[tokio::test]
async fn concurrent_register_same_key_distinct_versions() {
    let pg = TestPg::start().await;
    let r = PostgresCatalogRegistry::connect(&pg.config, default_policy(), default_audit()).await.unwrap();

    let mut handles = Vec::new();
    for _ in 0..8 {
        let r = r.clone();
        handles.push(tokio::spawn(async move {
            r.register("acme", "billing", sample_content(), sample_metadata("svc")).await
        }));
    }
    let mut versions = Vec::new();
    for h in handles {
        let res = h.await.unwrap();
        versions.push(res.unwrap().key.version);
    }
    versions.sort();
    assert_eq!(versions, vec![1, 2, 3, 4, 5, 6, 7, 8]);
}
```

8 register concurrents → versions 1..8 strictement.

---

## 11. Performance budget vs `InMemoryRegistry`

### 11.1 Reference SLO

`InMemoryRegistry` p99 latency (D12-bis-CT-1, mesuré sur loopback,
M3 Pro) :

| RPC | p99 |
|-----|----:|
| `RegisterCatalog` | **5 ms** |
| `EvolveCatalog` (8 concurrent) | **12 ms** |
| `GetCatalog` p50 | **1 ms** |

### 11.2 `PostgresCatalogRegistry` budget

Déploiement de référence : PG sur la même machine que `efe-server`,
loopback TCP, no TLS. Bench machine : M3 Pro, PG 15.

Estimation p99 :

| RPC | Budget D14 | Source |
|-----|-----------:|--------|
| `RegisterCatalog` | **20 ms** | ~5 ms acquire + ~3 ms advisory lock + ~5 ms count(*) × 2 + ~5 ms INSERT + commit |
| `EvolveCatalog` (8 concurrent) | **40 ms** | ~5 ms acquire + ~3 ms lock + ~3 ms SELECT source + ~3 ms count + ~5 ms INSERT + parallelism overhead |
| `GetCatalog` p50 | **3 ms** | acquire reused + single SELECT |

**Si bench dépasse les budgets** : 3 leviers :

1. **Statement caching** : sqlx fait du prepared-statement caching
   par connexion. Vérifier que `query!()` macro génère bien des
   prepared statements.
2. **Materialized view** pour `list_catalogs` (cf. § 7.4 DT-D14.2).
3. **PgBouncer** transaction-pooling devant PG : amortit le coût
   acquire à zéro pour les apps qui n'ouvrent pas de tx longue.

### 11.3 Mitigation strategies

**Si latence inacceptable** :

| Symptôme | Mitigation |
|----------|------------|
| `register` p99 > 50 ms | Profile : check si advisory lock est le bottleneck (DataDog APM) ; sinon connection pool size insuffisant |
| `list_catalogs` p99 > 50 ms | Switch à materialized view (cf. § 7.4 DT-D14.2) |
| `get` p50 > 10 ms | PG plan cache invalidation ; `EXPLAIN ANALYZE` pour vérifier que le plan utilise l'index PK |
| Random spikes | PgBouncer en transaction-pooling devant PG ; réduit le coût acquire à ~0 |

---

## 12. Observabilité D13 — instruments ajoutés

### 12.1 Connection pool gauges

```rust
// efe_telemetry/src/metrics/d14_instruments.rs (NEW)

prometheus_register_or_get_gauge!(
    pg_pool_active_connections,
    "efe_pg_pool_active_connections",
    "Number of currently in-use Postgres connections per pool.",
    &["pool"]
);

prometheus_register_or_get_gauge!(
    pg_pool_idle_connections,
    "efe_pg_pool_idle_connections",
    "Number of currently idle Postgres connections per pool.",
    &["pool"]
);
```

Mise à jour : un task background (cf. § 8.3) tick chaque 10 s :

```rust
async fn pool_metrics_ticker(pool: PgPool) {
    let mut interval = tokio::time::interval(Duration::from_secs(10));
    loop {
        interval.tick().await;
        let active = (pool.size() as i64) - (pool.num_idle() as i64);
        d14_instruments::pg_pool_active_connections()
            .with_label_values(&["catalog"])
            .set(active);
        d14_instruments::pg_pool_idle_connections()
            .with_label_values(&["catalog"])
            .set(pool.num_idle() as i64);
    }
}
```

### 12.2 Query latency histograms

```rust
prometheus_register_or_get_histogram_vec!(
    pg_query_duration_seconds,
    "efe_pg_query_duration_seconds",
    "Duration of individual SQL queries to the catalog database.",
    &["op", "result"],  // op = register|evolve|get|list_catalogs|list_versions|delete ; result = ok|error
    vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0]
);
```

Au sein de chaque `PostgresCatalogRegistry::*` méthode :

```rust
let start = Instant::now();
let res = run_register_inner(...).await;
let label = if res.is_ok() { "ok" } else { "error" };
d14_instruments::pg_query_duration_seconds()
    .with_label_values(&["register", label])
    .observe(start.elapsed().as_secs_f64());
res
```

### 12.3 Transaction failure counters

```rust
prometheus_register_or_get_counter_vec!(
    pg_tx_failures_total,
    "efe_pg_tx_failures_total",
    "Postgres transaction failures by error code.",
    &["op", "code"]
);
```

Map sqlx `Database::code()` → counter increment :

```rust
fn map_pg_error_with_metric(op: &str, e: sqlx::Error) -> CatalogError {
    if let sqlx::Error::Database(de) = &e {
        let code = de.code().unwrap_or_default().to_string();
        d14_instruments::pg_tx_failures_total()
            .with_label_values(&[op, &code])
            .inc();
    }
    map_pg_error(e)
}
```

### 12.4 Migration state gauge

```rust
prometheus_register_or_get_gauge!(
    pg_migration_version,
    "efe_pg_migration_version",
    "The latest applied migration version. -1 = no migration applied yet."
);
```

Set au boot après `migrator.run()` :

```rust
let last = sqlx::query_scalar!("SELECT MAX(version) FROM _sqlx_migrations")
    .fetch_optional(&pool).await?;
d14_instruments::pg_migration_version().set(last.unwrap_or(-1));
```

### 12.5 Spans

```rust
// efe_telemetry/src/spans.rs — D14 additions
pub const PG_REGISTER:        &str = "efe.pg.register";
pub const PG_EVOLVE:          &str = "efe.pg.evolve";
pub const PG_GET:             &str = "efe.pg.get";
pub const PG_LIST_CATALOGS:   &str = "efe.pg.list_catalogs";
pub const PG_LIST_VERSIONS:   &str = "efe.pg.list_versions";
pub const PG_DELETE:          &str = "efe.pg.delete";
pub const PG_MIGRATE:         &str = "efe.pg.migrate";
```

---

## 13. Auth + tenant isolation D11 — défense en profondeur SQL

### 13.1 Handler-level (recap)

D12-bis enforce :

```rust
fn assert_tenant_match(auth_tenant: &str, claimed_tenant: &str) -> Result<(), Status> {
    if auth_tenant == claimed_tenant { Ok(()) } else {
        Err(Status::permission_denied(format!(
            "tenant mismatch: JWT claims {auth_tenant:?}, request claims {claimed_tenant:?}"
        )))
    }
}
```

Si le handler est compromis (RCE bug), l'invariant peut être
contourné. **D14 le double en SQL** : toute requête a `WHERE tenant = $1`,
le tenant venant de la requête (qui passe par le handler ; donc
chaîné, mais audit-able).

### 13.2 SQL-level WHERE tenant = $1

| RPC | Where clause | Effect of bypass |
|-----|--------------|-------------------|
| `register` | INSERT with `tenant = $1` | Si `$1` est cross-tenant, on insère sous un mauvais tenant — visible dans dashboards D13 (label `tenant`) |
| `evolve` | SELECT … WHERE tenant=$1 AND name=$2 AND version=$3 | Cross-tenant `from` → NotFound |
| `get` | SELECT … WHERE tenant=$1 AND name=$2 AND version=$3 | Cross-tenant key → NotFound |
| `list_catalogs` | SELECT … WHERE tenant=$1 | Cross-tenant → empty list |
| `list_versions` | SELECT … WHERE tenant=$1 AND name=$2 | Cross-tenant → NotFound |
| `delete` | DELETE … WHERE tenant=$1 AND name=$2 AND version=$3 | Cross-tenant → 0 rows affected → NotFound |

**Le handler reste responsable du `assert_tenant_match`**, mais
même si on l'oublie ou on a un bug, la registry SQL ne peut pas
fuiter de données — elle reçoit `tenant=$1` du caller et filtre
strictement.

### 13.3 Optional: PostgreSQL Row-Level Security

**Optionnel D14, recommandé D17+** :

```sql
ALTER TABLE catalogs ENABLE ROW LEVEL SECURITY;

CREATE POLICY catalogs_tenant_isolation ON catalogs
    USING (tenant = current_setting('efe.tenant_id'));
```

Le runtime fait `SET LOCAL efe.tenant_id = $1` avant chaque tx.
Garantit que **aucune requête, même sans WHERE**, ne fuit le
tenant.

**Pas livré D14** : risque de complexité ops + perf overhead. Le
WHERE explicite suffit pour v0.1.14.

→ **DT-D14.3** : implémenter RLS en D17+.

---

## 14. Backup / restore / point-in-time recovery

### 14.1 Backup strategy

**Out-of-scope strict pour le code D14** ; documentation ops
livrée pour les opérateurs.

**Recommandation D14** :

| Tier | Backup tool | RPO | RTO |
|------|-------------|----:|----:|
| Tier 1 (dev) | None (re-register manuel) | n/a | n/a |
| Tier 2 (staging) | `pg_dump` quotidien via cron | 24 h | 1 h |
| Tier 3 (prod) | `pgBackRest` + WAL archiving | 5 min (PITR) | 15 min |

### 14.2 Restore procedure

```bash
# Tier 3 example
pgbackrest --stanza=efe-catalog --type=time \
    --target="2026-04-27 10:00:00+00" restore
```

Après restore, **vérifier l'intégrité catalogue** :

```bash
# Compte cohérent
psql efe_catalog_db -c "SELECT count(*) FROM catalogs"

# Aucune row sans content_sha256 (devrait être impossible)
psql efe_catalog_db -c "SELECT count(*) FROM catalogs WHERE content_sha256 = ''"

# Run le check trait-level (lance efe-server contre le PG restauré + invoque le ListCatalogs en boucle)
```

### 14.3 PITR with WAL archiving

Configuration PG (responsabilité ops) :

```ini
# postgresql.conf
archive_mode = on
archive_command = 'pgbackrest --stanza=efe-catalog archive-push %p'
wal_level = replica
```

EFE n'a aucun rôle direct dans le WAL archiving — c'est une
responsabilité PG.

---

## 15. Sécurité — TLS, secrets, RLS, SQL injection

### 15.1 TLS to Postgres

sqlx supporte 4 modes SSL :

| Mode | Description | Recommandé |
|------|-------------|:----------:|
| `disable` | Pas de TLS | Tier 1 dev only |
| `prefer` | Use TLS if available | Default |
| `require` | TLS required | Tier 2 staging |
| `verify-ca` | TLS + CA verify | Tier 3 prod |
| `verify-full` | TLS + CA + hostname verify | Tier 3 prod |

**Default D14** : `prefer`. Override `EFE_PG_SSL_MODE`.

### 15.2 Secret management

3 chemins :

1. **`EFE_PG_PASSWORD` env var** — simple, mais visible dans
   `kubectl describe pod`. Tier 1 only.
2. **`existingSecretName`** — Kubernetes Secret référencée par
   ConfigMap. Tier 2.
3. **External-secrets operator** — secret venant de Vault, AWS
   Secrets Manager, GCP Secret Manager. Tier 3.

Le Helm chart support les 3 paths avec une priorité claire (cf. § 16).

### 15.3 SQL injection defense

**sqlx `query!()` macro** = parametrized queries
unconditionally. Même si on appelle `query!("SELECT ... WHERE name = {}", input)`,
le compile fail (la macro exige `$N` placeholders).

**Test anti-régression** :

```rust
#[tokio::test]
async fn sql_injection_via_tenant_field_does_not_pass() {
    let pg = TestPg::start().await;
    let r = PostgresCatalogRegistry::connect(&pg.config, default_policy(), default_audit()).await.unwrap();
    let evil = "acme' OR '1'='1";
    // The registry should treat this as a literal string ; the row has tenant = exact `evil`.
    let _ = r.register(evil, "x", sample_content(), sample_metadata("alice")).await.unwrap();
    // List_catalogs("acme") should NOT include the row inserted under `acme' OR '1'='1`.
    let page = r.list_catalogs("acme", 100, None).await.unwrap();
    assert!(page.catalogs.iter().all(|c| c.tenant == "acme"),
        "SQL injection via tenant field leaked rows : {:?}", page.catalogs);
}
```

### 15.4 Optional: RLS

Cf. § 13.3.

---

## 16. Helm wiring — postgres connection block

### 16.1 `values.yaml` additions

```yaml
config:
  # ... existing fields ...
  catalog:
    # backend: "memory"  ← changement à "postgres" pour D14 prod
    backend: "memory"
    maxNamesPerTenant: 100
    maxVersionsPerName: 1000
    maxClassesPerCatalog: 256
    maxSentencesPerClass: 64
    # =========================================================
    # Sprint D14 — Postgres backend configuration.
    # Active uniquement quand `backend: "postgres"`.
    # =========================================================
    postgres:
      # Connection — host:port:database.
      host: "efe-pg.efe-catalog.svc.cluster.local"
      port: 5432
      database: "efe_catalog_db"
      user: "efe_catalog"
      sslMode: "prefer"  # disable | prefer | require | verify-ca | verify-full
      # Password : 3 paths in order of preference.
      # 1. existingSecretName — recommended.
      # 2. externalSecretName (external-secrets operator) — production.
      # 3. password — inline (tier 1 only, visible in kubectl describe).
      existingSecretName: "efe-pg-password"
      existingSecretKey: "password"
      externalSecretName: ""
      password: ""  # never set in production values.yaml
      # Pool sizing.
      poolMaxConnections: 8
      poolMinConnections: 2
      poolAcquireTimeoutMs: 5000
      poolMaxLifetimeS: 1800
      poolIdleTimeoutS: 600
      poolTestBeforeAcquire: true
```

### 16.2 ConfigMap rendering

```yaml
# templates/configmap.yaml — D14 additions
{{- if eq .Values.config.catalog.backend "postgres" }}
EFE_PG_HOST: {{ .Values.config.catalog.postgres.host | quote }}
EFE_PG_PORT: {{ .Values.config.catalog.postgres.port | quote }}
EFE_PG_DATABASE: {{ .Values.config.catalog.postgres.database | quote }}
EFE_PG_USER: {{ .Values.config.catalog.postgres.user | quote }}
EFE_PG_SSL_MODE: {{ .Values.config.catalog.postgres.sslMode | quote }}
EFE_PG_POOL_MAX_CONNECTIONS: {{ .Values.config.catalog.postgres.poolMaxConnections | quote }}
EFE_PG_POOL_MIN_CONNECTIONS: {{ .Values.config.catalog.postgres.poolMinConnections | quote }}
EFE_PG_POOL_ACQUIRE_TIMEOUT_MS: {{ .Values.config.catalog.postgres.poolAcquireTimeoutMs | quote }}
EFE_PG_POOL_MAX_LIFETIME_S: {{ .Values.config.catalog.postgres.poolMaxLifetimeS | quote }}
EFE_PG_POOL_IDLE_TIMEOUT_S: {{ .Values.config.catalog.postgres.poolIdleTimeoutS | quote }}
EFE_PG_POOL_TEST_BEFORE_ACQUIRE: {{ .Values.config.catalog.postgres.poolTestBeforeAcquire | quote }}
{{- end }}
```

### 16.3 Secret block (credentials)

```yaml
# templates/deployment.yaml — D14 additions for Postgres password
{{- if and (eq .Values.config.catalog.backend "postgres") .Values.config.catalog.postgres.existingSecretName }}
        - name: EFE_PG_PASSWORD
          valueFrom:
            secretKeyRef:
              name: {{ .Values.config.catalog.postgres.existingSecretName | quote }}
              key:  {{ .Values.config.catalog.postgres.existingSecretKey | default "password" | quote }}
{{- end }}
```

Pour external-secrets, le user fournit son propre `ExternalSecret`
(out-of-chart).

### 16.4 Optional: postgres operator

D14 ne ship pas d'operator — c'est une responsabilité ops. Le doc
fait une recommandation :

| Operator | Pros | Cons |
|----------|------|------|
| **CrunchyData PGO** | Mature, full HA, backups, monitoring | Heavy (lots of CRDs) |
| **Zalando postgres-operator** | Lightweight, well-tested | Less feature-rich |
| **CloudNativePG** | Modern, kubernetes-native | Newer (v1.x) |

→ Recommandation : CloudNativePG pour Tier 3.

---

## 17. Runtime wire-up — `Server::build` branche Postgres

### 17.1 Branch selection

```rust
// runtime.rs — D14 additions

let catalog_policy = CatalogPolicy::from(&config.catalog.policy);
let catalog_audit  = Arc::new(CatalogAuditEmitter::new());
let catalog_registry: Arc<dyn CatalogRegistry> = match self.catalog_registry {
    Some(r) => r,
    None => match config.catalog.backend {
        CatalogBackend::Memory => Arc::new(
            InMemoryRegistry::with_policy_and_audit(
                catalog_policy.clone(),
                CatalogAuditEmitter::new(),
            ),
        ),
        CatalogBackend::Postgres => {
            // === D14 NEW : real connection ===
            let registry = PostgresCatalogRegistry::connect(
                &config.catalog.postgres,
                catalog_policy.clone(),
                CatalogAuditEmitter::new(),
            )
            .await
            .map_err(|e| anyhow::anyhow!("Postgres backend init failed : {e}"))?;
            // Run migrations at boot (fail-fast).
            registry.run_migrations().await
                .map_err(|e| anyhow::anyhow!("migration failed at boot: {e}"))?;
            // Spawn pool metrics ticker (background task ; aborted on shutdown).
            tokio::spawn(pool_metrics_ticker(registry.pool().clone()));
            Arc::new(registry)
        }
    },
};
```

### 17.2 PgPool construction

`PostgresCatalogRegistry::connect` enrobe `sqlx::PgPool::connect`
avec :

* DSN built from `EFE_PG_*` env vars.
* TLS layer per `EFE_PG_SSL_MODE`.
* Pool config per `EFE_PG_POOL_*`.

### 17.3 Migration application at boot

`registry.run_migrations()` :

```rust
pub async fn run_migrations(&self) -> Result<(), CatalogError> {
    let m = sqlx::migrate!("../migrations");
    m.run(&self.pool).await.map_err(|e| CatalogError::Internal(format!("migration: {e}")))
}
```

Si migration fail → fail-fast. Pas de "best effort".

### 17.4 Health probe extension

Le tonic-health pour `CatalogService` doit refléter l'état de la
connexion PG :

```rust
async fn pg_health_check(pool: &PgPool, reporter: &HealthReporter) {
    let mut interval = tokio::time::interval(Duration::from_secs(15));
    loop {
        interval.tick().await;
        let healthy = sqlx::query("SELECT 1").fetch_one(pool).await.is_ok();
        if healthy {
            reporter.set_serving::<CatalogServiceServer<CatalogServiceImpl>>().await;
        } else {
            reporter.set_not_serving::<CatalogServiceServer<CatalogServiceImpl>>().await;
        }
    }
}
```

Si PG est down 30 s, le LB cesse de router vers `efe-server`.
**Seulement le service catalog est marqué NOT_SERVING** ; le swarm
service reste UP.

---

## 18. Threat model — 8 vecteurs

### 18.1 SQL injection

**Vecteur** : un client envoie `tenant = "'; DROP TABLE catalogs; --"`.
**Mitigation** : sqlx parametrized queries (cf. § 15.3). Test anti-régression.

### 18.2 Connection exhaustion (DoS)

**Vecteur** : un client envoie 10⁵ register concurrents → 10⁵
acquire pool, dépasse `pool_max_connections`.
**Mitigation** :
- Pool acquire timeout (5 s default) → rate-limit ressources.
- Rate-limit côté handler (D11 buckets per-tenant).
- Alert PagerDuty si `efe_pg_pool_active_connections` > 90 %.

### 18.3 Schema corruption (mid-migration crash)

**Vecteur** : `efe-server` crashe au milieu d'une migration.
**Mitigation** :
- sqlx::migrate utilise transaction par migration → atomicité.
- `_sqlx_migrations` table tracking évite double-application.
- Si un binaire repart, il reprend la migration.

### 18.4 Time-of-check / time-of-use

**Vecteur** : entre `check quota` et `INSERT`, deux clients
concurrents passent le check tous les deux puis insèrent.
**Mitigation** : advisory lock englobe **les deux étapes** (check +
INSERT) (cf. § 7.1 register pseudo-code).

### 18.5 Tenant data leak via SQL bug

**Vecteur** : un dev oublie le `WHERE tenant = $1` dans une nouvelle
query.
**Mitigation** : code review mandatory ; test anti-régression
(cf. § 13.2 cross-tenant tests). En D17+, RLS comme défense en
dernier ressort.

### 18.6 Credential exfiltration

**Vecteur** : `EFE_PG_PASSWORD` visible dans `kubectl describe` ou
core dump.
**Mitigation** :
- Recommandation `existingSecretName` ou external-secrets (cf. § 15.2).
- Rust `secrecy::Secret<String>` autour du password — `Drop` zero la
  mémoire.
- Pas de log du password (warning explicit dans le code si on
  logge la `CatalogPostgresConfig`).

### 18.7 Replication lag in HA setup

**Vecteur** : primary/standby PG, le client lit sur le standby qui
n'a pas vu la dernière INSERT.
**Mitigation** :
- D14 ne fait que **du read sur primary** (le pool pointe une seule
  URL).
- Read replicas pour `Get`/`List` est **out-of-scope D14** — D15 si
  bench le justifie.

### 18.8 Backup integrity

**Vecteur** : un backup corrompu, ou un restore qui échoue
silencieusement.
**Mitigation** :
- Recommandation pgBackRest (vérifie checksums).
- Restore drill mensuel sur staging — runbook.

---

## 19. Decisions log — récap des décisions clés

| # | Décision | Alternatives | Rationale |
|--:|----------|--------------|-----------|
| 1 | sqlx | diesel, sea-orm, tokio-postgres | Async-native + compile-time check + pool intégré (cf. § 3) |
| 2 | Single table v1 | LIST partition, table-par-tenant | Simplicité ops, scale suffisant pour cible v0.1.14 (cf. § 4) |
| 3 | Advisory locks per `(tenant, name)` | SERIALIZABLE, sequence, ON CONFLICT retry | Sémantique exacte, scale, no retry (cf. § 5) |
| 4 | JSONB pour content | TEXT serialized, séparées columns | Future queryability sans surcoût significatif (cf. § 6.1) |
| 5 | sqlx migrate forward-only | flyway, refinery, raw psql scripts | Built-in, robuste, pas de dep supplémentaire |
| 6 | Pool size default 8 | 4, 16, 32 | Tient sous PG max_connections=100 avec marge × 3 répliques |
| 7 | Migration au boot | au start CI uniquement, hors-binaire | Fail-fast cohérent, opérateur n'oublie jamais |
| 8 | Pas de FK sur parent_key | FK avec ON DELETE | App-level check pour `Conflict` mapping clean (cf. § 6.3) |
| 9 | Pas de RLS en v0.1.14 | RLS default-on | Complexity ops + perf overhead, WHERE explicit suffit (cf. § 13.3) |
| 10 | testcontainers pour intégration | docker-compose, mock | Hermétique, parallèle, no shared state |

---

## 20. Risques + mitigations

| ID | Sévérité | Risque | Mitigation |
|----|:--------:|--------|------------|
| **R-D14.1** | 🔴 HIGH | Migration boot-time fail → service catalog DOWN | Health probe par-service, alerting D13 ; runbook ops `pg_migrate_failed.md` |
| **R-D14.2** | 🔴 HIGH | Quota check race condition (ToCToU) | Advisory lock englobe check + INSERT (cf. § 18.4) |
| **R-D14.3** | 🟠 MED | Pool acquire timeout sous load | Pool sizing per tier, alert sur `efe_pg_acquire_timeout_total` |
| **R-D14.4** | 🟠 MED | PG primary down → tous les writes fail | Health probe pour drain LB ; à terme : standby fail-over (D15+) |
| **R-D14.5** | 🟠 MED | sqlx `query!()` cache drift CI | `cargo sqlx prepare --check` en CI (cf. § 9.2) |
| **R-D14.6** | 🟡 LOW | Latence `list_catalogs` haute | Materialized view (DT-D14.2) |
| **R-D14.7** | 🟡 LOW | Backup fail silencieux | Restore drill mensuel staging |
| **R-D14.8** | 🟡 LOW | Cross-tenant SELECT bug | Test anti-régression `cross_tenant_*` ; RLS futur |
| **R-D14.9** | 🟡 LOW | Advisory lock collision (2⁻⁶⁴) | Documenté ; `EFE` namespace seed |
| **R-D14.10** | 🟢 INFO | sqlx breaking change v0.8 | Pin v0.7 strict ; upgrade path en sprint dédié |

---

## 21. Plan wallclock — 7 j

| Jour | Activités | Output |
|-----:|-----------|--------|
| **J1** | DSN parsing + `CatalogPostgresConfig` + 11 env vars + Helm rendering + Python pin tests | `config.rs` PG block + `values.yaml` + `test_d14_postgres_helm_render.py` |
| **J2** | Migration scaffolding (`0001_init.sql`) + `sqlx prepare` + advisory lock helper | `migrations/0001_init.sql` + `lock_id_for()` + tests unitaires |
| **J3** | `PostgresCatalogRegistry::connect` + `register` + `evolve` + tests testcontainers happy paths | `postgres.rs` (~600 lignes) + 6 tests verts |
| **J4** | `get` + `list_catalogs` + `list_versions` + `delete` + tests testcontainers happy paths | `postgres.rs` complet + 12 tests verts |
| **J5** | Concurrency tests + tenant isolation + quotas + error mapping | tests d14 ~30 verts |
| **J6** | Runtime wire-up + observabilité (4 instruments) + health probe + perf bench | `runtime.rs` PG branch + bench |
| **J7** | Doc finalisation + `bench_baselines/v0.1.14/baseline.json` + audit interne D14 (skeleton) + CHANGELOG + sign-off | release v0.1.14 RC |

**Buffer** : 1 j supplémentaire pour la résolution des 5-10 bugs
attendus de l'audit interne (pattern D8 / D12-bis : trouver et fixer
~5-8 bugs).

---

## 22. SLO ajoutés en D14

| ID | Description | Cible | Type |
|----|-------------|------:|------|
| **D14-PG-1** | `RegisterCatalog` p99 latency (loopback PG) | ≤ 20 ms | GATE |
| **D14-PG-2** | `EvolveCatalog` p99 latency under 8-concurrent contention | ≤ 40 ms | GATE |
| **D14-PG-3** | `GetCatalog` p50 latency | ≤ 3 ms | GATE |
| **D14-PG-4** | Migration apply on empty PG ≤ 5 s | OBSERVATIONAL |
| **D14-PG-5** | Pool active connections ≤ 90 % of max sustained 5 min | ALERT |
| **D14-PG-6** | Concurrent register same key produces strictly monotone versions | INVARIANT |
| **D14-PG-7** | Cross-tenant SELECT/UPDATE/DELETE returns 0 rows | INVARIANT |

---

## 23. Anti-régression D14

```bash
# Migration applies on empty PG
cargo test -p efe-server --test d14_migrations

# Trait contract
cargo test -p efe-server --test d14_postgres_catalog_trait

# RPC end-to-end
cargo test -p efe-server --test d14_postgres_catalog_rpc

# Concurrency invariants
cargo test -p efe-server --test d14_postgres_catalog_trait \
    advisory_lock_serialises_concurrent_register_same_key

# Helm render
PYTHONPATH=clients/python python3 -m pytest scripts/tests/test_d14_postgres_helm_render.py

# Runtime wire-up
PYTHONPATH=clients/python python3 -m pytest scripts/tests/test_d14_runtime_wireup.py

# Cross-sprint regression (preserves D11/D12/D12-bis/D13 invariants)
PYTHONPATH=clients/python python3 -m pytest scripts/tests/ -q
```

---

## 24. Hand-off vers D15

D14 livre **persistence durable + multi-réplica safety**. D15 pourra
construire :

* **Compaction** : drop des versions ≥ 30 j non référencées avec
  retention configurable.
* **Audit events table** dédiée (mirror D13 audit stream) — durable
  même si Loki est down.
* **Read replicas** pour `Get`/`List` — séparation read/write traffic.
* **Materialized view** pour `list_catalogs` si bench D14 montre
  besoin (cf. R-D14.6).

→ Tracking dans `docs/bench/d15_catalog_lifecycle_architecture.md`
post-D14.

---

## 25. Annexes

### Annexe A — SQL DDL complet

Cf. § 6.1 + 6.4. Le DDL final post-merge sera dans
`crates/efe-server/migrations/0001_init.sql`.

### Annexe B — sqlx migrate folder structure

```
crates/efe-server/migrations/
├── 0001_init.sql
└── README.md
```

`README.md` contient le playbook ops (forward-only, no down.sql).

### Annexe C — testcontainers integration test template

Cf. § 10.3 pour le squelette `TestPg::start()`.

### Annexe D — Helm values.yaml diff

Cf. § 16.1.

### Annexe E — `EFE_PG_*` env vars matrix

| Env var | Default | Validate | Helm key |
|---------|--------:|----------|----------|
| `EFE_PG_HOST` | (required) | non-empty | `config.catalog.postgres.host` |
| `EFE_PG_PORT` | 5432 | 1..=65535 | `config.catalog.postgres.port` |
| `EFE_PG_DATABASE` | (required) | non-empty | `config.catalog.postgres.database` |
| `EFE_PG_USER` | (required) | non-empty | `config.catalog.postgres.user` |
| `EFE_PG_PASSWORD` | (required, via Secret) | non-empty | (via existingSecretName) |
| `EFE_PG_SSL_MODE` | `prefer` | enum (5 modes) | `config.catalog.postgres.sslMode` |
| `EFE_PG_POOL_MAX_CONNECTIONS` | 8 | ≥ 1 | `…poolMaxConnections` |
| `EFE_PG_POOL_MIN_CONNECTIONS` | 2 | ≥ 0, ≤ max | `…poolMinConnections` |
| `EFE_PG_POOL_ACQUIRE_TIMEOUT_MS` | 5000 | ≥ 100 | `…poolAcquireTimeoutMs` |
| `EFE_PG_POOL_MAX_LIFETIME_S` | 1800 | ≥ 60 | `…poolMaxLifetimeS` |
| `EFE_PG_POOL_IDLE_TIMEOUT_S` | 600 | ≥ 60 | `…poolIdleTimeoutS` |
| `EFE_PG_POOL_TEST_BEFORE_ACQUIRE` | true | bool | `…poolTestBeforeAcquire` |

### Annexe F — error mapping table

| sqlx error variant | PG code | `CatalogError` mapped |
|-------------------|---------|----------------------|
| `Database(de)` with `code = "23505"` | unique_violation | `Conflict("(tenant, name, version) already exists")` |
| `Database(de)` with `code = "23514"` | check_violation | `InvalidArgument("schema constraint violated")` |
| `Database(de)` with `code = "23502"` | not_null_violation | `Internal("missing required field — schema bug")` |
| `Database(de)` with `code = "40001"` | serialization_failure | `Internal("transaction conflict ; retry recommended")` |
| `Database(de)` with `code = "40P01"` | deadlock_detected | `Internal("deadlock ; retry recommended")` |
| `Database(de)` with `code = "53300"` | too_many_connections | `Internal("PG connection pool exhausted")` |
| `RowNotFound` | n/a | `NotFound(_)` |
| `PoolTimedOut` | n/a | `Internal("pool acquire timeout — DB overload")` |
| `Io(_)` | n/a | `Internal("transient network error")` |
| any other | n/a | `Internal(_)` (sanitized for client) |

### Annexe G — Bibliographie

- **sqlx** : <https://github.com/launchbadge/sqlx>
- **PostgreSQL Advisory Locks** : <https://www.postgresql.org/docs/current/explicit-locking.html#ADVISORY-LOCKS>
- **PostgreSQL Concurrency Control** : <https://www.postgresql.org/docs/current/transaction-iso.html>
- **xxhash-rust** : <https://crates.io/crates/xxhash-rust>
- **testcontainers** : <https://crates.io/crates/testcontainers>

---

> **Architecture finalisée** — 2026-04-27.
> **Prochaine étape** : J1 du plan wallclock (DSN parsing + Helm).
> **Audit interne D14** : à programmer en J7+1 selon pattern D8/D12-bis.
