# Sprint D12-bis — Audit interne

> **Date** : 2026-04-27 (post-livraison D12-bis)
> **Méthodologie** : revue ligne-à-ligne des 4 405 lignes livrées par
> D12-bis (handler 1 251 + conversions 757 + runtime 335 + scopes 209
> + config-delta + intégration 885 + Helm + Python pins). Comparaison
> code vs `docs/bench/d12bis_catalog_grpc_handler_architecture.md`.
> Adversarial reading : auth bypass, tenant leak, error leak, edge
> cases, mismatch architecture/code, wire-up incorrect, helm drift,
> coverage gaps.
> **Pattern référence** : audit D8 (8 bugs, 2 critiques) — l'audit
> D12-bis cherchait spécifiquement les "ça paraissait gardé mais ne
> l'était pas" (cf. bug D8 #1).

---

## 1. Bilan

| # | Sévérité | Domaine | Symptôme | Statut |
|--:|:--------:|---------|----------|:------:|
| **C1** | 🔴 CRIT | Quotas / wire-up | `runtime.rs::build()` ignore totalement `config.catalog.policy` ; `InMemoryRegistry::new()` hardcode `CatalogPolicy::default()`. Helm/env quotas muets. | ✅ Fixé |
| **C2** | 🔴 CRIT | Helm / pin | `values.yaml` = 64/32/64/32 ; pin test Python "match in-binary defaults" hardcodé à 64/32/64/32 ; vrais defaults = **100/1000/256/64**. Boucle d'auto-cohérence menteuse (le test passait pour la mauvaise raison). | ✅ Fixé |
| **H1** | 🟠 HIGH | Tests | Aucun des 51 tests d'intégration ne couvrait l'enforcement des 4 quotas. C1 sans gate CI. | ✅ Fixé |
| **M1** | 🟡 MED | Validation proto | `proto_content_to_trait` défaut `thresholds=None → (0.0, 0.0)` qui FAIL `validate_thresholds`. Erreur opaque "confidence_medium must be in (0,1) (got 0)" pour le client qui n'a rien envoyé de tel. | ✅ Fixé |
| **M2** | 🟡 MED | Validation config | `ServerConfig::validate()` ne valide pas `catalog.policy.*`. `EFE_CATALOG_MAX_NAMES_PER_TENANT=0` accepté au boot, échec opaque au premier register. | ✅ Fixé |
| **L1** | 🟢 LOW | Dette code | `CatalogServiceImpl.policy: Arc<CatalogPolicy>` marqué `#[allow(dead_code)]`. Architecture spec § 4.2 le mentionnait, implémentation jamais venue. | ✅ Fixé (drop) |
| **L2** | 🟢 LOW | Observabilité | Pas d'instrument `catalog_delete_duration_seconds` côté D13. Décision D13, hors-périmètre D12-bis ; couvert par les `efe_grpc_*_handled` génériques. | ⏸ Reporté D13-amendment |

**Méthodologie audit** : revue ligne-à-ligne de tous les fichiers
livrés par D12-bis + cross-check architecture/code + recherche
explicite des patterns de bug D8 (gates qui paraissent armés mais ne
le sont pas, drifts entre 3 sources de vérité, defaults oubliés).
Le bug **C1** est exactement le cas que l'audit visait à attraper :
un quota qui *paraissait* configurable depuis Helm mais qui ne
l'était pas — l'opérateur pouvait baisser `maxNamesPerTenant` à 10
sans aucun effet runtime.

---

## 2. Détail des fixes

### 2.1 Fix C1 — propagation `config.catalog.policy` au registry

**Fichiers modifiés** :

- `crates/efe-server/src/config.rs` :
  - Ajout `impl CatalogPolicyConfig::to_policy() -> CatalogPolicy`
  - Ajout `impl From<&CatalogPolicyConfig> for CatalogPolicy`
- `crates/efe-server/src/runtime.rs::ServerBuilder::build` :
  - Construction `CatalogPolicy::from(&config.catalog.policy)`
  - Substitution `InMemoryRegistry::new()` → `InMemoryRegistry::with_policy_and_audit(policy.clone(), AuditEmitter::new())` (la 2e branche `with_policy_and_audit` existait déjà depuis D12, juste jamais utilisée par le runtime).

**Vérification** : 4 nouveaux tests dans `tests/d12bis_catalog_integration.rs`
section G — chacun configure un quota tight (e.g. `max_names_per_tenant=2`),
drive le trafic à la frontière, et assert `ResourceExhausted` / `InvalidArgument`
au franchissement.

### 2.2 Fix C2 — alignement `values.yaml` ↔ `policy.rs::DEFAULT_*`

