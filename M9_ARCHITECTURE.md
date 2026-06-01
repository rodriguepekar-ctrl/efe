# M9 — Alignment Management & Cone Governance
## Architecture complète, ultra-détaillée — v2.0

---

**Document** : spécification d'architecture (contrat d'implémentation)
**Module** : `efe_core::alignment` (remplace le stub Sprint 0)
**Sprints ciblés** : Sprint 8 (registry MVP + policy) → Sprint 9 (evolution + composition + persistence) → Sprint 10 (governance + admin API)
**Dépend de** : M1 (Core Runtime, `AlignmentCone`, `EfeError`), M4 (ELS — lecture read-only), M8 (`AlignmentProjector`, `AlignmentBreach` events)
**Consommé par** : M1 Controller (résolution du cone au build de spec), M3 Genesis (validation spec), M8 Dissolution (projection + breach reporting), M11 Telemetry, M12 FFI/Server (admin API)
**Statut** : spec **gelée** pour implémentation — aucune ambiguïté subsistante. Toute dérive sans ADR rejetée en revue.

---

## Table des matières

1. [Mission et positionnement](#1-mission-et-positionnement)
2. [Scope MVP vs hors-périmètre](#2-scope-mvp-vs-hors-périmètre)
3. [Théorie — cônes d'alignement et algèbre associée](#3-théorie--cônes-dalignement-et-algèbre-associée)
4. [Layout de module et vue d'ensemble](#4-layout-de-module-et-vue-densemble)
5. [Types fondamentaux](#5-types-fondamentaux)
6. [Traits et abstractions](#6-traits-et-abstractions)
7. [PolicyEngine — validation des invariants](#7-policyengine--validation-des-invariants)
8. [Opérateurs de composition](#8-opérateurs-de-composition)
9. [Opérateurs d'évolution](#9-opérateurs-dévolution)
10. [InMemoryRegistry — implémentation MVP](#10-inmemoryregistry--implémentation-mvp)
11. [Breach logging et audit trail](#11-breach-logging-et-audit-trail)
12. [Événements et télémétrie](#12-événements-et-télémétrie)
13. [Intégration non-breaking — SwarmSpecBuilder::from_registry](#13-intégration-non-breaking--swarmspecbuilderfrom_registry)
14. [Backends de persistence (post-MVP)](#14-backends-de-persistence-post-mvp)
15. [Modèle de sécurité et threat model](#15-modèle-de-sécurité-et-threat-model)
16. [Budget de performance](#16-budget-de-performance)
17. [Interface publique — re-exports](#17-interface-publique--re-exports)
18. [Stratégie de test](#18-stratégie-de-test)
19. [Dépendances inter-modules](#19-dépendances-inter-modules)
20. [Plan de sprint M9](#20-plan-de-sprint-m9)
21. [Plan de migration depuis l'existant](#21-plan-de-migration-depuis-lexistant)
22. [Matrice de risques et mitigations](#22-matrice-de-risques-et-mitigations)
23. [Critères d'acceptation](#23-critères-dacceptation)
24. [Annexes](#24-annexes)

---

## 1. Mission et positionnement

### 1.1 Mission fonctionnelle

M9 est la **gouvernance centralisée des cônes d'alignement** pour EFE. Avant
M9, chaque `SwarmSpec` portait son propre `AlignmentCone` construit **inline**
par le client — validation locale, aucune versioning, aucune traçabilité
inter-requêtes, aucun quota. M9 introduit un **registre versionné**,
**policy-driven**, **thread-safe**, qui agit comme **source de vérité unique**
pour les cônes d'alignement utilisés par la Dissolution (M8) et référencés par
le Genesis (M3).

En une phrase : *M9 transforme l'`AlignmentCone` — aujourd'hui une donnée
de configuration libre — en un **artefact gouverné**, identifiable par une
clé stable `(tenant, task, version)`, validé par une policy, auditable via un
log de breaches et d'événements, évolutif via des opérations explicites.*

### 1.2 Pourquoi M9 est critique

Sans M9, les pathologies structurelles suivantes persistent :

1. **Dérive silencieuse des cônes.** Chaque client peut muter son cône
   d'une requête à la suivante. Impossible de reproduire un run antérieur
   si le cône a changé — rejoue = sortie différente = bug non-déterministe
   perçu par l'utilisateur.
2. **Absence de quota.** Un cône de 10 000 anchors passerait silencieusement
   la validation `AlignmentCone::new()` (qui ne vérifie que : non-vide +
   distance positive finie), pénalisant M5 (attention O(K) supplémentaire)
   et M8 (projection O(K·D)) sans recours possible côté opérateur.
3. **Pas de traçabilité des breaches.** M8 émet `EfeError::AlignmentBreach
   { delta, max }` mais on ne sait pas : quel cône ? quel tenant ? quel
   swarm_id ? quel round ? Impossible de corréler un pic de breaches à un
   changement de cône.
4. **Pas de gouvernance multi-tenant.** Deux tenants peuvent accidentellement
   partager le même cône (copie-collé), sans barrière technique. Un tenant
   peut aussi fournir un cône "hostile" (ancres dégénérées, cosine ≈ 1
   partout) qui rend le projecteur inutile.
5. **Pas de composition propre.** Un workflow multi-skill (ex. "sem + log")
   requiert un cône combiné. Sans M9, chaque équipe code sa propre fusion
   ad-hoc, avec bugs d'alignement de dimension, normalisation incorrecte,
   non-déterminisme.
6. **Pas d'historique d'évolution.** Quand un cône est ajusté (ex. ajout
   d'un anchor "code-review"), il n'y a aucun lien avec la version précédente.
   Rollback impossible.

M9 résout ces six problèmes avec un modèle **`Registry → Policy → Events → Log`**
strictement read-heavy / write-infrequent, thread-safe, testable en isolation.

### 1.3 Contraintes et garanties

| # | Contrainte | Justification |
|---|-----------|---------------|
| C1 | **Read-heavy, write-infrequent.** Lookup O(1) mandatoire. | Le cône est lu à chaque `SwarmSpec::build` (plusieurs milliers par seconde). Il est créé rarement (~1/jour/tenant). |
| C2 | **Thread-safe.** `Send + Sync` sur tous les types publics. | Plusieurs workers M1 lookent en parallèle. |
| C3 | **Immutabilité par version.** `evolve` crée une **nouvelle** version, ne mute jamais une version existante. | Garantit reproductibilité : un `ConeKey` identique ⇒ même cône à vie. |
| C4 | **Opt-in, non-breaking.** M3 et M8 continuent de fonctionner sans M9. | Le rollout doit être progressif. Aucun code existant ne peut casser au merge de M9. |
| C5 | **Déterministe.** `lookup(key)` est une fonction pure. | Requis pour re-run bit-exact. |
| C6 | **Borné en ressources.** Breach log = ring buffer ; versions cap par task. | Un tenant malveillant ne peut pas remplir le registre indéfiniment. |
| C7 | **Policy-driven.** Toute validation passe par `PolicyEngine::check`. | Rends les règles de gouvernance lisibles, testables, versionables. |
| C8 | **Audit complet.** Toute mutation émet un `RegistryEvent`. | M11 consomme les events pour dashboards + alerting. |

### 1.4 Position dans le pipeline

```
             ┌─────────────────────────────────────────────────┐
             │     M10 API gRPC / M12 FFI (admin endpoints)    │
             └────────────┬──────────────────────────┬─────────┘
                          │ register / evolve        │ lookup
                          ▼                          ▼
                 ┌──────────────────────────────────────────────┐
                 │            M9 — Alignment Registry            │
                 │  ┌──────────┐  ┌─────────┐  ┌──────────────┐  │
                 │  │PolicyEng.│→ │Registry │→ │ BreachLog    │  │
                 │  │(invrnts) │  │(RwLock) │  │ (ring)       │  │
                 │  └──────────┘  └────┬────┘  └──────────────┘  │
                 │                     │          ▲              │
                 │  ┌─────────────┐    │          │ breach reports
                 │  │ Composition │────┘          │              │
                 │  │ union/blend │               │              │
                 │  │ intersect   │               │              │
                 │  └─────────────┘               │              │
                 │                                │              │
                 │  ┌─────────────┐               │              │
                 │  │ Evolution   │               │              │
                 │  │ ops apply() │               │              │
                 │  └─────────────┘               │              │
                 │                                │              │
                 └────────┬───────────────────────┼──────────────┘
                          │ lookup                │ report_breach
                          ▼                       │
                 ┌──────────────────┐      ┌──────────────────┐
                 │  M3 Genesis      │      │  M8 Dissolution  │
                 │  (spec build)    │      │  (proj + check)  │
                 └──────────────────┘      └──────────────────┘
                          │                       │
                          ▼                       ▼
                     SwarmSpec                AlignmentBreach
                                              → M9::report_breach
```

### 1.5 Garanties offertes aux modules avals

1. **Lookup idempotent et déterministe** : `registry.get(&key)` deux fois
   retourne la même `Arc<CanonicalCone>` (adresse potentiellement différente
   mais contenu bit-identique).
2. **Intégrité policy** : un cône **stocké** dans le registre a **forcément**
   passé `PolicyEngine::check` au moment de `register` ou `evolve`. Un client
   lisant un cône peut sauter sa propre validation.
3. **Traçabilité parent** : chaque `CanonicalCone` évolué porte son
   `parent: ConeKey` + `evolution_op: ConeEvolutionOp`. Le graphe ancestral
   est reconstructible par `list_versions(tenant, task)`.
4. **Breach log borné** : taille ≤ `breach_log_capacity` ; éviction FIFO.
5. **Pas de side-effect global** : `InMemoryRegistry` n'écrit nulle part
   hors de son état interne ; l'ajout de persistence est opt-in.

---

## 2. Scope MVP vs hors-périmètre

### 2.1 IN — livré dans le MVP

| Fonctionnalité | Sprint | Justification |
|---|---|---|
| Trait `AlignmentRegistry` (DI) | S8 | Permet swap backend sans toucher callers. |
| `InMemoryRegistry` thread-safe | S8 | Seul backend MVP requis (prod single-node). |
| `PolicyEngine` avec 7 invariants | S8 | Bloque les cônes mal-formés à l'entrée. |
| Types `ConeKey`, `CanonicalCone`, `ConeMetadata`, `ConeHandle` | S8 | Fondation types. |
| 3 opérateurs de composition pure : `union_cones`, `blend_cones`, `intersect_cones` | S9 | Nécessaires pour workflows multi-skill. |
| 6 `ConeEvolutionOp` : `AppendAnchor`, `RemoveAnchor`, `ReplaceAnchor`, `UpdateRadius`, `ScaleAnchors`, `UnionWith` | S9 | Couvre 95 % des cas d'évolution. |
| `BreachReport` + `BreachLog` ring buffer | S8 | Traçabilité minimale des violations. |
| `RegistryEvent` enum + émission vers `TelemetryCollector` | S9 | Alimente M11. |
| `SwarmSpecBuilder::from_registry(&registry, &key).await` | S9 | Intégration opt-in. |
| Validator pur `validate_cone(&cone, &cfg) -> Result<()>` | S8 | Réutilisable hors registry. |
| Tests unit/intégration/concurrence/fuzz | S8-S10 | Voir §18. |

### 2.2 OUT — explicitement hors MVP

| Fonctionnalité | Motif | Sprint cible post-MVP |
|---|---|---|
| Backend Postgres / fichier | Pas de besoin multi-node au MVP. `InMemoryRegistry` suffit pour un seul worker. | P1 (Sprint 13+). |
| Signature cryptographique des cônes | Dépend d'un KMS pas encore déployé. Niveau de threat actuel n'en justifie pas la complexité. | P2. |
| Admin UI web | Le CLI `cargo run --bin efectl` couvre les besoins MVP. | P2. |
| Replication multi-region | Mono-région au MVP. | P2. |
| Quotas dynamiques par tenant (debit) | Pas de multi-tenant à densité élevée au MVP. | P1. |
| Diff visuel / graphique entre versions | Nice-to-have, pas bloquant. | P1. |
| Cône à géométrie non-convexe (ellipsoïde, etc.) | Ajoute une famille de projecteurs à spec, hors scope. | P3. |
| ML-driven cone learning | Post-MVP recherche. | P3+. |
| Approbation workflow (2-person rule) | Pas de use case MVP. | P2. |

### 2.3 Frontières claires avec M8

- **M9** stocke et valide les cônes. Il **n'applique pas** la projection.
- **M8** applique la projection via `AlignmentProjector`. Il **n'enregistre pas**
  les cônes.
- **Point de contact** : M8 reçoit un `AlignmentCone` (struct `SwarmSpec.alignment_cone`)
  et, en cas de breach, appelle `registry.report_breach(&cone_key, delta, swarm_id, ctx)`.
- **Couplage faible** : M8 ne dépend de M9 qu'au travers du trait
  `BreachReporter` (sous-trait minimal extrait de `AlignmentRegistry`). M8
  peut tourner sans M9 (avec un `NoopBreachReporter`).

---

## 3. Théorie — cônes d'alignement et algèbre associée

### 3.1 Définition formelle du cône

Un **cône d'alignement** `C = (A, r)` sur ℝ^D est spécifié par :

- `A = {a_1, ..., a_K}` : un ensemble fini de **K ancres**, `a_i ∈ ℝ^D`,
  toutes finies, de norme ≥ `min_anchor_norm`.
- `r ∈ ℝ_+*` : le **rayon de projection maximum** (`max_projection_distance`),
  fini et strictement positif.

Le cône **admissible** est l'ensemble :

```
  Adm(C) = { x ∈ ℝ^D : d(x, H(A)) ≤ r }
```

où `H(A)` est l'**enveloppe convexe** des ancres, et `d(·,·)` la distance L2.

**Intuition.** Le cône est une "bulle tubulaire" autour du polytope défini
par les ancres. Tout vecteur à distance ≤ r de ce polytope est accepté ;
au-delà, c'est un **breach**.

### 3.2 Projection strict (rappel M8)

Le projecteur actuel (`StrictAlignmentProjector`) applique :

```
  π(x) = (1-α) · x + α · ( Σ_i w_i · a_i )
```

où `w_i = max(0, cos(x, a_i))` normalisés à somme 1, et `α = config.projection_strength`.

**Breach si** : `||x - π(x)||_2 > r · (1 + breach_tolerance)`.

M9 ne redéfinit **pas** cette projection — il garde le projecteur comme
boîte noire et se concentre sur la **vie du cône** (création, validation,
évolution, audit).

### 3.3 Algèbre de cônes

M9 introduit trois opérateurs ∈ `Cone × Cone → Cone` :

| Opérateur | Définition formelle | Use case |
|-----------|--------------------|----------|
| **Union** `C₁ ∪ C₂` | `A = A₁ ∪ A₂`, `r = max(r₁, r₂)` | Additionner deux skills (ex. SEM + LOG). Accepte strictement plus de points. |
| **Blend** `β(C₁, C₂, t)`, t ∈ [0,1] | `A_i = (1-t)·a_i^{(1)} + t·a_i^{(2)}` (si `K₁=K₂`, sinon erreur) ; `r = (1-t)·r₁ + t·r₂` | Interpolation douce entre deux alignements (A/B testing, transition graduelle). |
| **Intersection** `C₁ ∩ C₂` | `A = { a_i ∈ A₁ : ∃ a_j ∈ A₂, cos(a_i,a_j) ≥ θ }` (θ = `cosine_match_threshold`) ; `r = min(r₁, r₂)` | Exiger l'alignement conjoint à deux politiques. Plus restrictif. |

**Propriétés vérifiées par les tests M9** :

1. **Idempotence** : `C ∪ C = C`, `C ∩ C = C`, `β(C,C,t) = C`.
2. **Commutativité** de `∪` et `∩`.
3. **Associativité** de `∪` testée jusqu'à 3 cônes.
4. **Non-mutation** : `union_cones(&a, &b)` ne mute ni `a` ni `b`.
5. **Préservation des invariants** : le résultat passe `PolicyEngine::check`
   **sauf** violation explicite (ex. `|A₁| + |A₂| > max_anchors` sur union).

### 3.4 Évolution de cône

Un cône évolue via une **opération explicite** (`ConeEvolutionOp`) qui
produit une **nouvelle version** :

```
  evolve(C_{v=n}, op) = C_{v=n+1}, avec metadata.parent = key(C_{v=n})
                                       metadata.evolution_op = Some(op)
```

**Jamais de mutation en place.** Toute évolution est append-only.

### 3.5 Invariants formels

Pour tout `CanonicalCone` stocké, le `PolicyEngine` garantit :

- **I1** `0 < K ≤ max_anchors`
- **I2** `0 < D ≤ max_dim`
- **I3** `min_radius ≤ r ≤ max_radius`
- **I4** `∀ i, ||a_i||_2 ≥ min_anchor_norm`
- **I5** `∀ i,j (i≠j), cos(a_i, a_j) < anchor_cosine_dup_threshold`
- **I6** `!forbid_k_gt_d ∨ K ≤ D`
- **I7** `∀ i, ∀ d, a_i[d] is finite` (pas de NaN / ±Inf)

Ces 7 invariants sont vérifiés **avant** tout `register` et **après** tout
`evolve` / composition. L'ordre d'évaluation est : I7 (fast) → I1 → I2 →
I3 → I4 → I5 → I6, pour fail-fast sur les erreurs les plus fréquentes.

---

## 4. Layout de module et vue d'ensemble

### 4.1 Arborescence des fichiers

```
efe-core/src/alignment/
├── mod.rs            # Façade : re-exports publics + doc module
├── key.rs            # ConeKey, parsing, Display/Hash/Eq
├── canonical.rs      # CanonicalCone + ConeMetadata + ConeHandle
├── registry.rs       # AlignmentRegistry trait + InMemoryRegistry
├── policy.rs         # PolicyEngine + PolicyConfig + RegistryConfig
├── validator.rs      # validate_cone (pur, sans state)
├── compose.rs        # union_cones / blend_cones / intersect_cones
├── evolution.rs      # ConeEvolutionOp enum + apply
├── breach.rs         # BreachReport + BreachContext + BreachLog
├── events.rs         # RegistryEvent pour M11
└── builder.rs        # SwarmSpecBuilder::from_registry extension
```

+ tests/alignment_e2e.rs (intégration)

### 4.2 Couplages entre fichiers

```
                       ┌────────┐
                       │ mod.rs │  ← re-exports
                       └───┬────┘
          ┌────────────┬───┼───┬──────────────┐
          ▼            ▼   ▼   ▼              ▼
    ┌─────────┐ ┌──────────┐┌─────┐┌───────────────┐
    │ key.rs  │ │canonical │││policy│ │ validator.rs │
    └─────────┘ │ .rs      ││ .rs │└───────────────┘
                └──────────┘└─────┘         ▲
                     ▲         ▲            │
                     │         │            │
                ┌────┴────┬────┴─────┐      │
                ▼         ▼          ▼      │
          ┌──────────┐ ┌─────────┐┌──────────┐
          │registry  │ │evolution││compose.rs│
          │.rs       │ │.rs      │└──────────┘
          └─────┬────┘ └─────────┘
                │
                ▼
          ┌──────────┐ ┌───────────┐
          │breach.rs │ │ events.rs │
          └──────────┘ └───────────┘
                ▲
                │
          ┌──────────┐
          │builder.rs│ (étend SwarmSpecBuilder)
          └──────────┘
```

### 4.3 Règles de dépendance interne

- **`key.rs`** : zéro dépendance intra-crate (sauf `types.rs` pour `TenantId`/`TaskId` nouveaux newtypes).
- **`canonical.rs`** : dépend de `key.rs`, `types::AlignmentCone`, `evolution::ConeEvolutionOp`.
- **`validator.rs`** : dépend de `types::AlignmentCone`, `policy::PolicyConfig`. Sans état, testable unitairement.
- **`policy.rs`** : dépend de `validator.rs`. Définit aussi `RegistryConfig`.
- **`compose.rs`** : dépend de `types::AlignmentCone`, `validator.rs` (re-validation du résultat).
- **`evolution.rs`** : dépend de `types::AlignmentCone`, `compose::union_cones` (pour `UnionWith`).
- **`breach.rs`** : dépend de `key.rs`. Aucun accès au registry.
- **`events.rs`** : dépend de `key.rs`, `breach.rs`.
- **`registry.rs`** : dépend de tout ci-dessus.
- **`builder.rs`** : dépend de `registry.rs` + `types::SwarmSpecBuilder`.
- **`mod.rs`** : re-exports uniquement.

Aucun cycle. Testable en isolation par fichier.

### 4.4 Conventions de code

- Tous les types publics `#[non_exhaustive]`.
- Toutes les fonctions publiques documentées (`#[warn(missing_docs)]` actif).
- `#[cfg_attr(not(test), deny(clippy::unwrap_used))]` respecté (aucun `unwrap` en prod).
- Utilisation obligatoire de `Result<T>` (alias crate) pour toute fonction fallible.
- Aucun `panic!` dans `alignment/` (utiliser `EfeError::Internal(&'static str)`).
- `Display` implémenté pour `ConeKey`, `BreachReport`, `RegistryEvent`.
- `Serialize + Deserialize` sur tous les types "données" (pour M12 admin API).
- `async_trait::async_trait` sur `AlignmentRegistry` pour compat Postgres.

---

## 5. Types fondamentaux

### 5.1 `ConeKey`

```rust
/// Identifiant stable d'un cône dans le registre.
///
/// Format canonique `tenant/task@v<version>`, ex. `acme/chat-qa@v3`.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ConeKey {
    pub tenant: String,
    pub task: String,
    pub version: u32,
}

impl ConeKey {
    /// Construit une clé, validant les champs.
    ///
    /// # Errors
    ///
    /// - `InvalidConfig` si `tenant` ou `task` vide ou contient
    ///   `/`, `@`, `:`, ou whitespace.
    /// - `InvalidConfig` si `version == 0` (les versions commencent à 1).
    pub fn new(tenant: impl Into<String>, task: impl Into<String>,
               version: u32) -> Result<Self> { ... }

    /// Parse la forme canonique `tenant/task@v<N>`.
    pub fn parse(s: &str) -> Result<Self> { ... }

    /// Clé de regroupement `tenant/task` (sans version).
    pub fn task_id(&self) -> String {
        format!("{}/{}", self.tenant, self.task)
    }
}

impl std::fmt::Display for ConeKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}@v{}", self.tenant, self.task, self.version)
    }
}
```

**Invariants** :
- `tenant.len() ≥ 1 && tenant.len() ≤ 64`
- `task.len() ≥ 1 && task.len() ≤ 128`
- `tenant` et `task` : `[A-Za-z0-9_-]+` (ASCII uniquement).
- `version ≥ 1`.

**Pourquoi pas UUID ?** Parce qu'un `ConeKey` doit être **lisible par un
humain** (dashboard, logs, runbook). UUID = opaque, pénible à corréler.

### 5.2 `ConeMetadata`

```rust
/// Métadonnées descriptives attachées à un CanonicalCone.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ConeMetadata {
    /// Nanos depuis l'epoch Unix au moment du register/evolve.
    pub created_at_nanos: u64,
    /// Identité qui a créé le cône (user/service). Free-form ≤ 128 chars.
    pub created_by: String,
    /// Description humaine ≤ 512 chars.
    pub description: String,
    /// Parent (pour un cône évolué), `None` si enregistré de zéro.
    pub parent: Option<ConeKey>,
    /// Opération qui a produit cette version, `None` si racine.
    pub evolution_op: Option<ConeEvolutionOp>,
    /// Étiquettes libres ; usage : filtrer dashboards, tagger A/B test.
    /// Max 16 entries, keys ≤ 64 chars, values ≤ 256 chars.
    pub labels: std::collections::BTreeMap<String, String>,
}
```

**Choix `BTreeMap` sur `HashMap`** : déterminisme de sérialisation (ordre
stable → diff JSON lisible dans les logs).

### 5.3 `CanonicalCone`

```rust
/// Le cône tel que stocké dans le registre : cône + métadonnées.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CanonicalCone {
    /// Le cône d'alignement lui-même (réutilise le type M1).
    pub cone: crate::types::AlignmentCone,
    /// Métadonnées.
    pub metadata: ConeMetadata,
}

impl CanonicalCone {
    /// Construit un cône canonical avec timestamp automatique.
    pub fn new(
        cone: crate::types::AlignmentCone,
        created_by: impl Into<String>,
        description: impl Into<String>,
    ) -> Self { ... }

    /// Getter court.
    pub fn anchors(&self) -> &[Vec<f32>] { &self.cone.anchors }
    pub fn radius(&self) -> f32 { self.cone.max_projection_distance }
    pub fn num_anchors(&self) -> usize { self.cone.anchors.len() }
    pub fn dim(&self) -> usize {
        self.cone.anchors.first().map(|a| a.len()).unwrap_or(0)
    }
}
```

**Important** : `CanonicalCone` est **Clone**, mais toute duplication par le
caller est discouragée (le registre stocke un `Arc<CanonicalCone>` et retourne
des `Arc`). Clone uniquement pour la sérialisation / debug / composition.

### 5.4 `ConeHandle`

```rust
/// Référence comptée vers un cône canonical + sa clé.
///
/// C'est le type retourné par `registry.get()` et `registry.register()`.
#[derive(Clone, Debug)]
pub struct ConeHandle {
    pub key: ConeKey,
    pub cone: std::sync::Arc<CanonicalCone>,
}

impl ConeHandle {
    /// Accès direct au cône sous-jacent.
    pub fn as_cone(&self) -> &crate::types::AlignmentCone {
        &self.cone.cone
    }
}
```

**Pattern `Arc<CanonicalCone>`** : évite les clones coûteux (un cône peut
faire 32 anchors × 2048 dim × 4 bytes = 256 KB).

### 5.5 `PolicyConfig`

```rust
/// Politique d'invariants applicable aux cônes.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PolicyConfig {
    pub max_anchors: usize,                  // I1 (default: 32)
    pub max_dim: usize,                      // I2 (default: 4096)
    pub min_radius: f32,                     // I3 lower (default: 0.01)
    pub max_radius: f32,                     // I3 upper (default: 10.0)
    pub min_anchor_norm: f32,                // I4 (default: 0.1)
    pub anchor_cosine_dup_threshold: f32,    // I5 (default: 0.99)
    pub forbid_k_gt_d: bool,                 // I6 (default: true)
}

impl Default for PolicyConfig { ... }

impl PolicyConfig {
    /// Valide les champs de la config eux-mêmes (meta-invariants).
    pub fn validate(&self) -> Result<()> { ... }
}
```

**Meta-invariants sur `PolicyConfig`** (vérifiés dans `validate`) :

- `max_anchors ≥ 1`
- `max_dim ≥ 1`
- `min_radius > 0`, `max_radius > min_radius`, tous deux finis
- `min_anchor_norm > 0`, finie
- `anchor_cosine_dup_threshold ∈ (0, 1)`
- Pas d'interaction pathologique possible (ex. `max_radius = f32::MAX` produit
  des breach tolérances explosives — vérifié à `PolicyConfig::validate`).

### 5.6 `RegistryConfig`

```rust
/// Configuration runtime du registre.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RegistryConfig {
    /// Nombre max de versions par (tenant, task). Au-delà, `register` fail.
    pub max_versions_per_task: u32,          // default: 16
    /// Capacité du ring buffer des breaches (global).
    pub breach_log_capacity: usize,          // default: 1024
    /// Policy appliquée à tout register/evolve.
    pub policy: PolicyConfig,
}

impl Default for RegistryConfig { ... }
impl RegistryConfig {
    pub fn validate(&self) -> Result<()> { ... }
}
```

### 5.7 `BreachReport` et `BreachContext`

```rust
/// Rapport d'un breach d'alignement observé.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BreachReport {
    pub cone_key: ConeKey,
    pub swarm_id: crate::types::SwarmId,
    pub observed_delta: f32,
    pub cone_radius: f32,
    pub timestamp_nanos: u64,
    pub context: BreachContext,
}

/// Où un breach a eu lieu dans le pipeline.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum BreachContext {
    /// En dissolution finale (M8).
    Dissolution,
    /// Pendant un round de fusion (M5 avec projection intra-round).
    FusionRound { round: u32 },
    /// Signalement manuel (admin, batch audit).
    Manual { reason: String },
}

impl std::fmt::Display for BreachReport { ... }
```

**`observed_delta` vs `cone_radius`** : on logge **les deux** pour permettre
un debug sans devoir retrouver la config du cône (qui peut avoir été évoluée
depuis). L'écart `observed_delta - cone_radius` est la **magnitude du breach**.

### 5.8 `ConeEvolutionOp`

```rust
/// Opération de modification appliquée à un cône existant.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub enum ConeEvolutionOp {
    /// Ajoute un nouvel anchor. Fail si ajouter dépasse `max_anchors`.
    AppendAnchor { anchor: Vec<f32> },
    /// Supprime l'anchor à l'index donné. Fail si `idx >= K` ou si K == 1
    /// (un cône doit garder ≥ 1 anchor).
    RemoveAnchor { idx: usize },
    /// Remplace l'anchor à l'index donné par un nouveau.
    ReplaceAnchor { idx: usize, anchor: Vec<f32> },
    /// Change le rayon. Doit rester dans [min_radius, max_radius].
    UpdateRadius { new_radius: f32 },
    /// Multiplie tous les anchors par `factor`. Utile pour renormaliser.
    ScaleAnchors { factor: f32 },
    /// Union avec un autre cône (référencé par sa clé).
    UnionWith { other: ConeKey },
}
```

**Propriétés** :
- Chaque variant est **atomique** et produit un **nouveau** `CanonicalCone`
  à passer à `register`.
- La validation de policy est re-exécutée après l'op.
- `UnionWith` est asynchrone (besoin du lookup de `other`) ⇒ implémentée
  seulement dans `AlignmentRegistry::evolve`, pas dans `apply` pur.

### 5.9 `RegistryEvent`

```rust
/// Événements émis par le registre à destination de M11.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum RegistryEvent {
    Registered { key: ConeKey, at_nanos: u64 },
    Evolved { from: ConeKey, to: ConeKey, op_kind: &'static str },
    Deleted { key: ConeKey },
    BreachReported { report: BreachReport },
    PolicyRejected {
        attempted_key: Option<ConeKey>,
        reason: &'static str,
    },
}
```

**`&'static str` pour `reason`** : les motifs de rejet sont un ensemble fini
connu à la compilation (ex. `"I1: K exceeds max_anchors"`). Aucun alloc sur
le hot path de rejet.

---

## 6. Traits et abstractions

### 6.1 `AlignmentRegistry` — trait principal

```rust
/// Contrat du registre de cônes. Async pour compat Postgres future.
#[async_trait::async_trait]
pub trait AlignmentRegistry: Send + Sync + std::fmt::Debug {
    /// Enregistre un nouveau cône à la clé donnée.
    ///
    /// - La version dans `key` doit être **strictement supérieure**
    ///   à toute version existante pour `(tenant, task)`, sinon erreur.
    /// - Policy vérifiée avant insertion.
    ///
    /// # Errors
    /// - `InvalidConfig` si policy violée.
    /// - `InvalidConfig` si version non monotone.
    /// - `InvalidConfig` si `max_versions_per_task` dépassé.
    async fn register(
        &self,
        key: ConeKey,
        cone: CanonicalCone,
    ) -> Result<ConeHandle>;

    /// Lookup par clé exacte.
    ///
    /// # Errors
    /// - `InvalidConfig("unknown cone key")` si absent.
    async fn get(&self, key: &ConeKey) -> Result<ConeHandle>;

    /// Liste toutes les versions existantes pour `(tenant, task)`, triées
    /// par version croissante. Vecteur vide si aucune.
    async fn list_versions(
        &self,
        tenant: &str,
        task: &str,
    ) -> Result<Vec<ConeKey>>;

    /// Fait évoluer un cône existant en appliquant une opération.
    /// Crée une nouvelle version (version précédente + 1).
    ///
    /// # Errors
    /// - `InvalidConfig` si `from` inexistant, policy violée, op invalide.
    async fn evolve(
        &self,
        from: ConeKey,
        op: ConeEvolutionOp,
        metadata_override: Option<ConeMetadata>,
    ) -> Result<ConeHandle>;

    /// Supprime une version précise. Ne supprime jamais la dernière version
    /// d'un (tenant, task) sauf si `force = true` (guard anti-footgun).
    async fn delete(&self, key: &ConeKey, force: bool) -> Result<()>;

    /// Retourne les `n` derniers breaches, du plus récent au plus ancien.
    /// Filtre optionnel par `tenant` / `task`.
    async fn breach_history(
        &self,
        n: usize,
        filter: Option<BreachFilter>,
    ) -> Result<Vec<BreachReport>>;

    /// Enregistre un breach observé par M8 (ou autre module).
    /// Ne bloque pas — même si le cône n'existe plus, le rapport est loggé.
    async fn report_breach(&self, report: BreachReport) -> Result<()>;

    /// Nom du backend pour télémétrie (ex. "in_memory", "postgres").
    fn backend_name(&self) -> &'static str;
}

/// Filtre pour breach_history.
#[derive(Clone, Debug, Default)]
pub struct BreachFilter {
    pub tenant: Option<String>,
    pub task: Option<String>,
    pub swarm_id: Option<crate::types::SwarmId>,
}
```

### 6.2 Sous-trait `BreachReporter`

```rust
/// Sous-trait minimal pour M8 — ne dépend pas du registre complet.
#[async_trait::async_trait]
pub trait BreachReporter: Send + Sync + std::fmt::Debug {
    async fn report_breach(&self, report: BreachReport) -> Result<()>;
}

// `AlignmentRegistry` l'implémente gratuitement :
#[async_trait::async_trait]
impl<R: AlignmentRegistry + ?Sized> BreachReporter for R {
    async fn report_breach(&self, r: BreachReport) -> Result<()> {
        AlignmentRegistry::report_breach(self, r).await
    }
}
```

**Bénéfice** : M8 peut dépendre de `Arc<dyn BreachReporter>` sans avoir à
importer tout `AlignmentRegistry`. Découplage fin.

### 6.3 Trait `CompositionOp` (helper interne)

```rust
/// Signature des 3 opérateurs de composition pour tests unifiés.
pub(crate) trait CompositionOp {
    fn apply(
        a: &crate::types::AlignmentCone,
        b: &crate::types::AlignmentCone,
    ) -> Result<crate::types::AlignmentCone>;
    fn name() -> &'static str;
}
```

Les structs `UnionOp`, `BlendOp { t: f32 }`, `IntersectOp { threshold: f32 }`
implémentent ce trait. Les fonctions publiques sont des wrappers :

```rust
pub fn union_cones(a: &AlignmentCone, b: &AlignmentCone) -> Result<AlignmentCone>
pub fn blend_cones(a: &AlignmentCone, b: &AlignmentCone, t: f32) -> Result<AlignmentCone>
pub fn intersect_cones(a: &AlignmentCone, b: &AlignmentCone, threshold: f32) -> Result<AlignmentCone>
```

---

## 7. PolicyEngine — validation des invariants

### 7.1 Contrat

`PolicyEngine::check(&cone) -> Result<()>` vérifie les 7 invariants (§3.5).
Fonction **pure**, **sans allocation** sur le chemin succès, **sans I/O**.

### 7.2 Ordre d'évaluation

L'ordre est choisi pour **fail-fast sur les cas fréquents** et **minimiser
le coût** en cas de rejet :

1. **I7** — finiteness. O(K·D). **Premier** car un NaN silencieux plus loin
   produit des comportements erratiques.
2. **I1** — `K ≤ max_anchors`. O(1).
3. **I2** — `D ≤ max_dim`. O(1).
4. **I3** — radius bounds. O(1).
5. **I4** — norms. O(K·D).
6. **I5** — pairwise cosine. **O(K²·D)**. **Dernier** car le plus cher.
7. **I6** — K vs D. O(1).

### 7.3 Pseudocode complet

```rust
impl PolicyEngine {
    pub fn check(&self, cone: &AlignmentCone) -> Result<()> {
        let k = cone.anchors.len();
        if k == 0 { return Err(policy_err("I1: K = 0")); }
        let d = cone.anchors[0].len();
        if d == 0 { return Err(policy_err("I2: D = 0")); }

        // I7 — finiteness (fail-fast).
        for (i, a) in cone.anchors.iter().enumerate() {
            if a.len() != d {
                return Err(policy_err(
                    "I2: anchors have inconsistent dimensions"));
            }
            for &v in a {
                if !v.is_finite() {
                    return Err(policy_err("I7: non-finite anchor"));
                }
            }
        }
        if !cone.max_projection_distance.is_finite() {
            return Err(policy_err("I7: non-finite radius"));
        }

        // I1, I2, I3.
        if k > self.cfg.max_anchors {
            return Err(policy_err("I1: K exceeds max_anchors"));
        }
        if d > self.cfg.max_dim {
            return Err(policy_err("I2: D exceeds max_dim"));
        }
        let r = cone.max_projection_distance;
        if r < self.cfg.min_radius {
            return Err(policy_err("I3: radius below min_radius"));
        }
        if r > self.cfg.max_radius {
            return Err(policy_err("I3: radius above max_radius"));
        }

        // I4 — norms.
        for a in &cone.anchors {
            let n2 = a.iter().map(|v| v * v).sum::<f32>();
            if n2.sqrt() < self.cfg.min_anchor_norm {
                return Err(policy_err("I4: anchor norm below min"));
            }
        }

        // I5 — pairwise cosine. Précalc des normes pour éviter O(K²·D)*2.
        let norms: Vec<f32> = cone.anchors.iter()
            .map(|a| a.iter().map(|v| v * v).sum::<f32>().sqrt())
            .collect();
        for i in 0..k {
            for j in (i + 1)..k {
                let dot = cone.anchors[i].iter()
                    .zip(&cone.anchors[j])
                    .map(|(a, b)| a * b)
                    .sum::<f32>();
                let denom = norms[i] * norms[j];
                // denom > 0 car I4 déjà vérifié → min_anchor_norm > 0.
                let cos = dot / denom;
                if cos >= self.cfg.anchor_cosine_dup_threshold {
                    return Err(policy_err(
                        "I5: pairwise cosine exceeds dup threshold"));
                }
            }
        }

        // I6 — K vs D.
        if self.cfg.forbid_k_gt_d && k > d {
            return Err(policy_err("I6: K > D forbidden"));
        }

        Ok(())
    }
}

fn policy_err(reason: &'static str) -> EfeError {
    EfeError::InvalidConfig(format!("policy violation: {reason}"))
}
```

### 7.4 Complexité

| Cas | Opérations dominantes | Complexité |
|---|---|---|
| Succès | I7 (K·D) + I4 (K·D) + I5 (K²·D) | **O(K²·D)** |
| Échec I7 | O(somme_préfixe) | O(K·D) worst-case |
| Échec I1-I3 | O(1) après I7 | O(K·D) |
| Échec I5 | O(i·j·D) jusqu'au premier conflit | ≤ O(K²·D) |

Pour `K=32, D=4096` (max MVP), coût pire cas ≈ `32² × 4096 ≈ 4.2M` FMAs,
≈ 2 ms en scalaire, &lt; 100 µs avec AVX2 (auto-vectorisé par rustc).

### 7.5 Placement dans le code

- **`validator.rs`** : exporte `validate_cone(cone, policy) -> Result<()>`,
  qui construit localement un `PolicyEngine` et appelle `check`. Pratique
  pour les tests et la composition.
- **`policy.rs`** : définit `PolicyEngine::new(PolicyConfig)`, `check`,
  `config()`.
- **`registry.rs`** : `InMemoryRegistry` garde un `PolicyEngine` interne,
  appelle `check` avant chaque `register`/`evolve`.

### 7.6 Tests dédiés PolicyEngine

1. `default_config_accepts_minimal_valid_cone`
2. `rejects_zero_anchors` (I1)
3. `rejects_too_many_anchors`
4. `rejects_zero_dim` (I2)
5. `rejects_dim_too_large`
6. `rejects_inconsistent_anchor_dims`
7. `rejects_negative_radius`
8. `rejects_nan_radius`
9. `rejects_infinite_radius`
10. `rejects_radius_above_max` / `below_min`
11. `rejects_near_zero_norm_anchor` (I4)
12. `rejects_duplicate_anchors_by_cosine` (I5 avec seuil 0.99)
13. `accepts_orthogonal_anchors` (cos ≈ 0)
14. `accepts_K_le_D` / `rejects_K_gt_D_when_forbidden`
15. `allows_K_gt_D_when_flag_off`
16. `rejects_nan_anchor` (I7)
17. `rejects_inf_anchor`
18. `error_message_identifies_failing_invariant` (regression)

---

## 8. Opérateurs de composition

### 8.1 `union_cones`

```rust
/// Union : concatène les anchors, prend le max des rayons.
///
/// # Erreurs
/// - Dimensions incompatibles entre les deux cônes.
/// - Résultat `K₁ + K₂` qui dépasserait `max_anchors` (re-validation).
pub fn union_cones(
    a: &AlignmentCone,
    b: &AlignmentCone,
) -> Result<AlignmentCone> {
    if a.anchors[0].len() != b.anchors[0].len() {
        return Err(EfeError::InvalidConfig(
            "union_cones: dimension mismatch".into(),
        ));
    }
    let mut anchors = Vec::with_capacity(a.anchors.len() + b.anchors.len());
    anchors.extend_from_slice(&a.anchors);
    anchors.extend_from_slice(&b.anchors);
    let radius = a.max_projection_distance.max(b.max_projection_distance);
    AlignmentCone::new(anchors, radius)
}
```

**Propriétés** :
- `K_out = K_a + K_b` (pas de dedup : la policy I5 peut rejeter si doublons).
- `r_out ≥ max(r_a, r_b) ≥ min(r_a, r_b)` : union strictement plus large.
- Commutatif par définition (`a ∪ b = b ∪ a` modulo ordre des anchors).

**Tests** :
- `union_is_commutative_up_to_anchor_order`
- `union_radius_is_max`
- `union_rejects_dim_mismatch`
- `union_preserves_finite_anchors`
- `union_keeps_anchor_data_intact` (pas de mutation des inputs)

### 8.2 `blend_cones`

```rust
/// Blend : combinaison convexe entre deux cônes de même cardinalité et
/// même dimension.
///
/// # Erreurs
/// - `K_a != K_b` ⇒ `InvalidConfig`.
/// - Dimensions différentes ⇒ `InvalidConfig`.
/// - `t ∉ [0, 1]` ⇒ `InvalidConfig`.
pub fn blend_cones(
    a: &AlignmentCone,
    b: &AlignmentCone,
    t: f32,
) -> Result<AlignmentCone> {
    if !(0.0..=1.0).contains(&t) || !t.is_finite() {
        return Err(EfeError::InvalidConfig(
            "blend_cones: t must be in [0, 1]".into(),
        ));
    }
    if a.anchors.len() != b.anchors.len() {
        return Err(EfeError::InvalidConfig(
            "blend_cones: anchor count mismatch".into(),
        ));
    }
    let d = a.anchors[0].len();
    if b.anchors[0].len() != d {
        return Err(EfeError::InvalidConfig(
            "blend_cones: dim mismatch".into(),
        ));
    }
    let anchors = a.anchors.iter().zip(&b.anchors)
        .map(|(ai, bi)| {
            ai.iter().zip(bi)
                .map(|(x, y)| (1.0 - t) * x + t * y)
                .collect::<Vec<f32>>()
        })
        .collect::<Vec<_>>();
    let radius = (1.0 - t) * a.max_projection_distance
                 + t * b.max_projection_distance;
    AlignmentCone::new(anchors, radius)
}
```

**Propriétés** :
- `t=0` ⇒ `a`, `t=1` ⇒ `b` (continuité).
- `t=0.5` ⇒ milieu géométrique par coordonnée.
- La pairing est **positional** : `a.anchors[i]` ↔ `b.anchors[i]`. Le caller
  est responsable de l'alignement sémantique.

**Note** : pour un blend robuste à l'ordre, utiliser `align_anchors` (helper
dans `compose.rs`) qui fait un Hungarian matching sur cosine. Non MVP.

### 8.3 `intersect_cones`

```rust
/// Intersection : garde les ancres de `a` qui ont un partenaire proche
/// dans `b` (cosine ≥ threshold). Rayon = min.
///
/// # Erreurs
/// - Dim mismatch.
/// - `threshold ∉ (-1, 1]`.
/// - Aucune ancre retenue ⇒ `InvalidConfig("empty intersection")`.
pub fn intersect_cones(
    a: &AlignmentCone,
    b: &AlignmentCone,
    threshold: f32,
) -> Result<AlignmentCone> { ... }
```

**Propriétés** :
- **Pas commutatif** strictement : le résultat contient les ancres de `a`
  filtrées, pas celles de `b`.
- Pour une version commutative, utiliser `intersect_cones_mutual` (non MVP)
  qui garde les pairs mutuels.
- `r_out = min(r_a, r_b)` : intersection plus restrictive.

### 8.4 Re-validation post-composition

Toute fonction de composition termine par `AlignmentCone::new(...)` qui
valide "non-empty anchors + positive finite radius". **Mais pas la policy
complète.** Le caller (typiquement `evolve`) doit appeler `PolicyEngine::check`
sur le résultat avant `register`.

**Garantie** : les fonctions de composition ne produisent jamais un cône
"syntaxiquement invalide" (panic, NaN silencieux). Elles peuvent produire
un cône "policy-invalide" (trop d'anchors, doublons) — détecté à l'étape
suivante.

### 8.5 Tests d'algèbre

- `union_idempotent_on_self`
- `union_commutative`
- `union_associative_three_cones`
- `blend_t0_is_a`, `blend_t1_is_b`, `blend_t05_is_midpoint`
- `blend_rejects_t_outside_unit`
- `intersect_self_with_self_is_self_when_threshold_less_than_1`
- `intersect_orthogonal_cones_is_empty_error`
- `composition_preserves_finiteness`
- `composition_pure_no_mutation_of_inputs`

---

## 9. Opérateurs d'évolution

### 9.1 Sémantique générale

`evolve(from, op)` est l'opération de mutation **logique** sur un cône :
- Lookup le cône à `from`.
- Applique `op` en produisant un nouveau `AlignmentCone`.
- Valide via `PolicyEngine::check`.
- Bump la version : `new_version = max_version_for_(tenant,task) + 1`.
- Attache `parent = Some(from)`, `evolution_op = Some(op)`, nouveau timestamp.
- `register` le résultat.
- Émet `RegistryEvent::Evolved`.

### 9.2 Pseudocode de `evolve`

```rust
async fn evolve(&self, from: ConeKey, op: ConeEvolutionOp,
                metadata_override: Option<ConeMetadata>)
    -> Result<ConeHandle>
{
    let parent_handle = self.get(&from).await?;
    let parent_cone = &parent_handle.cone.cone;

    // Applique l'op (peut échouer → InvalidConfig).
    let new_cone = match &op {
        ConeEvolutionOp::AppendAnchor { anchor } =>
            apply_append(parent_cone, anchor)?,
        ConeEvolutionOp::RemoveAnchor { idx } =>
            apply_remove(parent_cone, *idx)?,
        ConeEvolutionOp::ReplaceAnchor { idx, anchor } =>
            apply_replace(parent_cone, *idx, anchor)?,
        ConeEvolutionOp::UpdateRadius { new_radius } =>
            apply_update_radius(parent_cone, *new_radius)?,
        ConeEvolutionOp::ScaleAnchors { factor } =>
            apply_scale(parent_cone, *factor)?,
        ConeEvolutionOp::UnionWith { other } => {
            let other_handle = self.get(other).await?;
            union_cones(parent_cone, &other_handle.cone.cone)?
        }
    };

    // Validation policy.
    self.policy.check(&new_cone)?;

    // Nouvelle clé : version + 1.
    let latest = self.max_version_for(&from.tenant, &from.task).await?;
    let new_key = ConeKey::new(&from.tenant, &from.task, latest + 1)?;

    // Metadata avec parent.
    let metadata = metadata_override.unwrap_or_else(|| ConeMetadata {
        created_at_nanos: now_nanos(),
        created_by: parent_handle.cone.metadata.created_by.clone(),
        description: format!("evolved from {from} via {}", op.kind()),
        parent: Some(from.clone()),
        evolution_op: Some(op.clone()),
        labels: parent_handle.cone.metadata.labels.clone(),
    });

    let canonical = CanonicalCone { cone: new_cone, metadata };
    self.register(new_key.clone(), canonical).await
}
```

### 9.3 Implémentation des ops individuelles

#### 9.3.1 `apply_append`

```rust
fn apply_append(base: &AlignmentCone, anchor: &[f32]) -> Result<AlignmentCone> {
    if base.anchors.is_empty() {
        return Err(EfeError::Internal("apply_append: base has no anchors"));
    }
    let d = base.anchors[0].len();
    if anchor.len() != d {
        return Err(EfeError::InvalidConfig(format!(
            "append: anchor dim {} != cone dim {}", anchor.len(), d
        )));
    }
    let mut anchors = base.anchors.clone();
    anchors.push(anchor.to_vec());
    AlignmentCone::new(anchors, base.max_projection_distance)
}
```

#### 9.3.2 `apply_remove`

```rust
fn apply_remove(base: &AlignmentCone, idx: usize) -> Result<AlignmentCone> {
    if idx >= base.anchors.len() {
        return Err(EfeError::InvalidConfig(
            format!("remove: idx {} >= K {}", idx, base.anchors.len())
        ));
    }
    if base.anchors.len() == 1 {
        return Err(EfeError::InvalidConfig(
            "remove: refusing to empty the cone".into()
        ));
    }
    let mut anchors = base.anchors.clone();
    anchors.remove(idx);
    AlignmentCone::new(anchors, base.max_projection_distance)
}
```

#### 9.3.3 `apply_replace`

Combinaison logique d'`apply_remove` + `apply_append` à la bonne position.

#### 9.3.4 `apply_update_radius`

```rust
fn apply_update_radius(base: &AlignmentCone, new_radius: f32)
    -> Result<AlignmentCone>
{
    // AlignmentCone::new vérifie > 0 && finite.
    AlignmentCone::new(base.anchors.clone(), new_radius)
}
```

#### 9.3.5 `apply_scale`

```rust
fn apply_scale(base: &AlignmentCone, factor: f32) -> Result<AlignmentCone> {
    if !factor.is_finite() || factor == 0.0 {
        return Err(EfeError::InvalidConfig(
            "scale: factor must be finite and non-zero".into()
        ));
    }
    let anchors: Vec<Vec<f32>> = base.anchors.iter()
        .map(|a| a.iter().map(|v| v * factor).collect())
        .collect();
    AlignmentCone::new(anchors, base.max_projection_distance)
}
```

### 9.4 Tests évolution

- `append_adds_one_anchor`
- `append_rejects_dim_mismatch`
- `append_exceeding_max_anchors_rejected_by_policy`
- `remove_decreases_k`
- `remove_last_anchor_rejected`
- `remove_out_of_bounds_rejected`
- `replace_preserves_k_and_d`
- `update_radius_changes_only_radius`
- `update_radius_out_of_range_rejected`
- `scale_preserves_k_d_changes_norms`
- `scale_zero_rejected`
- `scale_nan_rejected`
- `evolve_creates_version_plus_one`
- `evolve_sets_parent_metadata`
- `evolve_unknown_parent_errors`
- `evolve_union_with_missing_other_errors`
- `evolve_preserves_immutability_of_parent`

---

## 10. InMemoryRegistry — implémentation MVP

### 10.1 Structure

```rust
pub struct InMemoryRegistry {
    inner: parking_lot::RwLock<Inner>,
    breach_log: parking_lot::Mutex<BreachLog>,
    events_tx: Option<tokio::sync::mpsc::UnboundedSender<RegistryEvent>>,
    policy: PolicyEngine,
    cfg: RegistryConfig,
    name: &'static str,
}

struct Inner {
    // key.to_string() → Arc<CanonicalCone>
    by_key: std::collections::HashMap<ConeKey, Arc<CanonicalCone>>,
    // "tenant/task" → sorted Vec<u32> of versions.
    versions_by_task: std::collections::HashMap<String, Vec<u32>>,
}
```

### 10.2 Choix `RwLock` vs `Mutex`

- **`RwLock` pour le catalogue principal** : lectures largement majoritaires
  (~99 %), `register`/`evolve`/`delete` rares. `parking_lot::RwLock` a un
  fast-path de lock sans overhead en lecture concurrente.
- **`Mutex` pour le breach log** : écritures fréquentes (chaque breach est
  rapide), lectures au debug uniquement. Ring buffer sous mutex suffit.
- **Pas de lock croisé** : une op `register` prend `inner.write()` puis
  **relâche** avant d'émettre l'event. Pas de deadlock possible.

### 10.3 Flux `register`

```
1. Policy check sur le cone.  [peut fail, retour immédiat]
2. Acquire inner.write().
3. Vérifier que key.version > max(versions_by_task.get(task_id)).
   Sinon: release lock + return InvalidConfig("non monotone version").
4. Vérifier que versions.len() < cfg.max_versions_per_task.
   Sinon: release lock + return InvalidConfig("too many versions").
5. Insert by_key[key] = Arc::new(canonical_cone).
6. Append version to versions_by_task[task_id], keep sorted.
7. Release write lock.
8. Emit RegistryEvent::Registered (best-effort, ignore channel full).
9. Return ConeHandle::clone (Arc clone = cheap).
```

### 10.4 Flux `get`

```
1. Acquire inner.read().
2. Lookup by_key[key].
3. Clone Arc (O(1) refcount bump).
4. Release read lock.
5. Return ConeHandle { key, cone: arc }.
```

**Pas d'event** émis sur `get` (trop bruyant). Les hits sont comptés par
M11 via un Prometheus counter géré par le caller si besoin.

### 10.5 Flux `evolve`

Déjà décrit §9.2. Note critique : le lookup du parent se fait **hors** du
write lock (appel `self.get(&from).await` qui prend read lock), puis le
`register` final reprend write lock. Pas de lock réentrant.

### 10.6 Flux `delete`

```rust
async fn delete(&self, key: &ConeKey, force: bool) -> Result<()> {
    let mut inner = self.inner.write();
    let task = key.task_id();
    let versions = inner.versions_by_task.get(&task)
        .ok_or_else(|| EfeError::InvalidConfig(
            "unknown tenant/task".into()))?;
    if versions.len() == 1 && !force {
        return Err(EfeError::InvalidConfig(
            "delete: cannot remove last version without force=true".into()));
    }
    let removed = inner.by_key.remove(key);
    if removed.is_none() {
        return Err(EfeError::InvalidConfig("unknown cone key".into()));
    }
    let vs = inner.versions_by_task.get_mut(&task).unwrap(); // guaranteed
    vs.retain(|&v| v != key.version);
    if vs.is_empty() {
        inner.versions_by_task.remove(&task);
    }
    drop(inner);
    self.emit(RegistryEvent::Deleted { key: key.clone() });
    Ok(())
}
```

### 10.7 Flux `report_breach`

```rust
async fn report_breach(&self, report: BreachReport) -> Result<()> {
    // Ne bloque pas sur le cone — un breach peut référencer un cône
    // déjà supprimé.
    let mut log = self.breach_log.lock();
    log.push(report.clone());
    drop(log);
    self.emit(RegistryEvent::BreachReported { report });
    Ok(())
}
```

### 10.8 Breach log (ring buffer)

```rust
pub struct BreachLog {
    capacity: usize,
    buffer: std::collections::VecDeque<BreachReport>,
}

impl BreachLog {
    pub fn new(capacity: usize) -> Self { ... }
    pub fn push(&mut self, r: BreachReport) {
        if self.buffer.len() == self.capacity {
            self.buffer.pop_front();
        }
        self.buffer.push_back(r);
    }
    pub fn recent(&self, n: usize) -> Vec<BreachReport> {
        self.buffer.iter().rev().take(n).cloned().collect()
    }
    pub fn filter<'a>(&'a self, f: &'a BreachFilter)
        -> impl Iterator<Item = &'a BreachReport> + 'a
    { ... }
}
```

**Complexité** :
- `push` : amortized O(1).
- `recent(n)` : O(n).
- `filter` : O(capacity) au pire.

### 10.9 Consistance mémoire

Tous les champs sont initialisés à la création (`new`). Aucune lazy-init.
Aucun `unsafe`. Aucun thread local. Safe pour `Arc<InMemoryRegistry>`
partagé entre workers tokio.

### 10.10 Taille mémoire approchée (N cônes, K=16, D=2048)

- Un `CanonicalCone` ≈ `16 × 2048 × 4 bytes` (anchors) + métadonnées ≈ 130 KB.
- `Arc` + HashMap bucket ≈ 200 bytes overhead.
- Pour 256 cônes × 4 tenants = 1024 cônes ≈ **130 MB RAM**. Acceptable MVP.

Au-delà, activer `FileBackend` ou `PostgresBackend` (post-MVP).

---

## 11. Breach logging et audit trail

### 11.1 Taxonomie des breaches

| Type | Émetteur | Critique ? |
|---|---|---|
| Dissolution breach | M8 | Oui — requête échoue. |
| FusionRound breach (intra-round projection) | M5 soft-align (si activé) | Non — auto-corrigé. |
| Manual breach | Admin / audit script | Variable. |

### 11.2 Cycle de vie d'un breach

```
1. M8.project() retourne Err(AlignmentBreach { delta, max }).
2. Dissolution engine construit un BreachReport { cone_key, swarm_id,
   observed_delta=delta, cone_radius=max, timestamp, context=Dissolution }.
3. Dissolution engine appelle breach_reporter.report_breach(report).
4. Registry:
   a. log.push(report) sous mutex.
   b. emit(RegistryEvent::BreachReported).
5. M11 consomme l'event:
   a. counter!("efe_breaches_total", "tenant"=tenant, "task"=task).inc()
   b. histogram!("efe_breach_magnitude").observe(delta - max)
   c. log_structured("alignment_breach", {cone, swarm, delta, max}).
6. Alert Grafana si rate > threshold (1 % des requêtes sur 5 min).
```

### 11.3 Spécification `BreachFilter`

```rust
#[derive(Clone, Debug, Default)]
pub struct BreachFilter {
    pub tenant: Option<String>,
    pub task: Option<String>,
    pub swarm_id: Option<SwarmId>,
    pub since_nanos: Option<u64>,
    pub until_nanos: Option<u64>,
    pub min_magnitude: Option<f32>,
}

impl BreachFilter {
    fn matches(&self, r: &BreachReport) -> bool { ... }
}
```

### 11.4 Rotation et rétention

- MVP : in-memory, rotation FIFO à `breach_log_capacity` (défaut 1024).
- Post-MVP : export vers log structuré (JSON lines) avec rotation horaire,
  retention 30 jours.

### 11.5 Confidentialité

Un `BreachReport` contient `swarm_id` (UUID) mais **pas** le prompt ni le
centroïde. Aucun PII. Peut être partagé librement entre équipes.

---

## 12. Événements et télémétrie

### 12.1 Channel d'events

```rust
pub fn events_channel(&self)
    -> tokio::sync::mpsc::UnboundedReceiver<RegistryEvent>;
```

Un seul consommateur au MVP (M11). Pattern `unbounded` OK car les events
sont rares (~10/s max). Overflow impossible en pratique.

### 12.2 Mapping event → télémétrie M11

| RegistryEvent | Métrique Prometheus | Span OpenTelemetry | Log JSON |
|---|---|---|---|
| `Registered` | `efe_cone_registered_total{tenant,task}` | `alignment.register` | `{"event":"registered", ...}` |
| `Evolved` | `efe_cone_evolved_total{op_kind}` | `alignment.evolve` | `{"event":"evolved", ...}` |
| `Deleted` | `efe_cone_deleted_total` | — | `{"event":"deleted", ...}` |
| `BreachReported` | `efe_breaches_total{tenant,task}`, `efe_breach_magnitude` (histogram) | `alignment.breach` | `{"event":"breach", ...}` + alert |
| `PolicyRejected` | `efe_policy_rejected_total{reason}` | — | WARN log |

### 12.3 Dashboards Grafana MVP

1. **Cone catalog** : nombre de cônes par tenant, distribution des versions.
2. **Breach rate** : breaches/minute, split par tenant, top offenders.
3. **Policy rejections** : rejections/minute, split par reason (I1..I7).
4. **Evolution activity** : evolve/s, split par op kind.

### 12.4 Alertes

- `breach_rate > 1 %` sur 5 min (P1)
- `policy_rejection_rate > 10/min` sur 10 min (P2 — possible attaque)
- `evolve_latency_p99 > 100ms` sur 5 min (P2 — backend dégradé)

---

## 13. Intégration non-breaking — SwarmSpecBuilder::from_registry

### 13.1 Extension du builder M1

On ajoute une méthode au `SwarmSpecBuilder` (dans `alignment/builder.rs` via
`impl` inhérent **séparé** — le builder lui-même reste dans `types.rs`).

```rust
// alignment/builder.rs
use crate::types::SwarmSpecBuilder;
use crate::alignment::{AlignmentRegistry, ConeKey};

impl SwarmSpecBuilder {
    /// Résout un cône depuis un registre et l'injecte dans le builder.
    ///
    /// Équivalent à :
    /// ```ignore
    /// let handle = registry.get(&key).await?;
    /// builder.alignment_cone(handle.as_cone().clone())
    /// ```
    pub async fn from_registry(
        self,
        registry: &(dyn AlignmentRegistry + '_),
        key: &ConeKey,
    ) -> Result<Self> {
        let handle = registry.get(key).await?;
        Ok(self.alignment_cone(handle.as_cone().clone()))
    }
}
```

### 13.2 Pourquoi non-breaking

- **Aucun champ nouveau** dans `SwarmSpec` ou `SwarmSpecBuilder`.
- **Méthode d'extension** : existe uniquement si `alignment` compilé.
- **Appel opt-in** : le caller décide d'utiliser `from_registry` ou l'ancien
  `alignment_cone(cone)`.
- **Serialization inchangée** : `SwarmSpec` se sérialise comme avant.
  Le registry n'est utilisé qu'au build.

### 13.3 Flux côté caller M10/M12

```rust
// Avant M9 :
let spec = SwarmSpec::builder()
    .problem_embedding(emb)
    .dim_latent(dim)
    .population(pop)
    .alignment_cone(my_cone_inline)
    .build()?;

// Avec M9 :
let key = ConeKey::parse("acme/chat-qa@v3")?;
let spec = SwarmSpec::builder()
    .problem_embedding(emb)
    .dim_latent(dim)
    .population(pop)
    .from_registry(&*registry, &key).await?
    .build()?;
```

### 13.4 Fallback et tolerance

Si le registre est indisponible (backend Postgres down post-MVP), le caller
peut retomber sur un cône inline :

```rust
let cone = match registry.get(&key).await {
    Ok(h) => h.as_cone().clone(),
    Err(_) => fallback_cone.clone(), // hardcoded safe default
};
```

**Pattern explicit** — pas de fallback silencieux automatique.

### 13.5 Tests d'intégration builder

- `builder_from_registry_happy_path`
- `builder_from_registry_unknown_key_errors`
- `builder_from_registry_then_build_produces_same_spec_as_inline`
- `builder_from_registry_does_not_clone_policy_check` (perf)
- `existing_m3_tests_pass_without_registry` (non-regression)

---

## 14. Backends de persistence (post-MVP)

### 14.1 `FileBackend` (P1, Sprint 13)

JSON lines ou TOML par cône. Simple, auditable, versionable en git.

```rust
pub struct FileBackend {
    root: PathBuf, // ex: /var/lib/efe/cones
    cfg: RegistryConfig,
    policy: PolicyEngine,
    cache: parking_lot::RwLock<HashMap<ConeKey, Arc<CanonicalCone>>>,
}
```

### 14.2 `PostgresBackend` (P1, Sprint 14)

Schema :

```sql
CREATE TABLE cones (
    tenant      TEXT NOT NULL,
    task        TEXT NOT NULL,
    version     INT  NOT NULL,
    canonical   JSONB NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (tenant, task, version)
);

CREATE TABLE breach_log (
    id          BIGSERIAL PRIMARY KEY,
    tenant      TEXT NOT NULL,
    task        TEXT NOT NULL,
    version     INT  NOT NULL,
    swarm_id    UUID NOT NULL,
    delta       REAL NOT NULL,
    radius      REAL NOT NULL,
    reported_at TIMESTAMPTZ NOT NULL,
    context     JSONB NOT NULL
);
```

Lecture pg avec cache LRU ; écritures dans une transaction.

### 14.3 Contrat stable pour swap

Les trois backends implémentent le même trait `AlignmentRegistry`. Le
contrôleur n'a aucune dépendance au backend concret — choix via config :

```toml
[alignment.registry]
backend = "in_memory" | "file" | "postgres"
```

---

## 15. Modèle de sécurité et threat model

### 15.1 Threats MVP

| # | Menace | Vecteur | Mitigation |
|---|---|---|---|
| T1 | Cône malveillant | Client pousse un cône avec anchors dégénérées (cos ≈ 1) pour rendre la projection inutile. | PolicyEngine I5 rejette cos ≥ 0.99. |
| T2 | DoS par volume | Client spam `register` en créant 10^6 versions. | `max_versions_per_task = 16` ; erreur dure au-delà. |
| T3 | DoS par taille | Client pousse un cône de 10^6 anchors. | PolicyEngine I1 cap à `max_anchors = 32`. |
| T4 | Exfiltration cross-tenant | Tenant A lit le cône de tenant B. | MVP : single-tenant, isolation par process. P1 : RBAC. |
| T5 | Tampering cône en transit | MitM modifie le cône entre client et registre. | MVP : TLS au niveau transport (M10). P2 : signature crypto. |
| T6 | Replay de breach fabriqué | Un attaquant injecte des `report_breach` faux pour corrompre métriques. | MVP : `report_breach` authentifié via JWT M10 (scope `breach:write`). |
| T7 | Overflow numérique dans policy | Anchors immenses (1e38) font exploser les carrés dans norm computation. | `f32::is_finite` vérifié dans I7 ; carrés saturés à +Inf captés par I7. |

### 15.2 Non-threats explicitement hors scope MVP

- **Confidentialité des cônes** : le cône n'est **pas** un secret (c'est une
  politique publique d'alignement). Pas de chiffrement au repos requis.
- **Audit bit-exact** post-rotation de clé : non requis MVP.
- **Non-repudiation crypto** : non requis MVP.

### 15.3 Zone de confiance

```
┌─ zone de confiance ─────────────────────────┐
│                                              │
│  M1 Controller  ←→  M9 Registry              │
│                                              │
│  (processus unique, socket local OK)         │
└──────────────────────────────────────────────┘
            ↑
            │ TLS + JWT (M10)
            │
      ┌─────────────────┐
      │  Client externe │
      └─────────────────┘
```

MVP : `M9` s'exécute dans le **même processus** que `M1 Controller`. Pas
de RPC inter-process. Zone de confiance maximale.

---

## 16. Budget de performance

### 16.1 Cibles

| Opération | Budget p99 | Condition |
|---|---|---|
| `get(&key)` | **5 µs** | InMemoryRegistry, cone en cache HashMap. |
| `register(key, cone)` (K=16, D=2048) | **5 ms** | Policy check O(K²·D) dominant. |
| `evolve` avec `AppendAnchor` | **5 ms** | Idem register. |
| `evolve` avec `UnionWith` (K₁+K₂=32) | **10 ms** | Union + policy re-check. |
| `report_breach` | **10 µs** | Lock mutex + push ring + emit (best-effort). |
| `list_versions(tenant, task)` | **10 µs** | Clone Vec<u32>. |
| `breach_history(100, filter)` | **100 µs** | Scan ring buffer cap 1024. |

### 16.2 Calculs

- `get` : HashMap lookup en `parking_lot::RwLock::read()` ≈ 200-500 ns typique,
  + Arc clone ≈ 50 ns, + message build ≈ 100 ns ⇒ ~1 µs p50, 5 µs p99 avec
  contention.
- `register` : PolicyEngine::check O(K²·D) : 16²·2048 = 524k FMAs ≈ 300 µs
  en scalaire, 50 µs AVX2 ; + HashMap insert ≈ 5 µs ⇒ budget 5 ms largement
  tenu.

### 16.3 Benchmarks criterion à livrer

```rust
#[bench] fn bench_policy_check_small_cone();    // K=4, D=128
#[bench] fn bench_policy_check_max_cone();      // K=32, D=4096
#[bench] fn bench_registry_get_hit();
#[bench] fn bench_registry_register();
#[bench] fn bench_registry_evolve_append();
#[bench] fn bench_union_cones_k16_d2048();
#[bench] fn bench_breach_log_push();
```

### 16.4 Règles anti-régression

- `bench_registry_get_hit` ne doit **jamais** dépasser 2 µs médiane sur
  CI runner standard. Régression > 20 % bloque le merge.

---

## 17. Interface publique — re-exports

### 17.1 `alignment/mod.rs`

```rust
//! Alignment cone governance (M9).
//!
//! See `M9_ARCHITECTURE.md` for design rationale.
//!
//! # Public surface
//!
//! ```
//! use efe_core::alignment::{
//!     AlignmentRegistry, InMemoryRegistry, ConeKey, ConeHandle,
//!     CanonicalCone, ConeMetadata, PolicyConfig, RegistryConfig,
//!     PolicyEngine, ConeEvolutionOp, BreachReport, BreachContext,
//!     BreachFilter, RegistryEvent,
//!     union_cones, blend_cones, intersect_cones,
//! };
//! ```

pub mod breach;
pub mod builder;
pub mod canonical;
pub mod compose;
pub mod events;
pub mod evolution;
pub mod key;
pub mod policy;
pub mod registry;
pub mod validator;

pub use breach::{BreachContext, BreachFilter, BreachLog, BreachReport};
pub use canonical::{CanonicalCone, ConeHandle, ConeMetadata};
pub use compose::{blend_cones, intersect_cones, union_cones};
pub use events::RegistryEvent;
pub use evolution::ConeEvolutionOp;
pub use key::ConeKey;
pub use policy::{PolicyConfig, PolicyEngine, RegistryConfig};
pub use registry::{AlignmentRegistry, BreachReporter, InMemoryRegistry};
pub use validator::validate_cone;
```

### 17.2 `lib.rs` — re-exports croisés

Au niveau crate root, on ajoute :

```rust
pub use alignment::{
    AlignmentRegistry, ConeKey, ConeHandle, CanonicalCone,
    PolicyEngine, RegistryConfig,
};
```

pour faciliter l'usage depuis `efe-server` et `efe-ffi` sans deep path.

---

## 18. Stratégie de test

### 18.1 Pyramide

```
      ┌───────────────┐   Acceptance : e2e incluant M8 + M10 (post-MVP)
      │               │
      │ Property +    │   Property-based (proptest, ~15 propriétés)
      │ Fuzz          │
      │               │   Fuzz (cargo-fuzz, 3 targets)
      ├───────────────┤
      │ Integration   │   tests/alignment_e2e.rs : scénarios bout-en-bout
      │               │
      ├───────────────┤
      │    Unit       │   Un test par fonction publique, ~80 tests
      │               │
      └───────────────┘
```

### 18.2 Unit tests — découpage par fichier

| Fichier | Nb tests | Exemples |
|---|---|---|
| `key.rs` | 10 | parse, display, validation chars, version_zero_rejected |
| `canonical.rs` | 8 | constructors, accessors, metadata serde |
| `validator.rs` | 5 | pass-through de PolicyEngine |
| `policy.rs` | 18 | voir §7.6 |
| `compose.rs` | 15 | voir §8.5 |
| `evolution.rs` | 17 | voir §9.4 |
| `breach.rs` | 8 | ring buffer FIFO, filter, serde |
| `events.rs` | 4 | serde roundtrip, display |
| `registry.rs` | 15 | lifecycle complet InMemoryRegistry |
| `builder.rs` | 5 | voir §13.5 |

**Total unit : ~105 tests**.

### 18.3 Integration tests (`tests/alignment_e2e.rs`)

1. **`register_get_roundtrip`** : register → get → compare bit-exact.
2. **`evolve_chain`** : register v1 → evolve append → evolve replace →
   evolve update_radius → list_versions retourne 4 versions triées.
3. **`cross_tenant_isolation`** : `acme/task@v1` et `other/task@v1` coexistent
   sans conflit.
4. **`breach_flow`** : register + simuler 10 breaches → `breach_history(10)`
   les retourne dans l'ordre inverse.
5. **`policy_violation_propagates`** : register avec K > max_anchors → erreur
   claire.
6. **`builder_from_registry_round_trip`** : résoudre via builder = identique
   à inline.
7. **`evolve_union_with_missing_other`** : erreur.
8. **`delete_last_without_force_rejected`**.
9. **`delete_with_force_removes_task`**.
10. **`event_channel_receives_registered_evolved`**.

### 18.4 Property-based tests (`proptest`)

- `prop_cone_key_roundtrip` : toute `ConeKey` valide sérialise/déserialise
  en identique.
- `prop_parse_display_roundtrip` : `parse(key.to_string()) == Ok(key)`.
- `prop_policy_accepts_cone_within_bounds` : cônes générés dans bornes
  acceptés.
- `prop_policy_rejects_cone_beyond_max_anchors`.
- `prop_union_commutative_up_to_order`.
- `prop_blend_t_endpoints_return_input_cones`.
- `prop_evolve_then_list_versions_monotone`.
- `prop_breach_log_bounded_by_capacity`.
- `prop_register_evolve_get_trilogy` : pour tout `(k, cone, op)`, le lookup
  final donne le résultat attendu.

### 18.5 Concurrence tests

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_register_and_get_stress() {
    let registry = Arc::new(InMemoryRegistry::new(RegistryConfig::default()));
    let mut handles = vec![];
    for t in 0..8 {
        let reg = registry.clone();
        handles.push(tokio::spawn(async move {
            for i in 0..100 {
                let key = ConeKey::new("tenant", &format!("task{t}"),
                                       i + 1).unwrap();
                let cone = make_test_cone(16, 128);
                let canonical = CanonicalCone::new(cone, "bot", "auto");
                reg.register(key.clone(), canonical).await.unwrap();
                reg.get(&key).await.unwrap();
            }
        }));
    }
    for h in handles { h.await.unwrap(); }
    // Verify total count.
    for t in 0..8 {
        let vs = registry.list_versions("tenant", &format!("task{t}"))
            .await.unwrap();
        assert_eq!(vs.len(), 100);
    }
}
```

Additional concurrency tests :
- `concurrent_evolve_on_same_task` (test serialization cohérence).
- `concurrent_report_breach_stress` (log ring buffer).
- `concurrent_read_during_write` (pas de deadlock, pas de lecture stale).

### 18.6 Fuzz targets (cargo-fuzz)

```rust
// fuzz/fuzz_targets/alignment_parse_key.rs
fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = ConeKey::parse(s);
    }
});

// fuzz/fuzz_targets/alignment_policy.rs
fuzz_target!(|data: (Vec<Vec<f32>>, f32)| {
    if let Ok(cone) = AlignmentCone::new(data.0, data.1) {
        let engine = PolicyEngine::new(PolicyConfig::default());
        let _ = engine.check(&cone);
    }
});

// fuzz/fuzz_targets/alignment_compose.rs
fuzz_target!(|(a, b): (AlignmentCone, AlignmentCone)| {
    let _ = union_cones(&a, &b);
    let _ = blend_cones(&a, &b, 0.5);
    let _ = intersect_cones(&a, &b, 0.5);
});
```

Run nightly CI : 1h par target. Coverage cible : > 85 %.

### 18.7 Miri

Exécuté sur tous les tests unitaires de `alignment/` pour détecter UB.
Zéro `unsafe` dans M9 ⇒ miri attendu 100 % propre.

### 18.8 Coverage cible

- **Ligne** : ≥ 90 % sur `alignment/`.
- **Branche** : ≥ 85 %.
- Rejet tout PR qui fait descendre la couverture sur les fichiers touchés.

---

## 19. Dépendances inter-modules

### 19.1 Dépendances sortantes

| Depuis | Vers | Nature |
|---|---|---|
| M9 | M1 (`types::AlignmentCone`) | Réutilisation du type existant. |
| M9 | M1 (`errors::EfeError`) | Propagation des erreurs. |
| M9 | M1 (`types::SwarmSpecBuilder`) | Extension via `from_registry`. |
| M9 | M4 (views read-only) | Lecture pendant `report_breach` pour diag. |
| M9 | M11 (TelemetryCollector) | Emission d'events. |

### 19.2 Dépendances entrantes

| Depuis | Consomme | Surface |
|---|---|---|
| M8 | `BreachReporter` trait | Une seule méthode (`report_breach`). |
| M3 | Indirect via `SwarmSpecBuilder::from_registry` | Opt-in. |
| M10 | `AlignmentRegistry` pour admin API | Tous les endpoints. |
| M12 | `AlignmentRegistry` pour FFI | Tous les endpoints. |

### 19.3 Anti-dépendances

**M9 ne dépend PAS de** : M3, M5, M6, M7, M8, M10, M12. Cette indépendance
est testée par compilation isolée via feature-flag `--no-default-features`
(post-MVP).

---

## 20. Plan de sprint M9

### 20.1 Sprint 8 (MVP registry + policy)

**Durée cible** : 2 semaines. Objectif : un registre en mémoire fonctionnel,
policy enforced, breach logging, events basiques.

| Jour | Livrable |
|---|---|
| J1-J2 | `key.rs`, `canonical.rs` + tests unit |
| J3-J4 | `policy.rs` + `validator.rs` + 18 tests PolicyEngine |
| J5-J6 | `breach.rs` + `events.rs` + tests ring buffer |
| J7-J8 | `registry.rs` — InMemoryRegistry register/get/delete |
| J9 | `registry.rs` — breach history + events channel |
| J10 | Integration test `register_get_roundtrip`, `breach_flow` |

**Gate sortie S8** : 45 tests passent, clippy pedantic zéro warning,
`cargo doc` sans warning, CI verte.

### 20.2 Sprint 9 (evolution + composition + builder)

**Durée cible** : 2 semaines. Objectif : ops d'évolution, composition,
intégration builder opt-in.

| Jour | Livrable |
|---|---|
| J1-J2 | `evolution.rs` — enum + apply_* helpers + 17 tests |
| J3 | `registry.rs::evolve` implémentation |
| J4-J5 | `compose.rs` — union/blend/intersect + 15 tests |
| J6 | `builder.rs` — from_registry extension + 5 tests |
| J7-J8 | Integration tests `evolve_chain`, `builder_from_registry` |
| J9 | Property-based tests (proptest) |
| J10 | Benchmarks criterion + baseline |

**Gate sortie S9** : 100+ tests, benchmarks établis, docs jour.

### 20.3 Sprint 10 (hardening + governance)

**Durée cible** : 2 semaines. Objectif : fuzz, concurrence, dashboards M11.

| Jour | Livrable |
|---|---|
| J1-J2 | Fuzz targets (3) + CI nightly |
| J3-J4 | Tests concurrence tokio multi-thread |
| J5 | Dashboards Grafana : catalog, breach rate, evolution |
| J6-J7 | Alerting rules Prometheus |
| J8 | ADR-009 (registry governance) |
| J9 | Hardening policy : audit log format, PII review |
| J10 | Gate release candidate |

### 20.4 Jalons

- **J-M9.1 (fin S8)** : Registry MVP fonctionnel en dev.
- **J-M9.2 (fin S9)** : Integration M3/M8 passe, builder opt-in OK.
- **J-M9.3 (fin S10)** : Prod-ready — fuzz clean, dashboards up, runbook publié.

---

## 21. Plan de migration depuis l'existant

### 21.1 État avant M9

Code actuel (résumé) :
- `types::AlignmentCone` : struct simple avec `anchors` + `max_projection_distance`.
- Chaque `SwarmSpec` en contient un **inline** via builder `.alignment_cone(c)`.
- `dissolution::alignment::StrictAlignmentProjector` l'utilise en lecture.
- Aucun stockage persistant.

### 21.2 Stratégie : additive, zéro suppression

Aucun champ n'est supprimé ou renommé. Aucune signature publique existante
ne change. M9 est additif :

1. Le stub `alignment/mod.rs` devient un vrai module.
2. Le trait `AlignmentRegistry` et les types associés sont **nouveaux**.
3. `SwarmSpecBuilder::from_registry` est ajouté ; `alignment_cone` existe
   toujours.

### 21.3 Migration progressive pour les callers

**Phase A (Sprint 8)** : déploiement de M9, zéro client ne l'utilise.
Toutes les requêtes continuent avec cône inline. CI continue de passer.

**Phase B (Sprint 9)** : un tenant pilote (`acme`) migre :
```rust
// Dans le service:
let registry = Arc::new(InMemoryRegistry::new(cfg));
registry.register(acme_key, acme_cone_canonical).await?;
// ...
let spec = SwarmSpec::builder()
    .from_registry(&*registry, &acme_key).await?
    .build()?;
```

**Phase C (Sprint 10+)** : autres tenants migrent. Eventually, le code
client ne construit plus de cône inline — il en pull tous depuis le registre.

### 21.4 Feature flag de transition

Pour permettre rollback en prod :

```toml
[features]
default = []
m9_registry = ["async-trait", "tokio/sync"]
```

Si `m9_registry` off, le module `alignment` expose uniquement le stub
Sprint 0 (docs seulement). Pour le MVP, **`m9_registry` est activée par
défaut**.

### 21.5 Communication équipe

- ADR-009 publié.
- Runbook : "Créer un cône via `efectl` CLI".
- Training session 1h pour l'équipe plateforme.
- Dashboards Grafana pré-configurés.

---

## 22. Matrice de risques et mitigations

| # | Risque | Proba | Impact | Mitigation |
|---|---|---|---|---|
| R1 | Over-engineering du registre (YAGNI) | Moyenne | Moyen | Scope strict S8/S9 ; revue d'architecture à fin S8. |
| R2 | Complexité async dans `AlignmentRegistry` freine dev | Faible | Moyen | `async_trait` adopté pattern-wide ; tests tokio dédiés. |
| R3 | Breach du contrat "jamais de mutation" | Faible | Élevé | Lint personnalisé : toute fn `pub async` ne prend pas `&mut self`. |
| R4 | Deadlock RwLock × Mutex | Faible | Élevé | Ordre d'acquisition documenté (jamais les deux simultanément). Tests concurrence. |
| R5 | Event channel bourré (unbounded → OOM) | Moyenne | Moyen | Best-effort send ; log WARN si channel absent. Monitoring M11. |
| R6 | Policy trop stricte bloque cas légitimes | Moyenne | Moyen | Chaque invariant a un test de tolérance ; tests de non-régression. |
| R7 | Policy trop laxe laisse passer cône dégénéré | Faible | Élevé | 7 invariants couvrent tous les cas connus. Fuzzing nightly. |
| R8 | Registry single-node ne scale pas | Faible (MVP) | Élevé (post-MVP) | Trait permet swap vers Postgres. Benchmarks cibles 1k cônes. |
| R9 | `SwarmSpecBuilder::from_registry` async breaks sync callers | Faible | Moyen | Signature async choisie dès le début ; wrapper sync disponible. |
| R10 | Migration progressive mal documentée | Moyenne | Faible | Runbook + 2 training sessions + pair migration acme pilote. |
| R11 | Cône versioning bug : v3 écrasé par v3 | Faible | Élevé | Contrat strict : version monotone ; tests dédiés. |
| R12 | Breach log filling under attack | Moyenne | Moyen | Ring buffer cap + rate limit side (M10 `breach:write` throttled). |

### 22.1 Risques résiduels acceptés

- **Pas de signature crypto des cônes** : accepté car threat model MVP
  ne l'exige pas.
- **Pas de multi-région** : accepté ; un seul registre par cluster OK.
- **Breach log perte en cas de crash** : accepté car in-memory ; P1 persistera.

---

## 23. Critères d'acceptation

### 23.1 Fonctionnels

- **CA-F1** : Le registre accepte 10 000 `register`/`get` par seconde sur
  un cœur, latence p99 < 10 ms.
- **CA-F2** : Les 7 invariants PolicyEngine sont testés individuellement,
  taux de détection = 100 % sur fixtures + 100 % sur fuzz 10M cas.
- **CA-F3** : `union_cones`, `blend_cones`, `intersect_cones` sont **pures**
  (entrées intactes) vérifié par tests.
- **CA-F4** : `evolve` crée une nouvelle version ; `parent` metadata
  toujours renseignée.
- **CA-F5** : `SwarmSpecBuilder::from_registry` produit un `SwarmSpec`
  identique à celui obtenu via `alignment_cone(cone)` manuel.
- **CA-F6** : Breach log respecte la capacité (ring buffer FIFO).
- **CA-F7** : Tous les types sérialisent/désérialisent roundtrip via serde
  JSON.

### 23.2 Non-fonctionnels

- **CA-NF1** : Zéro `unsafe` dans `alignment/`.
- **CA-NF2** : Zéro warning `clippy::pedantic`.
- **CA-NF3** : Coverage ligne ≥ 90 %, branche ≥ 85 %.
- **CA-NF4** : Miri passe sur tous les tests unit `alignment/`.
- **CA-NF5** : Fuzzing 3 targets × 1h sans crash, sans timeout.
- **CA-NF6** : `cargo doc --no-deps` sans warning.
- **CA-NF7** : Benchmarks criterion publiés sur CI (baseline établie S9).
- **CA-NF8** : Zero regression sur les 761 tests pré-M9 (tests existants
  M1-M8 passent sans modif).

### 23.3 Documentation

- **CA-D1** : Chaque item public documenté.
- **CA-D2** : ADR-009 "Alignment Registry Design" publié.
- **CA-D3** : Runbook "Managing cones with efectl" publié.
- **CA-D4** : Dashboards Grafana en repo `ops/grafana/alignment.json`.
- **CA-D5** : Migration playbook `docs/migration/m9.md`.

### 23.4 Intégration

- **CA-I1** : `tests/alignment_e2e.rs` passe (10 scénarios).
- **CA-I2** : Aucun test M1-M8 ne casse.
- **CA-I3** : CI `cargo test --all-features` < 5 min.

---

## 24. Annexes

### 24.1 Annexe A — Exemples d'usage

#### A.1 Enregistrement d'un cône depuis un service

```rust
use efe_core::alignment::{
    AlignmentRegistry, InMemoryRegistry, ConeKey,
    CanonicalCone, ConeMetadata, RegistryConfig,
};
use efe_core::types::AlignmentCone;

let registry = std::sync::Arc::new(
    InMemoryRegistry::new(RegistryConfig::default())
);

let cone = AlignmentCone::new(
    vec![vec![1.0, 0.0, 0.0], vec![0.0, 1.0, 0.0]],
    0.5,
)?;
let canonical = CanonicalCone::new(
    cone,
    "alice@acme.com",
    "chat-qa anchor set for product support",
);
let key = ConeKey::new("acme", "chat-qa", 1)?;

let handle = registry.register(key.clone(), canonical).await?;
println!("Registered {} (K={}, D={})",
         handle.key, handle.cone.num_anchors(), handle.cone.dim());
```

#### A.2 Résolution via builder

```rust
use efe_core::types::{SwarmSpec, LatentDim, PopulationMix, TemplateCode};

let spec = SwarmSpec::builder()
    .problem_embedding(vec![0.0; 3])
    .dim_latent(LatentDim::new(3)?)
    .population(PopulationMix::uniform(TemplateCode::Sem, 4)?)
    .max_rounds(4)?
    .from_registry(&*registry, &key).await?
    .build()?;
```

#### A.3 Évolution : ajouter un anchor

```rust
use efe_core::alignment::ConeEvolutionOp;

let new_handle = registry.evolve(
    key.clone(),
    ConeEvolutionOp::AppendAnchor {
        anchor: vec![0.0, 0.0, 1.0],
    },
    None,
).await?;
// new_handle.key == acme/chat-qa@v2
// new_handle.cone.metadata.parent == Some(acme/chat-qa@v1)
```

#### A.4 Reporting d'un breach (appelé par M8)

```rust
use efe_core::alignment::{BreachReport, BreachContext};
use efe_core::types::SwarmId;

let report = BreachReport {
    cone_key: key.clone(),
    swarm_id: SwarmId::new(),
    observed_delta: 0.72,
    cone_radius: 0.5,
    timestamp_nanos: now_nanos(),
    context: BreachContext::Dissolution,
};
registry.report_breach(report).await?;
```

### 24.2 Annexe B — Glossaire

| Terme | Définition |
|---|---|
| **Cone** | Région admissible dans l'ELS pour la projection finale, `(anchors, radius)`. |
| **Anchor** | Vecteur prior qui définit un sommet de l'enveloppe convexe du cone. |
| **ConeKey** | Identifiant stable `tenant/task@vN`. |
| **Canonical cone** | Cone + métadonnées enregistré dans le registre. |
| **Handle** | Référence comptée + clé, retournée par `get`/`register`. |
| **Policy** | Ensemble de 7 invariants appliqués à tout cone avant insertion. |
| **Breach** | Dépassement du rayon à la projection. |
| **Evolution** | Transformation explicite produisant une nouvelle version. |
| **Composition** | Opérateur sur 2 cones produisant un nouveau cone. |
| **Registry** | Stockage thread-safe versionné des cônes. |

### 24.3 Annexe C — Config TOML complète

```toml
[alignment.registry]
backend = "in_memory"
max_versions_per_task = 16
breach_log_capacity = 1024

[alignment.policy]
max_anchors = 32
max_dim = 4096
min_radius = 0.01
max_radius = 10.0
min_anchor_norm = 0.1
anchor_cosine_dup_threshold = 0.99
forbid_k_gt_d = true
```

### 24.4 Annexe D — Erreurs typiques renvoyées

| Situation | `EfeError` | Message |
|---|---|---|
| Version non monotone | `InvalidConfig` | `"register: version 2 not > current max 5"` |
| Policy I1 violée | `InvalidConfig` | `"policy violation: I1: K exceeds max_anchors"` |
| Lookup clé absente | `InvalidConfig` | `"registry: unknown cone key acme/foo@v1"` |
| Évolution sur parent absent | `InvalidConfig` | `"evolve: parent acme/foo@v1 not found"` |
| Blend t hors [0,1] | `InvalidConfig` | `"blend_cones: t must be in [0, 1]"` |
| Suppression dernière version sans force | `InvalidConfig` | `"delete: cannot remove last version without force=true"` |

### 24.5 Annexe E — FAQ

**Q1. Peut-on utiliser M9 sans migrer tout le code existant ?**
Oui. M9 est strictement opt-in. `alignment_cone(cone)` continue de marcher.

**Q2. Deux tenants peuvent-ils partager un cône ?**
Non par design : `ConeKey` inclut `tenant`. Pour partager, copier explicitement
en `register` sous la nouvelle clé.

**Q3. Peut-on supprimer une version et "garder" les autres ?**
Oui, sauf la dernière (sauf `force=true`). Les versions antérieures restent.

**Q4. Que se passe-t-il si un swarm référence une version supprimée ?**
Le swarm a déjà une copie inline du cône dans son `SwarmSpec` (résolue au
build). La suppression du cône dans le registre n'affecte pas le swarm en
cours. Un nouveau swarm pointant vers la clé supprimée aura une erreur au
`from_registry`.

**Q5. Comment tester mon code qui dépend de `AlignmentRegistry` ?**
Implémenter un `MockRegistry` (exemple fourni dans `tests/common/mock.rs`),
ou utiliser `InMemoryRegistry` directement — il est thread-safe et rapide.

**Q6. Que fait le registre si PolicyConfig change à chaud ?**
MVP : la config est fixée à `InMemoryRegistry::new`. Les cônes déjà stockés
restent valides. Future : re-validation explicite via `registry.revalidate()`.

**Q7. Comment migrer d'un cône inline existant vers le registre ?**
```rust
let canonical = CanonicalCone::new(existing_inline_cone, "migration", "auto");
registry.register(ConeKey::new("t", "legacy", 1)?, canonical).await?;
```

**Q8. Performance d'un `get` sous forte contention ?**
`parking_lot::RwLock` permet N lecteurs simultanés sans exclusion. Latence
médiane mesurée < 1 µs sous 32 threads.

**Q9. Événements `RegistryEvent` perdus si le consommateur est lent ?**
Le channel est `unbounded`. Si le consommateur est absent (MVP
`events_channel()` non appelé), les events sont ignorés silencieusement
(best-effort). Pas de backpressure vers le registre.

**Q10. Que se passe-t-il pendant un `evolve` si un autre caller lit le parent ?**
Read locks sont compatibles. Le parent ne change jamais (immutabilité
par version). Aucun hazard.

### 24.6 Annexe F — Liste des fichiers à créer/modifier

**À créer** :
- `efe-core/src/alignment/key.rs`
- `efe-core/src/alignment/canonical.rs`
- `efe-core/src/alignment/policy.rs`
- `efe-core/src/alignment/validator.rs`
- `efe-core/src/alignment/compose.rs`
- `efe-core/src/alignment/evolution.rs`
- `efe-core/src/alignment/breach.rs`
- `efe-core/src/alignment/events.rs`
- `efe-core/src/alignment/registry.rs`
- `efe-core/src/alignment/builder.rs`
- `efe-core/tests/alignment_e2e.rs`
- `docs/docs/adr/ADR-009-alignment-registry.md`
- `docs/docs/migration/m9.md`
- `ops/grafana/alignment.json`

**À modifier** :
- `efe-core/src/alignment/mod.rs` (remplace stub par façade).
- `efe-core/src/lib.rs` (ajoute re-exports M9).
- `efe-core/Cargo.toml` (ajoute `async-trait`, vérifie `tokio`,
  `parking_lot` déjà présents).
- `Cargo.toml` workspace (pas de changement si async-trait déjà
  dans workspace.dependencies).

**Inchangés** :
- `types::AlignmentCone` — reste tel quel, réutilisé.
- `dissolution/alignment.rs` — reste tel quel.
- `types::SwarmSpec` / `types::SwarmSpecBuilder` — inchangés ;
  méthode d'extension ajoutée via `impl` inhérent séparé dans
  `alignment/builder.rs`.

### 24.7 Annexe G — Checklist de revue PR M9

**Avant merge de chaque PR touchant `alignment/`** :

- [ ] `cargo fmt --check` passe.
- [ ] `cargo clippy --all-targets -- -D warnings` passe (pedantic).
- [ ] `cargo test --all-features` passe.
- [ ] `cargo doc --no-deps` sans warning.
- [ ] Couverture ligne ≥ 90 % sur fichiers touchés (cargo-llvm-cov).
- [ ] Pas de nouveau `unsafe`.
- [ ] Pas de nouveau `unwrap`/`expect` hors tests.
- [ ] Nouveaux types `#[non_exhaustive]`.
- [ ] Nouveaux types `Serialize + Deserialize` si données persistables.
- [ ] Documentation ajoutée pour tout item public (`#[warn(missing_docs)]`).
- [ ] Tests unit ajoutés pour toute nouvelle fonction publique.
- [ ] Intégration test si comportement cross-file.
- [ ] ADR mise à jour si décision architecturale.
- [ ] CHANGELOG.md mis à jour.

---

**Fin du document M9_ARCHITECTURE.md — v2.0.**

Ce document est la **référence d'implémentation** du module M9. Toute dérive
significative (nouveau trait, nouvel invariant, nouveau backend) nécessite
un ADR + mise à jour du présent document. La spec est **gelée pour Sprint 8**
à la date de signature ci-dessous.

| Rôle | Nom | Date |
|---|---|---|
| Auteur | Core Runtime Team | 2026-04-19 |
| Reviewer | Safety Lead | _pending_ |
| Approbateur | Platform Director | _pending_ |
