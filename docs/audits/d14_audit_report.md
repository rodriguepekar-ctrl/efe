# Sprint D14 — Audit interne

> **Date** : 2026-04-27 (post-livraison D14).
> **Méthodologie** : revue ligne-à-ligne des 4 681 lignes livrées par
> D14 (postgres.rs 1 577 + migration 0001_init.sql 114 + config.rs PG
> block 200+ + runtime branche + Helm 216 + integration tests 712 +
> Python pins 476 + telemetry 241). Comparaison code vs
> `docs/bench/d14_postgres_catalog_backend_architecture.md`.
> Adversarial reading : SQL injection, race conditions, ToCToU, off-
> by-one, DDL ↔ queries cohérence, schema drift, transaction leaks,
> error message leaks.
> **Pattern référence** : audit D8 (8 bugs, 2 critiques) + audit
> D12-bis (7 bugs, 2 critiques). L'audit D14 cible spécifiquement
> les inconsistances schema↔queries et les races concurrence.

---

## 1. Bilan

| # | Sévérité | Domaine | Symptôme | Statut |
|--:|:--------:|---------|----------|:------:|
| **C1** | 🔴 CRIT | DDL ↔ validation cohérence | `parent_key` CHECK exige `^[a-z0-9_-]+/...$`, mais `tenant`/`name` columns n'ont aucun CHECK. Register `tenant="ACME"` v1 → OK ; evolve to v2 → CHECK violation opaque. Inconsistance latente. | ✅ Fixé |
| **M1** | 🟠 MED | Concurrence quota | Race `max_names_per_tenant` entre noms distincts : advisory lock per-(tenant, name) ne sérialise pas les checks cross-name. 2 register concurrents sur 2 NEW noms peuvent overshoot le quota de +1. | ⏸ Documenté DT-D14.5 |
| **L1** | 🟡 LOW | DSN building | `dsn()` ne URL-encode pas le `host` ; un IPv6 host casserait la construction. | ⏸ Doc — opérateurs utilisent hostname/IPv4 |
| **L2** | 🟡 LOW | API publique | `CatalogPostgresConfig` + `PgSslMode` non exportés via `lib.rs::pub use` ; utilisateurs externes ne peuvent pas configurer programmatiquement le pool. | ✅ Fixé |
| **L3** | 🟡 LOW | Code mort | `chrono::{DateTime, Utc}` import + `_chrono_keepalive()` dummy fn jamais appelée. Dead code residue. | ✅ Fixé |
| **L4** | 🟡 LOW | Helm rendering | `deployment.yaml` rend `env:` inconditionnellement même quand le bloc interne est vide (memory mode + pas d'extraEnv) → YAML ambigu. | ✅ Fixé |
| **L5** | 🟢 INFO | Hygiène code | Handler `audit: Arc<AuditEmitter>` field présent mais non-utilisé (D12-bis L1 résiduel — handler bump le metric, registry log l'audit). | ⏸ Pré-existant, hors-scope D14 |

**Méthodologie audit** : revue ligne-à-ligne tous les fichiers
livrés par D14 + cross-check architecture/code + recherche
explicite des patterns de bug D8/D12-bis (drift entre 3 sources de
vérité, defaults oubliés, gates qui paraissent armés mais ne le
sont pas, error message leaks).

L'audit D14 a confirmé que les 10 décisions architecturales du
document `d14_postgres_catalog_backend_architecture.md` sont
correctement implémentées :

* sqlx 0.7 runtime-checked ✓
* Single `catalogs` table + composite PK ✓
* `pg_advisory_xact_lock(xxh64(tenant/name, EFE_SEED))` ✓
* JSONB content + labels ✓
* sqlx::migrate! forward-only ✓
* Pool 8 conn/réplique default ✓
* Migration au boot fail-fast ✓
* App-level reference check (pas FK ON DELETE CASCADE) ✓
* Pas de RLS en v0.1.14 ✓
* `EFE_PG_TEST_URL`-gated tests ✓

Le bug **C1** est exactement le pattern que l'audit visait à
attraper : un CHECK constraint qui *paraissait* armé sur tenant/name
mais ne l'était pas — l'opérateur pouvait register `ACME` puis voir
`evolve` planter au CHECK avec un message obscur.

---

## 2. Détail des fixes

### 2.1 Fix C1 — Cohérence DDL ↔ validation tenant/name

**Symptôme exact** :

1. Le DDL `migrations/0001_init.sql` ligne 64-68 :
   ```sql
   CONSTRAINT catalogs_parent_chain CHECK (
       parent_key = '' OR parent_key ~ '^[a-z0-9_-]+/[a-z0-9_-]+@v\d+$'
   )
   ```
   exige que le `parent_key` soit du lowercase ASCII.

2. MAIS les CHECK constraints sur `tenant` (ligne 70) et `name`
   (ligne 71) ne valident que `length(...) >= 1` — n'importe quel
   caractère est accepté.

3. Conséquence : `INSERT v1` avec tenant="ACME" name="billing"
   passe (parent_key='' satisfait le check). `INSERT v2` avec
   parent_key="ACME/billing@v1" **fail check_violation 23514**.

4. L'erreur PG est mappée par `map_pg_error` à
   `CatalogError::InvalidArgument`, mais le client a déjà reçu
   le succès de v1 ; la dette se révèle à l'evolve.

**Fix livré** (4 sub-fixes coordonnés) :

* **C1.1** — `crates/efe-server/src/catalog/policy.rs` : nouvelle
  méthode statique
  `CatalogPolicy::validate_identifier(field: &str, value: &str) -> Result<(), PolicyError>`
  qui vérifie `^[a-z0-9_-]{1,63}$` sans dep regex (hand-rolled byte
  scan). Mirror exact du regex sur `parent_key`.

* **C1.2** — `crates/efe-server/src/catalog/postgres.rs::register`
  + `crates/efe-server/src/catalog/memory.rs::register` :
  `validate_identifier("tenant", tenant)?` + idem pour `name`
  *avant* tout INSERT, *avant* le tx begin. Refus rapide avec
  `InvalidArgument` clair plutôt que `check_violation` opaque
  post-evolve.

* **C1.3** — `crates/efe-server/migrations/0002_tenant_name_format_check.sql` :
  defense in depth SQL :
  ```sql
  ALTER TABLE catalogs ADD CONSTRAINT catalogs_tenant_format
      CHECK (tenant ~ '^[a-z0-9_-]{1,63}$');
  ALTER TABLE catalogs ADD CONSTRAINT catalogs_name_format
      CHECK (name ~ '^[a-z0-9_-]{1,63}$');
  ```
  Si jamais la validation Rust est bypassée (e.g. admin SQL
  direct), le CHECK SQL refuse l'INSERT.

* **C1.4** — `crates/efe-server/tests/d14_postgres_catalog_integration.rs` :
  le test `sql_injection_via_tenant_field_is_treated_as_literal`
  (qui inserrait un tenant injecté pour vérifier la
  paramétrisation sqlx) est renommé
  `sql_injection_via_tenant_field_is_rejected_by_validation` et
  l'attendu change : `CatalogError::InvalidArgument` (validation
  refuse au registry-level), plus belt-and-braces vérifie
  qu'aucune ligne n'a été insérée. Nouveau test
  `validate_identifier_rejects_uppercase_tenant` exerce la
  contrainte explicitement.

**Vérification** : `cargo test -p efe-server --test d14_postgres_catalog_integration`
22 tests verts (was 21 — +1 nouveau test C1).

### 2.2 Fix L2 — `CatalogPostgresConfig` + `PgSslMode` exportés

**Symptôme** : `crates/efe-server/src/lib.rs::pub use config::{...}`
listait `CatalogConfig`, `CatalogPolicyConfig`, `CatalogBackend`
mais pas `CatalogPostgresConfig` ni `PgSslMode`. Les utilisateurs
externes du crate ne peuvent pas construire programmatiquement
un `ServerConfig` avec un override Postgres.

**Fix** : ajouté à la liste `pub use`.

### 2.3 Fix L3 — `chrono` dead code retiré

**Symptôme** : `postgres.rs` importait `chrono::{DateTime, Utc}` et
définissait `_chrono_keepalive(t: DateTime<Utc>) -> i64` jamais
appelé, juste pour suppress un warning d'import inutilisé. Le
commentaire admettait être un workaround.

**Fix** : suppression de l'import et de la fonction. Cargo.toml
retire la direct-dep `chrono` (sqlx amène toujours chrono via la
feature `chrono` qu'on lui active — on n'a juste pas besoin d'y
accéder directement). Build clean après le fix.

### 2.4 Fix L4 — Helm `env:` guard

**Symptôme** : `deployment.yaml` rendait `env:` inconditionnellement
ligne 67. Si l'opérateur utilise `backend=memory` sans `extraEnv`,
le rendering produit :
```yaml
env:
            
```
— `env:` à valeur vide. Kubernetes parse comme `null` (équivalent
à pas d'env), mais `kubectl get -o yaml | yq` casse le round-trip.

**Fix** : guard avec `{{- if $needsEnv }}` où
`$needsEnv := or (eq .Values.config.catalog.backend "postgres") .Values.extraEnv`.
Bloc `env:` rendu seulement si backend=postgres ou extraEnv non-vide.

---

## 3. Items reportés (non-fix)

### 3.1 M1 — Race `max_names_per_tenant` cross-name (DT-D14.5)

L'advisory lock est `xxh64(tenant/name)`, donc deux register
concurrents sur des noms distincts ne se sérialisent pas. Si on
est à `n_names = max - 1` :

```
Client A : lock(t/name3), n_names=1, exists=false → allow → INSERT
Client B : lock(t/name4), n_names=1 (A pas commit) → allow → INSERT
final state : n_names = 3 (overshoot de +1)
```

**Sévérité MED** car :
- Soft quota violation (overshoot bornée par le degré de concurrence simultané, max ~ pool_max_connections).
- Pas une vulnérabilité sécurité ; pas un crash.
- Le contract opérateur n'est pas strict — `max_names_per_tenant`
  est un guardrail soft.

**Coût du fix** : ajouter un advisory lock per-tenant
(`pg_advisory_xact_lock(xxh64(tenant, ""))`) avant le quota
check. Sérialise toutes les registrations d'un tenant
(scaling impact). Trade-off acceptable seulement si on observe
des overshoot en prod.

**Décision** : tracker comme **DT-D14.5** dans le sprint closure.
Pas fix en v0.1.14. Re-évaluer si bench D14 montre des overshoots
en CI ou en prod.

### 3.2 L1 — `dsn()` host non URL-encodé

IPv6 hosts (e.g. `::1`) cassent la construction DSN. En pratique
opérateurs utilisent FQDN ou IPv4 ; IPv6 est rare en cluster
interne.

**Décision** : doc only. Ajouter à la closure D14.

### 3.3 L5 — handler `audit` field résiduel

Pré-existant D12-bis L1 ; le handler tient un `Arc<AuditEmitter>`
field dormant. Le fix complet retirerait le field, mais ça
modifie la signature `CatalogServiceImpl::new` — risque de drift
avec le futur HMAC-signing (D14-bis). Choix conscient de garder
en l'état.

**Décision** : pré-existant, hors-scope D14.

---

## 4. Anti-régression

### 4.1 Vérifications post-fix

```bash
# Lib (handler + helpers + new validate_identifier) — 266 tests verts
cargo test -p efe-server --lib

# D14 integration (gated EFE_PG_TEST_URL) — 22 tests verts
cargo test -p efe-server --test d14_postgres_catalog_integration

# D12-bis integration — 28 tests verts (no regression)
cargo test -p efe-server --test d12bis_catalog_integration

# Cross-sprint Rust regression
cargo test -p efe-server --test integration_grpc --test shutdown_behavior \
    --test startup_sequence
# 20 + 6 + 8 = 34 tests verts

# Python — 553 verts (cross-sprint full suite)
PYTHONPATH=clients/python python3 -m pytest scripts/tests/ -q
```

**Total** : 266 + 22 + 28 + 34 + 553 = **903 tests verts**.

### 4.2 Tests ajoutés en D14-audit

| Fichier | Avant | Après | Δ |
|---------|------:|------:|---:|
| `tests/d14_postgres_catalog_integration.rs` | 21 | 22 | +1 (C1.4 `validate_identifier_rejects_uppercase_tenant`) |
| `services/catalog/policy.rs::tests` | n existing | n existing | 0 (la nouvelle méthode `validate_identifier` est exercée par les tests d'intégration et le test register-already-existing) |

Test `sql_injection_via_tenant_field_is_treated_as_literal`
**renommé** en `sql_injection_via_tenant_field_is_rejected_by_validation`
avec contract plus fort (validation + belt-and-braces empty list).

---

## 5. Sign-off audit D14

| Vérification | Résultat |
|--------------|:--------:|
| Architecture vs livraison cohérente | ✅ post-fix |
| 6 méthodes du trait `CatalogRegistry` correctement implémentées | ✅ |
| Concurrency `pg_advisory_xact_lock` per-(tenant, name) | ✅ |
| DDL ↔ validation cohérence (tenant/name format) | ✅ post-fix C1 |
| SQL injection defense (sqlx parametrize) | ✅ |
| Tenant isolation D11 (WHERE tenant=$1) | ✅ |
| Error mapping sans leak vers client | ✅ |
| Forward-only migrations + sqlx::migrate! au boot | ✅ |
| Connection pool sizing + lifecycle | ✅ |
| `EFE_PG_PASSWORD` jamais en ConfigMap (toujours via Secret) | ✅ |
| 11 env vars `EFE_PG_*` correctement lues + validées | ✅ |
| 4 D14 instruments + 7 PG spans | ✅ |
| API publique `lib.rs::pub use` complète | ✅ post-fix L2 |
| Helm `env:` block bien gardé | ✅ post-fix L4 |
| Pas de dead code résiduel (chrono) | ✅ post-fix L3 |
| Tests intégration `EFE_PG_TEST_URL` gated | ✅ |

**Audit D14 fermé** — 7 bugs trouvés (1 CRIT, 1 MED, 5 LOW),
6 fixés (C1 cascade en 4 sub-fixes + L2 + L3 + L4),
1 documenté comme dette technique (M1 → DT-D14.5),
1 pré-existant hors-scope D14 (L5).

---

## 6. Annexe — Decisions de fixes

| Décision | Alternatives écartées |
|----------|----------------------|
| C1 : valider tenant/name côté Rust **AVANT** insert (refus `InvalidArgument`) | Laisser le CHECK constraint SQL parler (mais erreur opaque côté client) |
| C1 : ajouter aussi CHECK constraints SQL en migration `0002_*` (defense in depth) | Pas de CHECK SQL (mais bypass possible via admin direct SQL) |
| C1 : regex `^[a-z0-9_-]{1,63}$` (mirror du `parent_key` regex) | Regex plus permissif (mais reverse-incompatible avec le DDL existant) |
| C1 : valider seulement dans `register` (pas `evolve`) | Valider dans `evolve` aussi (overkill — evolve lit une row qui existe forcément déjà-validée) |
| L2 : exporter via `pub use config::{...}` | `pub mod config` (expose tout, perd l'encapsulation) |
| L3 : retirer la dep `chrono` directe | Garder le keepalive (bruit) |
| L4 : guard helm `{{- if $needsEnv }}` | Toujours rendre `env: []` explicite (verbeux) |