**Fichiers modifiés** :

- `deploy/helm/efe/values.yaml` : 64/32/64/32 → 100/1000/256/64 avec commentaires explicites mirror-ing les constantes `policy::DEFAULT_*`.
- `scripts/tests/test_d12bis_catalog_helm_render.py::test_values_yaml_quotas_match_efe_server_defaults` :
  - Ajout helper `_read_policy_defaults_from_rust()` qui parse les 4 `pub const DEFAULT_*` directement depuis `crates/efe-server/src/catalog/policy.rs` (regex). Plus de hardcoded values.
  - Le test casse maintenant si quiconque touche à values.yaml OU à policy.rs sans mettre l'autre en cohérence.

**Note** : ce fix fonctionne en synergie avec C1 — sans C1, les valeurs Helm n'ont aucun effet runtime, et le test pinçait juste un drift de documentation. Après C1+C2, les valeurs Helm sont :
1. Lisibles (commentaires "= policy::DEFAULT_*")
2. Cohérentes avec le binaire (`policy.rs`)
3. Effectives à runtime (injectées au registry via C1)
4. Anti-drift (test Python C2)

### 2.3 Fix H1 — tests d'enforcement quotas

**Fichier modifié** : `crates/efe-server/tests/d12bis_catalog_integration.rs`

- `TestServer::start_no_auth_with_cfg<F>` ajouté pour permettre la mutation arbitraire de `ServerConfig` avant build (factorisation de `start_no_auth`).
- 4 tests Section G (un par quota) + 2 tests Section H (M1).

| Test | Configuration | Vérifie |
|------|---------------|---------|
| `quota_max_names_per_tenant_is_enforced_via_config` | `max_names_per_tenant=2` | 3e register → `ResourceExhausted` |
| `quota_max_versions_per_name_is_enforced_via_config` | `max_versions_per_name=2` | 3e evolve → `ResourceExhausted` |
| `quota_max_classes_per_catalog_is_enforced_via_config` | `max_classes_per_catalog=2` | register avec 3 classes → `InvalidArgument` |
| `quota_max_sentences_per_class_is_enforced_via_config` | `max_sentences_per_class=1` | register avec 2 sentences → `InvalidArgument` |
| `register_rejects_missing_thresholds_with_clear_error` | (default) | `thresholds=None` → `InvalidArgument` mentionnant "thresholds" |
| `register_rejects_missing_cone_policy_with_clear_error` | (default) | `cone_policy=None` → `InvalidArgument` mentionnant "cone_policy" |

### 2.4 Fix M1 — validation explicite `cone_policy` + `thresholds`

**Fichier modifié** : `crates/efe-server/src/services/catalog/conversions.rs::proto_content_to_trait`

