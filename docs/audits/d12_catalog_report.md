# Sprint D12 — Catalog admin endpoints + versioning report (skeleton)

> **Statut** : skeleton à compléter dès l'arrivée du gRPC handler
> en D12-bis + 3 weeks de runs verts post-merge sur le workflow
> `bench-catalog.yml`. Suit `d11_isolation_report.md` dans la
> série des rapports d'audit chiffrés.

---

## 1. Contexte

D12 livre l'API d'admin de catalogues versionnés (cf. spec
`docs/bench/d12_catalog_admin_versioning_architecture.md`). Cette
section se remplit quand :

1. La PR de freeze v0.1.12 est mergée.
2. Le gRPC handler `crates/efe-server/src/services/catalog.rs`
   atterrit en D12-bis et est wiré dans `runtime::Server`.
3. La dépendance `sha2 = "0.10"` est ajoutée à
   `crates/efe-server/Cargo.toml` (additive direct dep) et le
   placeholder `compute_digest` dans `catalog/memory.rs` est
   remplacé par un vrai SHA-256 sur le canonical-JSON encoding
   de `CatalogContent`.
4. Les tests d'intégration `crates/efe-server/tests/d12_catalog_*.rs`
   sont implémentés (~30 tests : concurrent register/evolve,
   sha256 witness round-trip, RBAC scope checks, tenant filtering).
5. La cible `ghz` lance l'eval des 5 latency budgets D12-CT-1..5
   sous `.github/workflows/bench-catalog.yml`.
6. 3 main runs verts du workflow.

> **À la première mesure live** : remplacer ce skeleton par les
> chiffres réels et les commentaires d'analyse (cf.
> `d11_isolation_report.md` ou `d10_dx_report.md` pour le format).

---

## 2. Mesure live D12.1 — Proto contract

### 2.1 Inventaire RPC

> **À mesurer** : 6 RPCs (Register / Evolve / Get / List /
> ListVersions / Delete) — vérifié par
> `scripts/tests/test_d12_proto_contract.py::test_catalog_service_has_6_rpcs`.

### 2.2 Inventaire messages

> **À mesurer** : 25 messages — vérifié par
> `test_message_inventory_complete`.

### 2.3 Tag-number stability

> **À mesurer** : `RegisterCatalogRequest` / `CanonicalCatalog` /
> `CatalogMetadata` tag numbers pinnés — vérifié par les 3 tests
> `test_*_tag_numbers_stable`.

---

## 3. Mesure live D12.2 — Rust scaffolding

### 3.1 Cargo test (in-memory impl)

> **À mesurer** : `cargo test --package efe-server --lib catalog::`
> doit produire `27 passed; 0 failed`.
>
> Couverture du trait `CatalogRegistry` :
>
> * `register` : v1 → v2, schema rejection, quota cap.
> * `evolve` : 5 ops (APPEND, REMOVE, UPDATE_THRESHOLDS,
>   REPLACE_CLASS_ANCHORS, RECOMPILE), parent_key chain.
> * `get` : include_compiled flag.
> * `list_catalogs` : tenant-scoped filter.
> * `list_versions` : ascending order.
> * `delete` : reference check, force=true cascade.

### 3.2 Cargo test (full lib regression)

> **À mesurer** : `cargo test --package efe-server --lib` doit
> produire `183 passed; 0 failed` (156 existing + 27 new). Confirmé
> en local le jour du landing.

---

## 4. Mesure live D12.3 — SDK Python

### 4.1 Tests anti-régression

> **À mesurer** : `pytest scripts/tests/test_d12_*.py` doit
> produire `37 passed; 0 failed` :
>
> * `test_d12_disjoint_keys.py` : 9 tests (RESERVED_PREFIXES, SLOs,
>   namespacing, comparators).
> * `test_d12_surface_extensions.py` : 16 tests (4 nouveaux types,
>   5 nouvelles méthodes, parser CLI).
> * `test_d12_proto_contract.py` : 12 tests (RPC inventory, message
>   inventory, tag numbers, helper round-trip).

### 4.2 Smoke import

> **À mesurer** : `from efe_tool import EvolveOp, RegisteredCatalog,
> CatalogAuditMetadata, CompiledClassRef` doit succeed sans
> ImportError, et chaque type doit être un dataclass frozen ou
> un Enum.

---

## 5. Mesure live D12.4 — CLI

### 5.1 Parser

> **À mesurer** : 6 verbes catalog (`register`, `evolve`, `list`,
> `versions`, `get`, `delete`) parsables sans warnings ; chaque
> commande emit un document JSON schema-versionné sur stdout.

### 5.2 Lifecycle E2E (post-D12-bis)

> **À mesurer** : `examples/05_catalog_admin.py` doit compléter en
> < 30 s contre un `efe-server` D12-bis local, avec sortie
> conforme aux 8 étapes du script.

---

## 6. Mesure live D12.5 — SLO compliance

### 6.1 Tableau de mesures

> **À mesurer** :
>
> | SLO ID         | Cible       | Mesuré      | Statut          |
> |----------------|-------------|-------------|-----------------|
> | `D12-CT-1`     | ≤ 200 ms    | TBD         | SKIPPED→PASS    |
> | `D12-CT-2`     | ≤ 1 500 ms  | TBD         | SKIPPED→PASS    |
> | `D12-CT-3`     | ≤ 250 ms    | TBD         | SKIPPED→PASS    |
> | `D12-CT-4`     | ≤ 50 ms     | TBD         | SKIPPED→PASS    |
> | `D12-CT-5`     | ≤ 75 ms     | TBD         | SKIPPED→PASS    |
> | `D12-CT-6`     | ≥ 100 %     | TBD         | SKIPPED→PASS    |
> | `D12-CT-7`     | ≥ 100 %     | TBD         | SKIPPED→PASS    |
>
> Source : `bench_baselines/v0.1.12/baseline.json` après le
> 3e run vert.

### 6.2 Régressions (D1-D11)

> **À mesurer** : aucune régression sur les 63 SLOs des sprints
> précédents. Vérifié par `cargo test --workspace` + le bench
> aggregator (qui charge la baseline antérieure et compare).

---

## 7. Risques résiduels post-mesure

> **À compléter** post-mesure :
>
> * R-D12-1 : si les latency SLOs (`D12-CT-1..5`) doivent être
>   relâchés, documenter l'écart vs la cible architecture et la
>   mitigation (ex : sharding par tenant, postgres backend, etc.).
> * R-D12-2 : suivre la dérive de cardinalité `catalog::*` après
>   D12-bis (chaque tenant peut accumuler 1 000 versions × 100
>   names = 100k chains au max ; à monitorer).
> * R-D12-3 : suivre la dérive du quota `MAX_VERSIONS_PER_NAME`
>   en prod ; relâcher de 1 000 à 10 000 si plusieurs tenants
>   atteignent le plafond.

---

## 8. Sign-off

> **À compléter** post-mesure :
>
> * **Tech lead** : ___ , ___-___-2026
> * **SRE** : ___ , ___-___-2026
> * **Security** : ___ , ___-___-2026
> * **Product** : ___ , ___-___-2026
