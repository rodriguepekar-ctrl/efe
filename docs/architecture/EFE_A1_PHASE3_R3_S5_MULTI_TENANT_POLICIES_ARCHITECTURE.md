# EFE A1 — Phase 3 R3.S5 — Multi-tenant policies (fix L5/D-tenant)

> **Statut** : architecture (à implémenter)
> **Sprint** : Phase 3 R3.S5
> **Priorité roadmap** : 🟢 nice-to-have (10 h, ~10 ¢ live)
> **Pré-requis** : R3.S1 (cost µ¢), R3.S2 (SlmCircuitBreaker), R3.S6-R2 (scoring table per-request)
> **Fichiers cible** : `crates/efe-coordinator/src/multi_tenant.rs` (NEW),
> `coordinator.rs`, `request.rs`, `workflows/{mission_a,mission_b,speculative}.rs`,
> `lib.rs`, `tests/multi_tenant_policies.rs` (NEW)

---

## 0. Résumé exécutif

EFE est annoncé « multi-tenant par défaut » (`context.rs:22` :
« multi-tenancy par défaut dans EFE »). Or **l'isolation est aujourd'hui une
fiction** : les 3 workflows construisent leur `CallContext` avec
`coord.config.default_tenant` (un `Uuid::nil()` global) — cf.
`mission_a.rs:79`, `mission_b.rs:68`. Aucune requête ne porte de `tenant_id`,
et aucune politique n'est différenciée par client.

R3.S5 livre **la couche de politiques par tenant** :

1. **Identité** — propager un vrai `tenant_id` par requête (fin de la fiction
   `default_tenant`), tracé dans l'audit chain (`AuditEntry.tenant_id` existe
   déjà, `context.rs:82`).
2. **Politiques** — `TenantPolicies` (table de scoring, allow-list de LLM,
   plafond de coût µ¢, seuil de circuit) résolues via un `PolicyRegistry`.
3. **Enforcement** — appliquer ces politiques **à des points d'ancrage réels
   et déjà existants** du pipeline (override scoring, filtrage allow-list,
   resserrement de budget, isolation de circuit).

**Principe directeur (hérité de R3.S4)** : *zéro dead config*. Chaque champ de
`TenantPolicies` DOIT se brancher sur un point d'enforcement vérifiable. Le
champ `retry_policy` listé dans la roadmap est **explicitement descopé**
(§4.3) faute d'infrastructure de retry à paramétrer — l'inclure serait du
faux contrôle.

**Critère d'acceptation central** : *2 tenants distincts produisent des
routings/coûts/allow-lists différents sur la même requête, et l'audit chain
trace le bon `tenant_id` par mission.*

---

## 1. Scope / Non-scope

### 1.1 Dans le scope R3.S5

| # | Livrable | Point d'enforcement réel |
|---|---|---|
| 1 | `tenant_id` par requête (`Option<TenantId>`, défaut `nil`) | `CallContext` (audit) |
| 2 | `TenantPolicies` + `PolicyRegistry` (`src/multi_tenant.rs`) | nouveau module |
| 3 | `Coordinator.policies: Arc<PolicyRegistry>` + builder | `coordinator.rs` |
| 4 | Override **scoring table** per-tenant | `mission_a.rs:195-200` |
| 5 | Filtrage **allow-list LLM** per-tenant | `mission_a.rs:218`, `mission_b` fan-out |
| 6 | Plafond **coût µ¢/requête** per-tenant | `Budget` resserré → `router.rs:118` |
| 7 | **Seuil de circuit** per-tenant + **isolation** circuit | `slm_circuits` keyed `(tenant, robot)` |
| 8 | Propagation `tenant_id` dans l'audit (fin fiction `default_tenant`) | `mission_a.rs:79`, `mission_b.rs:68` |

### 1.2 Hors scope (déféré, justifié §4.3 / §13)