- `cone_policy: None` rejeté avec `Status::invalid_argument("CatalogContent.cone_policy is required …")` (au lieu d'un default 0.7 silencieux).
- `cone_policy.max_projection_distance` validé `(0, 1]` immédiatement (fail-fast au handler avant la registry).
- `thresholds: None` rejeté avec `Status::invalid_argument("CatalogContent.thresholds is required …")` (au lieu d'un default `(0.0, 0.0)` qui faisait FAIL la `validate_thresholds` du registry avec un message confondant).

**Tests ajoutés** : 4 dans `conversions.rs::tests` (`proto_content_rejects_missing_cone_policy`, `proto_content_rejects_invalid_cone_policy_distance`, `proto_content_rejects_cone_policy_distance_over_one`, `proto_content_rejects_missing_thresholds`).

### 2.5 Fix M2 — validation `config.catalog.policy` au boot

**Fichier modifié** : `crates/efe-server/src/config.rs::validate`

- `max_names_per_tenant >= 1`
- `max_versions_per_name >= 1`
- `max_classes_per_catalog >= MIN_CLASSES_PER_CATALOG` (= 2, cohérent avec la règle "≥ 2 classes" du registry)
- `max_sentences_per_class >= 1`

Tout `0` ou valeur sub-min retourne `ConfigError::Invalid` au boot avec un message expliquant pourquoi le quota n'a pas de sens.

### 2.6 Fix L1 — drop du field `Arc<CatalogPolicy>` mort

**Fichiers modifiés** :

- `crates/efe-server/src/services/catalog/mod.rs::CatalogServiceImpl` : drop du field `policy`, mise à jour de `new()`, doc explicite que la policy vit côté registry.
- `crates/efe-server/src/runtime.rs` : drop du field `catalog_policy: Arc<CatalogPolicy>` du struct `Server` ; suppression du paramètre du `CatalogServiceImpl::new()`.
- Tests `build_handler` mis à jour.

L'architecture spec § 4.2 mentionnait `policy: Arc<CatalogPolicy>` sur le handler, mais aucun callsite ne s'en servait. Le keep-as-future-extension n'avait jamais été instancié — le drop est la simplification correcte. Si D14 a besoin d'une policy locale au handler (pour éviter un round-trip réseau au registry distant), elle pourra être réinjectée à ce moment-là, c'est un changement Arc-passable trivial.

---

## 3. Anti-régression

### 3.1 Vérifications post-fix

```bash
# Lib + intégration Rust
cargo test -p efe-server --lib services::catalog       # 55 tests OK (was 51)
cargo test -p efe-server --test d12bis_catalog_integration  # 28 tests OK (was 22)

# Suites adjacentes (pour s'assurer que le drop policy n'a rien cassé)
cargo test -p efe-server --lib                          # 248 tests OK (was 244)
cargo test -p efe-server --test integration_grpc        # 20 tests OK (no regression)
cargo test -p efe-server --test shutdown_behavior       # 6 tests OK
cargo test -p efe-server --test startup_sequence        # 8 tests OK
cargo test -p efe-telemetry --lib                       # 117 tests OK

# Python — surface pin + cross-sprint
PYTHONPATH=clients/python python3 -m pytest scripts/tests/ -q
# 517 passed, 2 skipped, 0 failed
```

**Total** : 310 Rust + 517 Python = **827 tests verts**.

### 3.2 Tests ajoutés en D12-bis-audit

| Fichier | Avant | Après | Δ |
|---------|------:|------:|---:|
| `services/catalog/conversions.rs::tests` | 22 | 26 | +4 (M1) |
| `services/catalog/mod.rs::tests` | 29 | 29 | 0 |
| `tests/d12bis_catalog_integration.rs` | 22 | 28 | +6 (4 H1 + 2 H1-bis pour M1 wire-level) |
| `scripts/tests/test_d12bis_catalog_helm_render.py` | 9 | 9 | 0 (refactor — `_read_policy_defaults_from_rust` helper, pas un nouveau test) |
| **Total D12-bis** | **82** | **92** | **+10** |

---

## 4. Risques résiduels

- **R-D12bis-1 (LOW)** : la politique `MIN_CLASSES_PER_CATALOG = 2` est en `pub const` dans `policy.rs` ; si quelqu'un la baisse à 1, le test M2 sur `max_classes_per_catalog` continue de fonctionner mais perd sa cohérence schemax-≥-2. Mitigation : pin test pourrait check `MIN_CLASSES_PER_CATALOG == 2`. Non bloquant — la registry refuse toujours `< 2` classes au runtime via `validate_content`.

- **R-D12bis-2 (LOW)** : `CatalogPolicyConfig` et `CatalogPolicy` ont 4 champs identiques ; ils peuvent diverger lors d'un ajout de quota. Mitigation : `to_policy()` est un mapping field-par-field explicite, donc l'ajout d'un champ casse le compile (le destructuring est exhaustif via `Self {…}`). En pratique le compilateur attrape la divergence au prochain `cargo build`.

- **R-D12bis-3 (LOW)** : pas d'instrument `catalog_delete_duration_seconds`. Couvert par `efe_grpc_*_handled{method="/efe.v1.catalog.CatalogService/DeleteCatalog"}`. À considérer pour un D13-amendment si l'opérateur veut une histogram delete-spécifique.

---

## 5. Sign-off audit D12-bis

| Vérification | Résultat |
|--------------|:--------:|
| Architecture vs livraison cohérente | ✅ post-fix |
| 6 RPC RBAC mappings (scopes.rs) | ✅ |
| Auth + tenant isolation D11 | ✅ |
| Error mapping sans leak | ✅ (test `catalog_internal_error_does_not_leak_inner_message`) |
| Quotas configurables Helm → runtime | ✅ post-fix C1 |
| Validation early-fail au boot | ✅ post-fix M2 |
| Validation early-fail au handler | ✅ post-fix M1 |
| Tests d'enforcement quotas | ✅ post-fix H1 |
| Helm/binary defaults cohérents | ✅ post-fix C2 |
| Pas de field mort | ✅ post-fix L1 |
| 22→28 tests intégration tonic loopback | ✅ |
| `cargo test --workspace` impacté | ✅ (signature change `CatalogServiceImpl::new` ; runtime + tests à jour) |
| Python pin tests up-to-date | ✅ post-fix C2 |

**Audit D12-bis fermé** — 7 bugs trouvés, 6 fixés, 1 reporté (L2, hors-périmètre).