- **`retry_policy`** — aucune infra de retry/backoff à paramétrer (la
  « fallback chain S8 » itère `available_llms`, ce n'est pas un retry).
- **PII policy par tenant** — `PiiRedactor` est un closure global
  (`common.rs:6-16`) ; la rendre per-tenant est un sprint à part (touche les
  3 providers).
- **Rate-limiting / quota par tenant** — aucune infra (les `ClaudeRateLimits`
  sont observationnels, `anthropic.rs:36-49`, jamais enforce). Sprint dédié.
- **Persistance des policies** (Postgres/fichier) — R3.S5 livre le registre
  en mémoire ; le chargement depuis une source externe est un wiring gateway.
- **Data residency / region pinning** — relève de R3 routing contextuel (S13).

### 1.3 Invariant de non-régression

> Une requête **sans** `tenant_id` (ou avec `tenant_id = nil`) et un
> Coordinator **sans** `PolicyRegistry` doivent produire **exactement** le
> comportement R3.S4 actuel (byte-pour-byte sur les réponses, à `tenant_id`
> d'audit près). Backward-compat 100 %.

---

## 2. Problème (limitations attaquées)

### L5-a — Isolation tenant fictive

`run_mission_a` (`mission_a.rs:79`) :

```rust
let ctx = CallContext::with_swarm(coord.config.default_tenant, swarm_id, audit.clone());
```

`default_tenant` vaut `Uuid::nil()` par défaut (`coordinator.rs:139`) et n'est
**jamais** dérivé du caller. Conséquence : dans un déploiement SaaS, **toutes
les entrées d'audit de tous les clients portent le même `tenant_id`**. Le
champ `AuditEntry.tenant_id` (`context.rs:82`), pourtant prévu pour
« isolation audit », est inexploitable. C'est un risque de conformité (on ne
peut pas prouver qu'une donnée client A n'a pas fui vers B) et un trou de
traçabilité.

### L5-b — Table de scoring singleton

La `TaskClassScoringTable` (`scoring.rs:207`) est calibrée une fois
(`default_3llm_table`, `scoring.rs:236`). En SaaS, un client « qualité avant
tout » et un client « coût avant tout » devraient router différemment la même
`task_class`. Le mécanisme d'override per-request existe déjà
(`request.rs:65` + `mission_a.rs:195`) **mais rien ne le dérive d'un profil
client** : c'est au caller de fournir la table à chaque requête.

### L5-c — Aucune garde de coût / d'allow-list par client

Rien n'empêche un tenant d'utiliser un LLM premium hors de son plan, ni de
dépasser un plafond de coût mensuel implicite. Le `Budget` est **par requête**
(`router.rs:118`), jamais borné par un contrat client.

### L5-d — Blast radius de circuit inter-tenant

`slm_circuits` est keyed par **robot seul** (`coordinator.rs:50`,
`"analyzer"`). Si un tenant bruyant fait tomber le circuit `analyzer`, **tous
les tenants** perdent l'analyzer (skip → mode dégradé). Pas d'isolation de
panne.

---

## 3. État actuel du code (inventaire ancré)

| Élément | Emplacement | Constat |
|---|---|---|
| `TenantId = Uuid` | `efe-nano-app-coordinator/src/context.rs:23` | existe, alias UUID |
| `CallContext{swarm_id,tenant_id,audit}` | `context.rs:29-37` | porte le tenant |
| `AuditEntry.tenant_id` | `context.rs:82` | tracé par robot |
| `default_tenant: Uuid` | `coordinator.rs:131` (défaut `nil` :139) | global, jamais dérivé |
| `CallContext::with_swarm(default_tenant,…)` | `mission_a.rs:79`, `mission_b.rs:68` | **fiction** |
| `MissionARequest` (pas de `tenant_id`) | `request.rs:24-66` | à étendre |
| `MissionBRequest` (`llms`, pas de `budget`, pas de router) | `request.rs:172-201` | enforcement différent |
| `SpeculativeRequest` | `request.rs:286-343` | drafter/verifier |
| `task_class_scoring_table: Option<Arc<…>>` | `request.rs:65` | override existant |
| Override scoring → router | `mission_a.rs:195-200` | point d'ancrage #4 |
| `available_llms` → `RouterInput` | `mission_a.rs:218` | point d'ancrage #5 |
| Budget filter (µ¢) | `clients/router.rs:118` | point d'ancrage #6 |
| `slm_circuits: HashMap<String,SlmCircuitBreaker>` | `coordinator.rs:50` | keyed robot |
| `slm_circuit(robot)` lookup | `coordinator.rs:118-120` | à étendre tenant-aware |
| `Coordinator` (`#[derive(Clone)]`, `clients: Arc<…>`) | `coordinator.rs:147-163` | policies doit être `Arc` |
| `TaskClassScoringTable` (`Clone+Default+Serde`, `version`) | `scoring.rs:206-222` | per-tenant friendly |
| `PiiRedactor` (closure globale) | `llm/providers/common.rs:6-16` | hors scope |
| `RetryPolicy` | **inexistant** dans ces 2 crates | descopé §4.3 |

---

## 4. Décisions chirurgicales

### 4.1 Matrice politique × mission × point d'enforcement

C'est le cœur de la rigueur R3.S5 : **on n'ajoute un champ que s'il a un point
d'ancrage par mission.**

| Champ `TenantPolicies` | Mission A | Mission B | Speculative | Mécanisme |
|---|---|---|---|---|
| `scoring_table` | ✅ override router (`mission_a.rs:195`) | ⚪ N/A (pas de router) | ⚪ N/A (drafter/verifier fixés par la requête) | injecte `Arc<table>` si requête n'en fournit pas |
| `allowed_llms` | ✅ filtre `available_llms` | ✅ filtre `llms` (fan-out) | ✅ valide `drafter_llm`/`verifier_llm` | intersection / rejet |
| `max_cost_microcents_per_query` | ✅ resserre `budget` | ⚠️ post-hoc (Mission B n'a pas de `Budget`) | ✅ resserre `budget` | `min(req, policy)` |
| `slm_circuit_threshold` | ✅ circuit per-tenant | ✅ idem | ⚪ (drafter circuit R3.S4 séparé) | construction lazy |

Légende : ✅ enforce, ⚠️ enforce partiel (documenté), ⚪ non applicable.

**Conséquence design** : `allowed_llms` est le seul champ universel (3
missions). `scoring_table` est Mission-A-only (seule mission avec router). Le
plafond de coût est strict pour A/Spec (via `Budget`) et **best-effort** pour
B (qui n'a pas de budget — on log un dépassement, §7.2, sans bloquer faute de
point de coupe avant fan-out).

### 4.2 Sémantique de résolution (priorité)

Pour tout paramètre `P` :

```
P_effectif = override_requête(P)  si présent
           ⊓ politique_tenant(P)  si tenant a une policy
           ⊔ défaut_global(P)     sinon
```

Où `⊓` = « le plus restrictif » (cost cap : `min` ; allow-list :
intersection) et la requête prime sur le tenant **uniquement quand elle est
plus restrictive ou explicitement fournie pour un champ non-sécuritaire**
(scoring table). Pour les champs **sécuritaires** (`allowed_llms`,
`max_cost`), **la politique tenant est un plafond dur** : une requête ne peut
JAMAIS élargir au-delà de sa policy. C'est l'invariant de sécurité §11.

Règle explicite par champ :

| Champ | Si requête fournit | Si tenant a policy | Résultat |
|---|---|---|---|
| `scoring_table` | requête gagne (préférence non-sécuritaire) | sinon policy | sinon router global |
| `allowed_llms` | requête ∩ policy (jamais ∪) | policy borne | sinon `available_llms` brut |
| `max_cost_µ¢` | `min(req.budget, policy)` | policy borne | sinon `req.budget` |
| `circuit_threshold` | — (non exposé requête) | policy | sinon défaut R3.S2 (3) |

### 4.3 Descope de `retry_policy` (anti dead-config)

La roadmap (`…R3_PRODUCTION_HARDENING…md:383`) liste
`retry_policy: RetryPolicy`. **Constat** : `RetryPolicy` n'existe pas dans
`efe-coordinator` ni `efe-nano-app-coordinator` (grep : seulement dans
`efe-nano-sdk`, type sans rapport). Surtout, **aucun workflow n'effectue de
retry** : la « fallback chain S8 » (`mission_a.rs:391-398`) itère
`available_llms` dans l'ordre, sans backoff ni nombre d'essais configurable.

Ajouter `retry_policy` créerait un champ que **rien ne lit** — exactement le
type de faux contrôle rejeté en R3.S4 (énergie SLM honnête) et R3.S3 (bruit =
absence de contenu, pas longueur). **Décision : `retry_policy` est déféré** à
un sprint qui introduira d'abord une vraie boucle de retry. Note laissée dans
le code (`// R3.S5: retry_policy deferred — no retry infra to parametrize`).

### 4.4 Isolation de circuit : keying `(tenant, robot)`

Pour résoudre L5-d, les circuits deviennent keyed par
`circuit_key(tenant, robot)` = `format!("{tenant}:{robot}")` quand une policy
tenant est active, sinon par `robot` seul (compat R3.S2). Construction
**lazy** au premier accès tenant, à partir de `policy.slm_circuit_threshold`.
Cela donne une isolation réelle du blast radius **sans** dupliquer les
circuits pour les déploiements mono-tenant (clé = robot).

---

## 5. Modèle de données (`src/multi_tenant.rs`)

```rust
// SPDX-License-Identifier: UNLICENSED
//! R3.S5 — Politiques par tenant + registre.

use std::collections::HashMap;
use std::sync::Arc;

use efe_nano_app_coordinator::types::TaskClassScoringTable;
use efe_nano_app_coordinator::TenantId; // = Uuid

/// Politiques d'un tenant. Tous les champs ont un point d'enforcement réel
/// (cf. architecture §4.1). `retry_policy` est volontairement absent (§4.3).
#[derive(Debug, Clone)]
pub struct TenantPolicies {
    /// Identité (= clé dans le registre, dupliquée pour self-contained audit).
    pub tenant_id: TenantId,

    /// Table de scoring tenant-spécifique (override router Mission A).
    /// `None` ⇒ router global / table par-requête.
    pub scoring_table: Option<Arc<TaskClassScoringTable>>,

    /// Allow-list de provider IDs. `None` ⇒ aucun filtrage (tous permis).
    /// `Some(vec![])` ⇒ AUCUN LLM permis (deny-all explicite, fail-closed).
    pub allowed_llms: Option<Vec<String>>,

    /// Plafond dur de coût par requête en microcents. `None` ⇒ pas de plafond
    /// tenant (le `Budget` de la requête s'applique seul).
    pub max_cost_microcents_per_query: Option<u64>,

    /// Seuil d'échecs du circuit SLM (per-tenant). Défaut R3.S2 = 3.
    pub slm_circuit_threshold: u32,
}

impl TenantPolicies {
    /// Politique « ouverte » : aucun override, défauts globaux. Utile pour les
    /// tests et comme base de `..` struct-update.
    #[must_use]
    pub fn open(tenant_id: TenantId) -> Self {
        Self {
            tenant_id,
            scoring_table: None,
            allowed_llms: None,
            max_cost_microcents_per_query: None,
            slm_circuit_threshold: DEFAULT_SLM_CIRCUIT_THRESHOLD, // 3
        }
    }

    /// Résout l'allow-list effective : `requested ∩ allowed` (jamais ∪).
    /// `None` policy ⇒ renvoie `requested` inchangé.
    #[must_use]
    pub fn filter_llms(&self, requested: &[String]) -> Vec<String> { /* §7 */ }

    /// Plafond effectif : `min(req_budget_µ¢, policy_cap)` (le plus strict).
    #[must_use]
    pub fn cap_microcents(&self, req_budget_microcents: u64) -> u64 { /* §7 */ }

    /// `true` si `llm` est autorisé (allow-list `None` ⇒ tout autorisé).
    #[must_use]
    pub fn permits_llm(&self, llm: &str) -> bool { /* §7 */ }
}

/// Seuil de circuit par défaut (aligné R3.S2 `default_slm_circuits`).
pub const DEFAULT_SLM_CIRCUIT_THRESHOLD: u32 = 3;

/// Registre de politiques. Lookup O(1) par `TenantId`.
///
/// Immuable après construction (les politiques sont chargées au boot par le
/// gateway). Le `Coordinator` le détient en `Arc` pour rester `Clone` cheap.
#[derive(Debug, Default, Clone)]
pub struct PolicyRegistry {
    by_tenant: HashMap<TenantId, TenantPolicies>,
}

impl PolicyRegistry {
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Insère/écrase la policy d'un tenant. Builder-style.
    #[must_use]
    pub fn with(mut self, p: TenantPolicies) -> Self {
        self.by_tenant.insert(p.tenant_id, p);
        self
    }

    /// Lookup. `None` ⇒ tenant inconnu ⇒ comportement global (backward-compat).
    #[must_use]
    pub fn get(&self, tenant: &TenantId) -> Option<&TenantPolicies> {
        self.by_tenant.get(tenant)
    }

    #[must_use]
    pub fn len(&self) -> usize { self.by_tenant.len() }
    #[must_use]
    pub fn is_empty(&self) -> bool { self.by_tenant.is_empty() }
}
```

**Notes de conception**

- `allowed_llms: Option<Vec<String>>` à 3 états : `None` = pas de filtre,
  `Some([])` = **deny-all** (fail-closed explicite, important pour la
  sécurité : une policy vide ne doit pas « ouvrir » par accident).
- `scoring_table: Option<Arc<…>>` réutilise le type exact déjà attendu par
  `mission_a.rs:196` → branchement sans conversion.
- `PolicyRegistry` est `Default` + `Clone` → s'insère trivialement dans le
  builder existant et reste compatible `Coordinator: Clone`.

---

## 6. Wiring Coordinator + requêtes

### 6.1 `Coordinator` / `CoordinatorBuilder`

```rust
// coordinator.rs — Coordinator
pub struct Coordinator {
    // … existant …
    /// R3.S5 — politiques par tenant (vide par défaut ⇒ mono-tenant global).
    pub policies: Arc<PolicyRegistry>,
}
```

```rust
// CoordinatorBuilder
fn policies: Option<PolicyRegistry>   // champ builder

#[must_use]
pub fn policies(mut self, reg: PolicyRegistry) -> Self {
    self.policies = Some(reg);
    self
}

// build() :
policies: Arc::new(self.policies.unwrap_or_default()),
```

`Arc<PolicyRegistry>` : aligné sur `clients: Arc<CoordinatorClients>`,
préserve `#[derive(Clone)]` sans clone profond.

### 6.2 `tenant_id` sur les 3 requêtes

```rust
// request.rs — sur MissionARequest, MissionBRequest, SpeculativeRequest :
/// R3.S5 — tenant propriétaire. `None`/`nil` ⇒ `default_tenant` global
/// (backward-compat). Sert (1) à l'audit, (2) au lookup PolicyRegistry.
#[serde(default)]
pub tenant_id: Option<TenantId>,
```

`#[serde(default)]` ⇒ les payloads pré-S5 désérialisent à `None`
(backward-compat JSON garantie). Les helpers constructeurs existants
(`MissionARequest::with_default_scoring`, `with_drafter_and_verifier`)
initialisent `tenant_id: None`.

### 6.3 Fonction de résolution (cœur, testée isolément)

Un seul point de vérité, appelé en tête de chaque workflow :

```rust
// multi_tenant.rs
/// Résout le tenant effectif + sa policy.
/// - `req_tenant`: `req.tenant_id`
/// - `default_tenant`: `coord.config.default_tenant`
/// Retourne `(effective_tenant_id, Option<&TenantPolicies>)`.
pub fn resolve<'a>(
    registry: &'a PolicyRegistry,
    req_tenant: Option<TenantId>,
    default_tenant: TenantId,
) -> (TenantId, Option<&'a TenantPolicies>) {
    let tid = req_tenant.unwrap_or(default_tenant);
    (tid, registry.get(&tid))
}
```

Le `tenant_id` retourné **remplace** `coord.config.default_tenant` dans la
construction du `CallContext` (fin de la fiction L5-a).

---

## 7. Enforcement par mission

### 7.1 Mission A (`run_mission_a`)

Insertions (numéros = ordre d'exécution) :

```rust
// (0) en tête, après swarm_id :
let (tenant_id, policy) =
    multi_tenant::resolve(&coord.policies, req.tenant_id, coord.config.default_tenant);

// (1) audit — remplace default_tenant (fix L5-a) :
let ctx = CallContext::with_swarm(tenant_id, swarm_id, audit.clone());

// (2) allow-list — AVANT le router (point d'ancrage mission_a.rs:218) :
let effective_llms = match policy {
    Some(p) => p.filter_llms(&req.available_llms), // req ∩ allow
    None => req.available_llms.clone(),
};
if effective_llms.is_empty() {
    // deny-all ou intersection vide → fail-closed explicite.
    return Err(CoordError::Config(
        "no LLM permitted for tenant (allow-list)".into()));
}

// (3) scoring table — priorité requête > policy > global (mission_a.rs:195) :
let table_for_call: Option<Arc<TaskClassScoringTable>> =
    req.task_class_scoring_table.clone()
       .or_else(|| policy.and_then(|p| p.scoring_table.clone()));
let router_for_call = match table_for_call.as_ref() {
    Some(t) => Some(RouterClient::for_task_class(t.as_ref(), req.task_class)),
    None => coord.clients.router.as_deref().copied(),
};

// (4) budget cap — resserre AVANT RouterInput (router.rs:118 filtre dessus) :
let mut budget = req.budget.clone();
if let Some(p) = policy {
    let capped = p.cap_microcents(budget.max_cost_microcents_resolved());
    budget.max_cost_microcents = capped; // min(req, policy)
}
// … RouterInput { available_llms: effective_llms, user_budget: budget, … }

// (5) circuit — clé tenant-aware (fix L5-d) :
let circuit = coord.slm_circuit_for(tenant_id, "analyzer", policy);
```

Le reste du pipeline (analyzer/router/compressor/verifier) est inchangé : il
consomme `effective_llms`, `budget`, et les circuits tenant-aware.

### 7.2 Mission B (`run_mission_b`)

Mission B **n'a pas de router ni de `Budget`** (`request.rs:172-201`) → seul
`allowed_llms` s'applique strictement :

```rust
let (tenant_id, policy) = multi_tenant::resolve(…);
let ctx = CallContext::with_swarm(tenant_id, swarm_id, audit.clone());

let effective_llms = match policy {
    Some(p) => p.filter_llms(&req.llms),
    None => req.llms.clone(),
};
// Mission B exige ≥2 LLMs ; après filtrage on re-vérifie :
if effective_llms.len() < 2 {
    return Err(CoordError::Config(
        "tenant allow-list leaves < 2 LLMs for Mission B vote".into()));
}
// fan-out sur effective_llms (au lieu de req.llms)
```

**Plafond de coût Mission B (best-effort, ⚠️)** : Mission B somme le coût des
N LLMs *après* fan-out (`MissionBResponse.total_cost_microcents`,
`request.rs:247`). Il n'y a **pas** de point de coupe pré-fan-out. R3.S5
**ne bloque donc pas** Mission B sur le coût ; il **trace un dépassement** via
un nouveau champ `ChewingMetrics.tenant_cost_cap_exceeded: Option<bool>` (cf.
§9) pour alerting. Bloquer Mission B sur coût exigerait une estimation
pré-fan-out (router-like) — hors scope, noté §13.

### 7.3 Speculative (`run_speculative`)

```rust
let (tenant_id, policy) = multi_tenant::resolve(…);

// allow-list : drafter ET verifier doivent être permis (sinon fail-closed).
if let Some(p) = policy {
    if !p.permits_llm(&req.drafter_llm) || !p.permits_llm(&req.verifier_llm) {
        return Err(CoordError::Config(
            "tenant allow-list forbids drafter or verifier".into()));
    }
}
// budget cap identique à Mission A (Speculative a un Budget, request.rs:305) :
let mut budget = req.budget.clone();
if let Some(p) = policy {
    budget.max_cost_microcents = p.cap_microcents(budget.max_cost_microcents_resolved());
}
// CallContext tenant-aware via le bus swarm_id (Speculative trace déjà).
```

Le drafter-circuit R3.S4 (`with_drafter_circuit_opt`) reste keyed par l'ID du
drafter ; on le rend tenant-aware via la même `slm_circuit_for` (§8) quand une
policy existe.

---

## 8. Isolation de circuit tenant-aware

Nouveau helper sur `Coordinator` (ou `CoordinatorClients`) :

```rust
/// Clé de circuit : `"{tenant}:{robot}"` si une policy tenant existe,
/// sinon `"{robot}"` (compat R3.S2 mono-tenant).
fn circuit_key(tenant: TenantId, robot: &str, has_policy: bool) -> String {
    if has_policy { format!("{tenant}:{robot}") } else { robot.to_string() }
}
```

**Stratégie de construction** : le `slm_circuits: HashMap<String,…>` actuel
(`coordinator.rs:50`) est pré-rempli par robot. Pour le multi-tenant, deux
options :

- **Option A — clés statiques pré-déclarées** : le gateway insère
  `"{tenant}:analyzer"` etc. à la construction du Coordinator. Simple, pas de
  mutation runtime, mais exige de connaître les tenants au boot.
- **Option B — lazy/`DashMap`** : créer le circuit au premier accès
  `(tenant, robot)` depuis `policy.slm_circuit_threshold`. Flexible mais
  introduit une mutation concurrente (le champ deviendrait
  `Arc<DashMap<…>>`).

**Recommandation R3.S5** : **Option A** (statique). Raison : (1) cohérence
avec R3.S2 (circuits pré-déclarés, immuables) ; (2) `Coordinator: Clone` reste
trivial ; (3) le nombre de tenants actifs est connu au déploiement.
`slm_circuit_for(tenant, robot, policy)` fait le lookup
`slm_circuits.get(&circuit_key(...))` et **retombe sur la clé robot-seule** si
la clé tenant n'existe pas (dégradation propre vers la sémantique R3.S2).
Helper `CoordinatorClients::tenant_slm_circuits(tenants, threshold)` pour
pré-générer les clés `(tenant, robot)`.

> **Limitation honnête** : sans clé `(tenant, robot)` pré-déclarée, le seuil
> per-tenant n'a pas d'effet (le circuit partagé robot-seul s'applique). Le
> seuil per-tenant **n'isole** que si les circuits tenant ont été déclarés.
> C'est documenté et testé (§10, test `tenant_circuit_isolation`).

---

## 9. Métriques & audit

### 9.1 `ChewingMetrics` (extension)

```rust
/// R3.S5 — tenant effectif de la mission (audit + analytics).
#[serde(default)]
pub tenant_id: Option<TenantId>,

/// R3.S5 — `Some(true)` si le coût a dépassé le plafond tenant (Mission B
/// best-effort, §7.2). `None` si pas de plafond / pas applicable.
#[serde(default)]
pub tenant_cost_cap_exceeded: Option<bool>,

/// R3.S5 — `Some(n)` = nb de LLMs retirés par l'allow-list tenant (0 si
/// aucun). `None` si pas de policy. Sert à détecter une allow-list trop
/// stricte qui dégrade la qualité de routing.
#[serde(default)]
pub tenant_llms_filtered: Option<usize>,
```

Tous `#[serde(default)]` ⇒ backward-compat. **Tripwire** : les 2 sites de
construction littérale de `ChewingMetrics` (test serde `metrics.rs`, test I3
`events/stream.rs`) doivent être patchés (leçon R3.S4 : le compilateur enforce
les champs de struct littéraux mais pas les `vec!` de couverture).

### 9.2 Audit chain

`AuditEntry.tenant_id` (`context.rs:82`) est désormais peuplé avec le **vrai**
tenant (via le `CallContext` corrigé). Aucun changement de schéma : on remplit
enfin correctement un champ existant. L'`HmacAuditChain`
(`efe-coordinator/src/audit.rs`) chaîne les entrées inchangé.

---

## 10. Plan de tests (≥ 12 offline + live)

### 10.1 Unitaires `TenantPolicies` / `PolicyRegistry` (`src/multi_tenant.rs`)

1. `policies_open_has_no_overrides` — `open()` = tous `None`, seuil 3.
2. `filter_llms_intersection_not_union` — `allowed=[A,B]`, `req=[B,C]` → `[B]`.
3. `filter_llms_none_policy_passthrough` — `allowed=None` → `req` inchangé.
4. `filter_llms_deny_all_yields_empty` — `allowed=Some([])` → `[]` (fail-closed).
5. `cap_microcents_takes_min` — cap 500, req 2000 → 500 ; cap 5000, req 1000 → 1000.
6. `cap_microcents_none_policy_passthrough` — `cap=None` → req inchangé.
7. `permits_llm_respects_allow_list` + `permits_llm_none_allows_all`.
8. `registry_lookup_hit_and_miss` — get(connu)=Some, get(inconnu)=None.
9. `resolve_falls_back_to_default_tenant` — `req_tenant=None` → default + lookup.

### 10.2 Intégration workflow (`tests/multi_tenant_policies.rs`)

10. `tenant_policies_default_falls_back_to_global` — Coordinator sans
    registry + requête sans tenant ⇒ comportement R3.S4 identique
    (non-régression, byte-compare sur `final_text`/`selected_llm`).
11. `tenant_a_quality_table_vs_tenant_b_cost_table` — 2 policies avec tables
    distinctes ⇒ **2 `selected_llm` différents** sur la même `task_class`
    (critère d'acceptation central).
12. `tenant_allowed_llms_filters_available` — policy `allowed=[gemini]`,
    requête `[claude, gemini]` ⇒ router ne voit que gemini ;
    `tenant_llms_filtered == Some(1)`.
13. `tenant_deny_all_fails_closed` — `allowed=Some([])` ⇒
    `Err(CoordError::Config)` (pas de fuite).
14. `tenant_max_cost_caps_mission` — policy cap 500 µ¢, requête budget 50 000 µ¢
    ⇒ router filtre les LLMs > 500 µ¢ (réutilise la logique `router.rs:473`).
15. `tenant_id_propagated_to_audit` — exécuter avec `tenant_id=T` ⇒ toutes les
    `AuditEntry.tenant_id == T` (fin fiction L5-a) ; vérifier via le
    `CallContext`/audit writer.
16. `mission_b_allow_list_below_quorum_errors` — allow-list laisse 1 LLM ⇒ B
    refuse (`< 2`).
17. `tenant_circuit_isolation` — 2 tenants, circuits `(T1,analyzer)` ouvert,
    `(T2,analyzer)` fermé ⇒ T1 skip analyzer, T2 OK (blast radius isolé).
18. `request_table_overrides_tenant_table` — requête fournit une table ⇒ elle
    gagne sur la policy (sémantique §4.2, champ non-sécuritaire).

### 10.3 Live (gate `EFE_LIVE_LLM=1`, skip CI)

- `r3s5_live_two_tenants_diverge` — Claude+Gemini réels, tenant qualité vs
  tenant coût ⇒ routings observés différents + coûts µ¢ différents tracés.

---

## 11. Sécurité (anti-confusion tenant)

| Menace | Mitigation R3.S5 |
|---|---|
| **Élargissement par requête** (req demande un LLM hors plan) | allow-list = **intersection**, jamais union (§4.2) ; `permits_llm` fail-closed |
| **Plan vide ouvre par accident** | `Some([])` = deny-all explicite (test #4, #13) |
| **Cross-tenant audit** (entrées mélangées) | vrai `tenant_id` dans `CallContext` (fix L5-a, test #15) |
| **Blast radius circuit** (tenant bruyant DoS les autres) | clés `(tenant, robot)` (§8, test #17) |
| **Budget contourné** | cap = `min(req, policy)` ; resserrement **avant** le filtre router (§7.1) |
| **Policy injection via payload** | `scoring_table` est `#[serde(skip)]` (jamais désérialisée d'un payload — `request.rs:64`) ; les policies viennent du registre boot, pas du wire |
| **Tenant inconnu** | `get()=None` ⇒ défauts globaux (pas d'escalade), comportement explicite |

**Invariant de sécurité formel** : pour tout champ sécuritaire `S`
(`allowed_llms`, `max_cost`), `S_effectif ⊑ S_policy` (jamais plus permissif
que la policy). Encodé par `filter_llms` (∩) et `cap_microcents` (`min`).

---

## 12. Plan de modification fichier-par-fichier

| Fichier | Action | Détail |
|---|---|---|
| `src/multi_tenant.rs` | **NEW** | `TenantId` re-export, `TenantPolicies`, `PolicyRegistry`, `resolve`, `circuit_key`, ~9 tests |
| `src/lib.rs` | patch | `pub mod multi_tenant; pub use multi_tenant::{TenantPolicies, PolicyRegistry};` |
| `src/coordinator.rs` | patch | `Coordinator.policies: Arc<PolicyRegistry>` ; builder `.policies()` ; `slm_circuit_for` ; `tenant_slm_circuits` helper |
| `src/request.rs` | patch | `tenant_id: Option<TenantId>` sur les 3 requêtes + init dans helpers |
| `src/metrics.rs` | patch | `tenant_id`, `tenant_cost_cap_exceeded`, `tenant_llms_filtered` + littéral test |
| `src/events/stream.rs` | patch | littéral `ChewingMetrics` test I3 (tripwire R3.S4) |
| `src/workflows/mission_a.rs` | patch | resolve + allow-list + table + budget + circuit (§7.1) |
| `src/workflows/mission_b.rs` | patch | resolve + allow-list + quorum re-check + cost trace (§7.2) |
| `src/workflows/speculative.rs` | patch | resolve + allow-list drafter/verifier + budget cap (§7.3) |
| `tests/multi_tenant_policies.rs` | **NEW** | tests #10-#18 |
| `tests/live_missions.rs` | patch | `r3s5_live_two_tenants_diverge` (gate live) |
| `CHANGELOG.md` | patch | section R3.S5 |

---

## 13. Phasage d'implémentation (12 phases)

1. **Phase 1** — `src/multi_tenant.rs` : types + `resolve` + `circuit_key` +
   9 tests unitaires. `cargo test` module isolé.
2. **Phase 2** — `lib.rs` re-exports. `cargo check`.
3. **Phase 3** — `coordinator.rs` : champ `policies` + builder `.policies()` +
   `slm_circuit_for` + `tenant_slm_circuits`. `cargo check`.
4. **Phase 4** — `request.rs` : `tenant_id` ×3 + init helpers. `cargo check`.
5. **Phase 5** — `metrics.rs` + `events/stream.rs` : 3 champs + littéraux test.
6. **Phase 6** — wire `mission_a.rs` (resolve, allow-list, table, budget,
   circuit).
7. **Phase 7** — wire `mission_b.rs` (resolve, allow-list, quorum, cost trace).
8. **Phase 8** — wire `speculative.rs` (resolve, allow-list, budget).
9. **Phase 9** — `tests/multi_tenant_policies.rs` (tests #10-#18).
10. **Phase 10** — `tests/live_missions.rs` live test (compile + skip).
11. **Phase 11** — suite complète `--features all-providers` + `clippy --tests`
    (exit 0) ; vérifier les 2 critères d'acceptation.
12. **Phase 12** — CHANGELOG + commit (fichiers explicites, jamais `add -A`).

---

## 14. Risques & mitigations

| Risque | Prob. | Impact | Mitigation |
|---|---|---|---|
| Dead config (`retry_policy`) | — | moyen | **descopé** §4.3 |
| Régression mono-tenant | faible | élevé | invariant §1.3 + test #10 (byte-compare) |
| Seuil circuit per-tenant sans effet (clé manquante) | moyen | faible | documenté §8 + test #17 explicite |
| Mission B cost cap non bloquant mal compris | moyen | moyen | trace `tenant_cost_cap_exceeded`, doc §7.2 |
| Littéraux `ChewingMetrics` oubliés | moyen | bas | tripwire connu R3.S4 (compile-check tests) |
| Confusion sémantique req vs policy | moyen | élevé | table §4.2 + tests #14/#18 |
| Allow-list vide ouvre par défaut | faible | **critique** | `Some([])`=deny-all, test #4/#13 |

---

## 15. Critères d'acceptation

- [ ] ≥ 18 tests offline PASS (9 unitaires + 9 intégration).
- [ ] **2 tenants distincts → 2 routings différents** sur la même requête
      (test #11).
- [ ] **2 tenants → 2 coûts µ¢ différents** observés (cap, test #14).
- [ ] Audit chain trace le **vrai** `tenant_id` par mission (test #15) — fin
      de la fiction `default_tenant`.
- [ ] Allow-list **fail-closed** (`Some([])` → erreur, pas de fuite, test #13).
- [ ] Isolation de circuit inter-tenant (test #17).
- [ ] Non-régression mono-tenant byte-pour-byte (test #10).
- [ ] `cargo test --features all-providers` exit 0 ; `clippy --tests` exit 0.
- [ ] Backward-compat JSON : payloads pré-S5 désérialisent (`tenant_id=None`).

---

## 16. Hors scope / suites futures

| Item | Sprint candidat | Raison du report |
|---|---|---|
| `retry_policy` per-tenant | post-S5 (infra retry d'abord) | aucun retry à paramétrer (§4.3) |
| PII redaction per-tenant | sprint dédié | `PiiRedactor` global, touche 3 providers |
| Rate-limit / quota per-tenant | sprint dédié | aucune infra (headers observationnels) |
| Persistance policies (PG/fichier) | wiring gateway | R3.S5 = registre en mémoire |
| Mission B cost **blocking** (pré-fan-out) | post-S5 | exige estimation router-like avant fan-out |
| Data residency / region pin | S13 routing contextuel | orthogonal aux policies de coût/qualité |

---

## 17. Annexe — pourquoi ces points d'ancrage et pas d'autres

Le pipeline expose déjà **exactement** les leviers nécessaires, ce qui rend
R3.S5 chirurgical plutôt qu'invasif :

- **Scoring** : l'override per-request (`request.rs:65`, S6-R2) prouve que le
  router accepte une table à la volée (`mission_a.rs:196`). Per-tenant =
  fournir cette table depuis le profil au lieu de la requête.
- **Allow-list** : le router consomme `available_llms` (`mission_a.rs:218`) ;
  filtrer cette liste **avant** suffit, sans toucher au router.
- **Coût** : le filtre budgétaire µ¢ existe (`router.rs:118`, testé
  `router.rs:473`) ; resserrer `Budget.max_cost_microcents` **réutilise** ce
  filtre — zéro nouvelle logique de coût.
- **Circuit** : R3.S2 a posé `slm_circuits` + lookup (`coordinator.rs:118`) ;
  changer la clé `robot` → `{tenant}:{robot}` isole sans réécrire la garde.

Aucun de ces points n'exige de refonte : R3.S5 est une **couche de
résolution** posée devant des mécanismes déjà éprouvés (R3.S1/S2/S6-R2).
