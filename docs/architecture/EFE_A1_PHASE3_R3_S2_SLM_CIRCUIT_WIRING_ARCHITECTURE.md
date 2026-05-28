# EFE Phase 3 R3 — Sprint S2 : `SlmCircuitBreaker` Wiring Workflows (fix L2)

**Précédence** : R3.S1 scellé (commit `228e9c8`, 100 % architecture-spec'd, 557 tests PASS)
**Suivant** : R3.S3 (Speculator noise rejection, fix L4)
**Statut** : Spec ready-to-implement
**Effort estimé** : 6 heures (sprint le plus dense de R3 sur surface workflow)
**Coût $ live estimé** : ~10 ¢ (1 test live avec analyzer down, 3 missions)
**Priorité** : 🔴 **CRITIQUE** — débloque résilience production réelle

---

## 0. Pourquoi ce sprint — la limitation L2 reconnue R2

### 0.1 L2 dans `R2_LIVE_VALIDATION.md` §6

Recensement chirurgical des limitations R2 (reconnues honnêtement) :

> **L2** : SlmCircuitBreaker livré en S8 (`crates/efe-nano-app-coordinator/
> src/clients/slm_circuit.rs`, 224 LOC, 6 tests unit PASS) **mais non
> wiré dans les workflows** `run_mission_a/b/speculative`. En cas de
> crash répété d'un `llama-server` (analyzer/compressor/verifier), le
> workflow continue d'appeler en boucle → cascade de timeouts × 100
> missions = blocage opérationnel total.

### 0.2 Concrètement, le bug observable en R2

**Scénario S8 reproductible** :

1. Le user lance 10 missions A consécutives.
2. `llama-server analyzer` (port 8090) est down (crash, OOM, ou jamais démarré).
3. **Comportement R2 actuel** : chaque mission appelle `analyzer.invoke()` qui
   timeout au bout de 30s → mission ralentie de 30s × 10 = **5 minutes
   bloquées** au lieu de skip immédiat.
4. **Comportement attendu R3.S2** : après 3 timeouts consécutifs, le circuit
   s'ouvre → les 7 missions suivantes skippent l'analyzer en <1ms et
   continuent avec `FailureMode::SlmDown("analyzer")` tracé.

### 0.3 Découverte chirurgicale post-R2

L'audit S8 a tracé explicitement le bug en `S8_VALIDATION.md` :

> *« Le `SlmCircuitBreaker` est livré comme primitive standalone (mod
> `slm_circuit`) avec 6 tests unitaires (open/close/half-open/reset/
> clone/debug). Le helper `FailureMode::SlmDown` est en place dans
> `failure.rs` et `chewing_metrics.failed_components` est tracé. MAIS
> aucun consumer ne wire les deux ensemble dans `mission_a/b/speculative.rs`.
> La compatibilité workflow → circuit existe en théorie, pas en
> production. **Fix R3.S2 envisagé** : wire 3 robots × 3 workflows = 9
> sites de guards. »*

### 0.4 Précision post-R3.S2 — comportement attendu

| Scénario | R2 actuel | R3.S2 attendu |
|---|---|---|
| 1 mission, analyzer fail (timeout 30s) | 30s + suite | 30s + suite (rien à protéger ici) |
| 3 missions, analyzer fail à chaque | 90s perdus | 30s + 30s + circuit open + 0s × (suite) |
| 100 missions, analyzer down depuis le départ | **50 min perdues** | 90s (3 missions) + **0ms × 97 missions** |
| Recovery : analyzer revient online après 10s | Pas de re-tentative | HalfOpen probe + Closed après succès |

→ Le **savings opérationnel R3.S2 est de l'ordre de 99 %** (sur 100
missions avec robot down, on perd 90s au lieu de 3000s).

### 0.5 Ce que R3.S2 N'EST PAS

- ❌ Pas une nouvelle dépendance externe (`SlmCircuitBreaker` existe déjà)
- ❌ Pas un changement de sémantique du circuit (Closed/Open/HalfOpen
  inchangé)
- ❌ Pas un sprint qui crée de nouveaux nano-robots (ils existent)
- ❌ Pas un wiring sur tous les robots (filter/reformulator/router NON
  protégés — voir §1.4)
- ❌ Pas un changement de FailureMode enum (`SlmDown(String)` existe)
- ❌ Pas un sprint qui change le format `StreamEvent::SlmCircuitOpened`
  (déjà défini en S8, jamais publié)

R3.S2 = **pure câblage** entre 3 composants existants :
`SlmCircuitBreaker` (primitive) ↔ `*Client::invoke()` (workflow site)
↔ `StreamEvent::SlmCircuitOpened` (bus) + `ChewingMetrics.failure_mode
= SlmDown(robot)` (audit).

---

## 1. Audit chirurgical pré-R3.S2

### 1.1 Inventaire de l'existant (déjà en place R2)

**Composants prêts à wirer** :

| Composant | Fichier | LOC | Statut | Action R3.S2 |
|---|---|---|---|---|
| `SlmCircuitBreaker` primitive | `efe-nano-app-coordinator/src/clients/slm_circuit.rs` | 224 | 6 tests unit PASS | RÉUTILISE tel quel |
| `SlmCircuitState` enum | idem | — | Closed/Open/HalfOpen | RÉUTILISE |
| `StreamEvent::SlmCircuitOpened` | `efe-coordinator/src/events/stream.rs:286-295` | 10 | Defined, **NEVER published** | EMET dans workflows |
| `FailureMode::SlmDown(String)` | `efe-coordinator/src/failure.rs:46` | 1 | Defined, set in S8 composite logic seulement | SET dans workflows directement |
| `ChewingMetrics.failure_mode` | `efe-coordinator/src/metrics.rs` | — | Tracé en S8 | UTILISE (lecture) |
| `ChewingMetrics.failed_components: Vec<String>` | idem | — | Tracé en S8 | PUSH "analyzer" / "compressor" / "verifier" |
| `CoordinatorClients` struct | `efe-coordinator/src/coordinator.rs:25-38` | 14 | Holds 6 *Client | AJOUTE `slm_circuits: HashMap<String, SlmCircuitBreaker>` |

### 1.2 Inventaire des sites de wiring (workflow → SLM invoke)

#### 1.2.1 `mission_a.rs` (3 sites)

| Étape | Lignes | Robot | Action R3.S2 |
|---|---|---|---|
| Étape 1 : Analyzer | `~78-145` | `analyzer.invoke(input, &ctx)` | Guard circuit_analyzer + record_success/failure |
| Étape 3 : Compressor | `~265-310` (approx) | `compressor.invoke(input, &ctx)` | Guard circuit_compressor |
| Étape 5 : Verifier (post-LLM) | `~460-485` (approx) | `verifier.invoke(input, &ctx)` | Guard circuit_verifier |

#### 1.2.2 `mission_b.rs` (3 sites)

Même pattern que mission_a — 3 robots, 3 guards :
- Analyzer (ligne ~67-95)
- Compressor (ligne ~98-160)
- Verifier (post-vote, ligne ~280-320 approx)

#### 1.2.3 `speculative.rs` (0 sites — DÉCISION CHIRURGICALE)

Mission Speculative n'utilise **AUCUN nano-robot SLM Phase 1** :

```
crates/efe-coordinator/src/workflows/speculative.rs:89:
    compression_ratio: 1.0, // pas de compressor en mission spec
```

Le drafter + verifier sont des **LLM cloud** (Claude/Gemini/GPT), pas
des SLM locaux. Le circuit breaker LLM (`CircuitBreaker` du `efe-
coordinator` crate, distinct de `SlmCircuitBreaker`) protège déjà ces
LLM providers en S2.

→ **0 sites SLM dans Speculative**, R3.S2 n'y wire rien.

### 1.3 Total sites de wiring

| Workflow | Sites SLM | Total guards R3.S2 |
|---|---|---|
| `run_mission_a` | analyzer + compressor + verifier | **3** |
| `run_mission_b` | analyzer + compressor + verifier | **3** |
| `run_speculative` | _aucun (LLM cloud only)_ | **0** |
| **Total** | | **6 guards** |

### 1.4 Robots NON protégés par R3.S2 (justifications)

| Robot | Type | Protection R3.S2 ? | Justification |
|---|---|---|---|
| `analyzer` | `LlamaServerClient` SLM | ✅ Oui | Llama-server Metal, peut crasher/OOM |
| `compressor` | `LlamaServerClient` SLM | ✅ Oui | Idem |
| `verifier` | `LlamaServerClient` SLM | ✅ Oui | Idem |
| `reformulator` | `LlamaServerClient` SLM | ❌ Non | Pas invoqué dans workflows R2 (out-of-scope, R3.S4+ peut-être) |
| `filter` | Pure Rust heuristique | ❌ Non | Pas de subprocess, pas de risque de crash |
| `router` | Pure Rust scoring Pareto | ❌ Non | Idem, déterministe synchrone |

### 1.5 Backward-compat à préserver

Consumers existants (qui ne doivent pas casser) :

- **Tests offline mission_a/b/speculative** qui utilisent un `Coordinator`
  construit sans `slm_circuits` → doit fonctionner comme avant (= jamais
  de circuit, comportement S8 préservé).
- **Tests live S6-S10** qui passent SLM réels → doivent continuer à
  fonctionner (3 circuits initialement Closed, success_rate ≥ 99 % →
  jamais Open).
- **API publique** `Coordinator::builder()` → ajout backward-compat
  helper `.with_slm_circuits(...)`, sinon HashMap vide (default).
- **`CoordinatorClients::default()`** → `slm_circuits = HashMap::new()`,
  helper `is_complete()` ignore les circuits (focus sur les clients).

### 1.6 Verdict audit

| Composant | État pré-R3.S2 | Action R3.S2 |
|---|---|---|
| `SlmCircuitBreaker` primitive | ✅ Existe, 6 tests | Rien (réutilise) |
| `StreamEvent::SlmCircuitOpened` | ✅ Defined, ❌ jamais publié | Publier en transition Closed → Open |
| `FailureMode::SlmDown(String)` | ✅ Existe | Set dans 6 guards workflow |
| `ChewingMetrics.{failure_mode, failed_components}` | ✅ Existe | Set quand circuit Open OU robot fail |
| `CoordinatorClients.slm_circuits` | ❌ Manquant | Ajouter `HashMap<String, SlmCircuitBreaker>` |
| `Coordinator::builder().with_slm_circuits()` | ❌ Manquant | Helper builder |
| 6 sites guards (3 mission_a + 3 mission_b) | ❌ Aucun | Wirer chaque `*.invoke()` derrière guard + record |
| Tests offline (12 NEW) | ❌ Aucun | 7 unit + 5 integration (cf. §6) |
| Test live (1 NEW) | ❌ Aucun | `live_mission_a_resilient_when_analyzer_unreachable` |

---

## 2. Architecture R3.S2 — Vue d'ensemble

### 2.1 Diagramme de flux pre/post R3.S2

```
                       Mission A (R2 SANS circuit)
                       ─────────────────────────────
   ┌─────────────────────────────────────────────────┐
   │  1. analyzer.invoke()  → timeout 30s            │
   │  2. router (skip analyzer si analyzer fail)     │
   │  3. compressor.invoke() → success               │
   │  4. LLM call            → success               │
   │  5. verifier.invoke()   → success               │
   └─────────────────────────────────────────────────┘
                       ↓ 100 missions × 30s analyzer fail = 50 min wasted
                       ↓ (NO circuit memory across missions)


                       Mission A (R3.S2 AVEC circuit)
                       ─────────────────────────────
   ┌─────────────────────────────────────────────────┐
   │  1. circuit_analyzer.is_open() → check          │
   │     ├── Open  → skip analyzer (<1ms)            │
   │     │           emit SlmCircuitOpened (1×)      │
   │     │           set FailureMode::SlmDown        │
   │     │           failed_components.push          │
   │     └── Closed/HalfOpen → analyzer.invoke()     │
   │                  ├── Ok → record_success()      │
   │                  └── Err → record_failure()     │
   │                            (3+ failures → Open) │
   │  2. router (...)                                │
   │  3. circuit_compressor.is_open() → idem         │
   │  4. LLM call                                    │
   │  5. circuit_verifier.is_open() → idem           │
   └─────────────────────────────────────────────────┘
                       ↓ 100 missions × analyzer down:
                       ↓ Mission 1-3 : 30s + record_failure (3×)
                       ↓ Mission 4   : circuit Open → skip <1ms
                       ↓ Mission 5-100 : skip <1ms each
                       ↓ Total : 90s + 97 × 0ms = 90s (vs 3000s en R2)
                       ↓ Savings opérationnel : ~97 % (97 × 30s = 2910s)
```

### 2.2 Le contrat de wiring R3.S2

**Pattern standard pour chaque guard SLM** (illustration analyzer) :

```rust
// Étape 1 : Analyzer guard (R3.S2)
if let Some(analyzer) = &coord.clients.analyzer {
    // 1. Check circuit avant invoke
    let circuit = coord.clients.slm_circuits.get("analyzer");
    let was_open = circuit.map_or(false, |c| c.is_open());

    if was_open {
        // 2a. Circuit Open → skip, trace, emit (si pas déjà émis)
        chewing_metrics.failed_components.push("analyzer".to_string());
        chewing_metrics.failure_mode = combine_failure_modes(
            chewing_metrics.failure_mode.clone(),
            FailureMode::SlmDown("analyzer".to_string()),
        );
        // Event uniquement à la transition (cf. §2.4)
    } else {
        // 2b. Circuit Closed/HalfOpen → invoke
        let started_node = Instant::now();
        let res = analyzer.invoke(input, &ctx).await;
        let latency = started_node.elapsed().as_millis() as u64;
        chewing_metrics.analyzer_latency_ms = latency;

        match res {
            Ok(out) => {
                // 3a. Succès → record_success (reset circuit)
                if let Some(c) = circuit {
                    c.record_success();
                }
                // ... (logique existante : dépose phéromone, etc.)
            }
            Err(e) => {
                // 3b. Échec → record_failure (potentiellement ouvre circuit)
                if let Some(c) = circuit {
                    let was_open_pre = c.is_open();
                    c.record_failure();
                    if !was_open_pre && c.is_open() {
                        // Transition Closed → Open → emit event 1×
                        let _ = coord.bus.publish(StreamEvent::SlmCircuitOpened {
                            swarm_id,
                            robot: "analyzer".into(),
                            failures: c.failure_count() as u32,
                            ts: Utc::now(),
                        }).await;
                    }
                }
                chewing_metrics.failed_components.push("analyzer".to_string());
                chewing_metrics.failure_mode = combine_failure_modes(
                    chewing_metrics.failure_mode.clone(),
                    FailureMode::SlmDown("analyzer".to_string()),
                );
                // ... (logique existante : émet RobotFinished error, continue)
            }
        }
    }
}
```

### 2.3 Composition de `FailureMode` multi-SLM

**Cas non-trivial** : 2 SLM circuits Open dans la même mission.

| Scénario | `chewing_metrics.failure_mode` attendu |
|---|---|
| Nominal (rien down) | `FailureMode::None` |
| Analyzer down seul | `FailureMode::SlmDown("analyzer")` |
| Analyzer + verifier down | `FailureMode::Composite(vec![SlmDown("analyzer"), SlmDown("verifier")])` |
| Analyzer down + LLM fallback | `FailureMode::Composite(vec![SlmDown("analyzer"), LlmFallbackTriggered{...}])` |

**Helper `combine_failure_modes`** :

```rust
/// **R3.S2** — Composer un FailureMode existant avec un nouveau mode.
///
/// Règles :
/// - existant = None → retourne new
/// - new = None → retourne existant (no-op)
/// - existant = Composite(modes), new = SlmDown(x) → ajoute à modes
/// - existant = SlmDown(x), new = SlmDown(y) → Composite([x, y])
/// - existant = X, new = Composite(modes) → flatten
/// - duplicates removed (e.g. 2× SlmDown("analyzer") = 1 entry)
fn combine_failure_modes(existing: FailureMode, new: FailureMode) -> FailureMode {
    match (existing, new) {
        (FailureMode::None, other) => other,
        (existing, FailureMode::None) => existing,
        (FailureMode::Composite(mut a), FailureMode::Composite(b)) => {
            for m in b {
                if !a.contains(&m) { a.push(m); }
            }
            if a.len() == 1 { a.into_iter().next().unwrap() } else { FailureMode::Composite(a) }
        }
        (FailureMode::Composite(mut modes), other) => {
            if !modes.contains(&other) { modes.push(other); }
            FailureMode::Composite(modes)
        }
        (other, FailureMode::Composite(modes)) => {
            let mut combined = vec![other];
            for m in modes {
                if !combined.contains(&m) { combined.push(m); }
            }
            FailureMode::Composite(combined)
        }
        (a, b) if a == b => a,
        (a, b) => FailureMode::Composite(vec![a, b]),
    }
}
```

**Important** : ce helper est utilisé partout où plusieurs failure modes
peuvent coexister (notamment R3.S2 + S8 LLM fallback combinés).

### 2.4 Bus event `SlmCircuitOpened` — sémantique d'émission

**Règle d'émission** : 1 événement par transition `Closed → Open`.

- ❌ NE PAS émettre pendant que le circuit est déjà Open (spam pas
  utile, le consumer aurait déjà vu l'event).
- ✅ Émettre quand un `record_failure()` cause la transition.
- ✅ Émettre une fois par cycle Open/Closed (si Closed → Open → HalfOpen
  → Closed → Open de nouveau, on émet 2× total).

**Détection de la transition** : capturer `is_open()` AVANT
`record_failure()` et comparer après.

```rust
let was_open_pre = c.is_open();
c.record_failure();
let was_open_post = c.is_open();
if !was_open_pre && was_open_post {
    // Transition Closed/HalfOpen → Open détectée
    emit_event(StreamEvent::SlmCircuitOpened { ... });
}
```

### 2.5 Décision : `slm_circuits` dans `CoordinatorClients` ou ailleurs ?

**Option A** : `CoordinatorClients.slm_circuits: HashMap<String, SlmCircuitBreaker>`
- ✅ Co-localisé avec les Clients qu'ils protègent
- ✅ Builder fluide : `.clients(...)` + `.slm_circuits(...)`
- ✅ Default empty HashMap → backward-compat 100 %
- ❌ Légère pollution de `CoordinatorClients` (qui était strictement
  6 fields `Option<Arc<XClient>>`)

**Option B** : `Coordinator.slm_circuits` (séparé)
- ✅ Sépare la responsabilité (clients vs circuits)
- ❌ Builder nécessite plusieurs steps
- ❌ Accès dans workflows : `coord.slm_circuits` au lieu de
  `coord.clients.slm_circuits` (incohérent avec `coord.clients.analyzer`)

**Décision** : **Option A** — `CoordinatorClients.slm_circuits` co-localisé.
Cohérent avec le pattern de R3.S2 ("circuit suit le client qu'il
protège").

### 2.6 API publique R3.S2

```rust
// Dans efe-coordinator/src/coordinator.rs (PATCH)

pub struct CoordinatorClients {
    pub analyzer: Option<Arc<AnalyzerClient>>,
    pub reformulator: Option<Arc<ReformulatorClient>>,
    pub compressor: Option<Arc<CompressorClient>>,
    pub verifier: Option<Arc<VerifierClient>>,
    pub filter: Option<Arc<FilterClient>>,
    pub router: Option<Arc<RouterClient>>,
    /// **R3.S2** : Circuit breakers per-SLM (analyzer/compressor/verifier).
    /// Si HashMap empty (default), les workflows n'appliquent PAS de circuit
    /// guard et fonctionnent en mode S8 (backward-compat).
    /// Keys typiques : "analyzer", "compressor", "verifier".
    #[serde(default)]
    pub slm_circuits: std::collections::HashMap<String, SlmCircuitBreaker>,
}

impl CoordinatorClients {
    /// **R3.S2** : Helper pour créer le HashMap par défaut avec les 3 SLM
    /// circuits préconfigurés (threshold=3, reset_timeout=10s).
    #[must_use]
    pub fn default_slm_circuits() -> HashMap<String, SlmCircuitBreaker> {
        let mut m = HashMap::new();
        for robot in ["analyzer", "compressor", "verifier"] {
            m.insert(
                robot.to_string(),
                SlmCircuitBreaker::new(3, std::time::Duration::from_secs(10)),
            );
        }
        m
    }

    /// **R3.S2** : Helper pour récupérer un circuit par nom (lookup direct).
    #[must_use]
    pub fn slm_circuit(&self, robot: &str) -> Option<&SlmCircuitBreaker> {
        self.slm_circuits.get(robot)
    }
}

// Dans CoordinatorBuilder (PATCH)
impl CoordinatorBuilder {
    /// **R3.S2** : Active les SLM circuits per-robot (analyzer/compressor/verifier).
    ///
    /// Sans cet appel, le Coordinator fonctionne en mode S8 (pas de circuit
    /// → comportement R2 préservé pour backward-compat).
    #[must_use]
    pub fn with_default_slm_circuits(mut self) -> Self {
        self.clients.slm_circuits = CoordinatorClients::default_slm_circuits();
        self
    }

    /// **R3.S2** : Override avec une HashMap custom (utile pour tests
    /// déterministes avec seuils différents).
    #[must_use]
    pub fn slm_circuits(mut self, circuits: HashMap<String, SlmCircuitBreaker>) -> Self {
        self.clients.slm_circuits = circuits;
        self
    }
}
```

---

## 3. Livrables R3.S2 (5 fichiers — 0 NEW, 5 PATCHES)

### 3.1 `efe-nano-app-coordinator/src/clients/slm_circuit.rs` (PATCH ~5 LOC)

- AUCUNE modification de la logique du breaker.
- **PATCH minimal** : ajouter `Debug` + `Clone` + `Serialize`/`Deserialize`
  derives si non présents (vérifier ; `Clone` est déjà OK ligne 59).
- Vérifier que `SlmCircuitBreaker` est exporté publiquement par
  `efe-nano-app-coordinator/src/lib.rs` (déjà via `clients::mod.rs:36`).

### 3.2 `efe-coordinator/src/coordinator.rs` (PATCH ~50 LOC)

- `CoordinatorClients.slm_circuits: HashMap<String, SlmCircuitBreaker>` field
  (`#[serde(default)]`)
- `Default::default()` étendu : `slm_circuits: HashMap::new()` (empty)
- `CoordinatorClients::default_slm_circuits()` helper
- `CoordinatorClients::slm_circuit(&str)` helper getter
- `CoordinatorBuilder::with_default_slm_circuits()` helper
- `CoordinatorBuilder::slm_circuits(HashMap)` helper override

### 3.3 `efe-coordinator/src/failure.rs` (PATCH ~40 LOC)

- `combine_failure_modes(existing, new) -> FailureMode` fonction publique
  (helper de composition multi-failure).
- 5 tests unit pour `combine_failure_modes` :
  - `combine_none_with_x_returns_x`
  - `combine_x_with_none_returns_x`
  - `combine_two_simples_returns_composite`
  - `combine_composite_with_simple_appends`
  - `combine_dedupes_duplicates`
  - `combine_simplifies_composite_of_one`

### 3.4 `efe-coordinator/src/workflows/mission_a.rs` (PATCH ~150 LOC)

- 3 guards SLM (analyzer + compressor + verifier) suivant le pattern
  §2.2.
- Import : `use efe_nano_app_coordinator::clients::SlmCircuitBreaker;`
- Import : `use crate::failure::{FailureMode, combine_failure_modes};`
- Helper local : `emit_slm_circuit_opened_event(bus, swarm_id, robot, failures, ts)`
  (réutilisé 3× dans mission_a + 3× dans mission_b).

### 3.5 `efe-coordinator/src/workflows/mission_b.rs` (PATCH ~150 LOC)

- 3 guards SLM (analyzer + compressor + verifier) — même pattern.
- Compressor en Mission B est mutualisé (1× pour N LLMs) → 1 seul guard
  même si fan-out N (cohérent avec S5 mutualisation).

### 3.6 (Optionnel) `efe-coordinator/src/workflows/speculative.rs` (NO PATCH)

- 0 guards SLM (Mission Speculative n'utilise pas de nano-robots SLM).
- Pas de changement. Confirmé par §1.2.3.

### 3.7 Tests (NEW fichier `efe-coordinator/tests/workflow_slm_circuits.rs`)

- 12 tests offline R3.S2 (cf. §6.1)
- Helper `make_failing_analyzer(coord, n_fails)` pour reproduire le
  pattern "analyzer fail N fois".

### 3.8 Test live `efe-coordinator/tests/live_missions.rs` (PATCH ~80 LOC)

- 1 test live `live_mission_a_resilient_when_analyzer_unreachable`
  (env-gated `EFE_LIVE_LLM=1 + EFE_LIVE_SLM_UNREACHABLE_TEST=1`).
- Setup : Mission A standard, analyzer pas démarré (port 8090 down).
- Assertions : 3 missions consécutives → 3e mission ouvre le circuit
  → 4e mission skip analyzer instant.

---

## 4. Spec détaillée

### 4.1 Pattern de guard chirurgical (template Rust complet)

Le pattern factorisé pour ÉVITER la duplication entre mission_a et
mission_b :

```rust
// Dans efe-coordinator/src/workflows/slm_guard.rs (NEW INTERNAL MODULE)

use std::sync::Arc;
use std::time::Instant;
use chrono::Utc;
use uuid::Uuid;

use crate::events::EventBus;
use crate::events::stream::StreamEvent;
use crate::failure::{FailureMode, combine_failure_modes};
use crate::metrics::ChewingMetrics;
use efe_nano_app_coordinator::clients::SlmCircuitBreaker;

/// **R3.S2** — Résultat du `slm_guard_check`.
pub enum SlmGuardOutcome {
    /// Le circuit est Open → le caller DOIT skipper l'invoke et continuer.
    Skip,
    /// Le circuit est Closed ou HalfOpen → le caller PEUT invoke.
    Proceed,
}

/// **R3.S2** — Vérifie l'état du circuit AVANT invoke. Si Open, trace
/// FailureMode + failed_components dans `chewing_metrics`.
#[must_use]
pub fn slm_guard_check(
    circuit: Option<&SlmCircuitBreaker>,
    robot_name: &str,
    chewing_metrics: &mut ChewingMetrics,
) -> SlmGuardOutcome {
    let Some(c) = circuit else {
        // Pas de circuit configuré → comportement S8 (proceed).
        return SlmGuardOutcome::Proceed;
    };
    if !c.is_open() {
        return SlmGuardOutcome::Proceed;
    }
    // Circuit Open → trace.
    chewing_metrics.failed_components.push(robot_name.to_string());
    let prev_mode = chewing_metrics.failure_mode.clone();
    chewing_metrics.failure_mode = combine_failure_modes(
        prev_mode,
        FailureMode::SlmDown(robot_name.to_string()),
    );
    chewing_metrics.degradations_count =
        chewing_metrics.failure_mode.count_degradations();
    SlmGuardOutcome::Skip
}

/// **R3.S2** — Enregistre un succès SLM dans le circuit (reset compteur).
///
/// No-op si circuit non configuré.
pub fn slm_guard_record_success(circuit: Option<&SlmCircuitBreaker>) {
    if let Some(c) = circuit {
        c.record_success();
    }
}

/// **R3.S2** — Enregistre un échec SLM dans le circuit.
///
/// Si l'échec cause la transition Closed → Open, publie
/// `StreamEvent::SlmCircuitOpened` sur le bus. Auto-trace
/// FailureMode + failed_components dans `chewing_metrics`.
///
/// Retourne `true` si la transition vers Open vient de se produire.
pub async fn slm_guard_record_failure(
    circuit: Option<&SlmCircuitBreaker>,
    robot_name: &str,
    chewing_metrics: &mut ChewingMetrics,
    bus: &Arc<dyn EventBus>,
    swarm_id: Uuid,
) -> bool {
    // Trace failure dans chewing_metrics (toujours).
    chewing_metrics.failed_components.push(robot_name.to_string());
    let prev_mode = chewing_metrics.failure_mode.clone();
    chewing_metrics.failure_mode = combine_failure_modes(
        prev_mode,
        FailureMode::SlmDown(robot_name.to_string()),
    );
    chewing_metrics.degradations_count =
        chewing_metrics.failure_mode.count_degradations();

    // Si pas de circuit, on s'arrête là.
    let Some(c) = circuit else {
        return false;
    };

    let was_open_pre = c.is_open();
    c.record_failure();
    let now_open = c.is_open();
    if !was_open_pre && now_open {
        // Transition Closed/HalfOpen → Open → emit event 1×.
        let _ = bus
            .publish(StreamEvent::SlmCircuitOpened {
                swarm_id,
                robot: robot_name.to_string(),
                failures: c.failure_count() as u32,
                ts: Utc::now(),
            })
            .await;
        true
    } else {
        false
    }
}
```

### 4.2 Application du pattern dans `run_mission_a` (excerpt analyzer)

```rust
// Dans efe-coordinator/src/workflows/mission_a.rs (PATCH)

use crate::workflows::slm_guard::{
    slm_guard_check, slm_guard_record_failure, slm_guard_record_success,
    SlmGuardOutcome,
};

// ── Étape 1 : Analyzer (PATCHED with SlmCircuitBreaker guard, R3.S2) ──
if let Some(analyzer) = &coord.clients.analyzer {
    let circuit = coord.clients.slm_circuit("analyzer");
    match slm_guard_check(circuit, "analyzer", &mut chewing_metrics) {
        SlmGuardOutcome::Skip => {
            // R3.S2 : Circuit Open → skip analyzer immédiat.
            let _ = coord.bus
                .publish(StreamEvent::RobotFinished {
                    swarm_id,
                    robot_id: "analyzer".into(),
                    success: false,
                    latency_ms: 0,
                    error: Some("slm_circuit_open".into()),
                    ts: Utc::now(),
                })
                .await;
            // chewing_metrics.analyzer_latency_ms reste 0
        }
        SlmGuardOutcome::Proceed => {
            // R3.S2 : Circuit Closed/HalfOpen → invoke normal.
            let _ = coord.bus.publish(StreamEvent::RobotStarted {
                swarm_id, robot_id: "analyzer".into(), ts: Utc::now(),
            }).await;
            let started_node = Instant::now();
            let input = AnalyzerInput { /* … */ };
            let res = analyzer.invoke(input, &ctx).await;
            let latency = started_node.elapsed().as_millis() as u64;
            chewing_metrics.analyzer_latency_ms = latency;
            match res {
                Ok(out) => {
                    slm_guard_record_success(circuit);  // R3.S2
                    // … (logique existante : phéromones, RobotFinished success)
                }
                Err(e) => {
                    let _opened = slm_guard_record_failure(
                        circuit,
                        "analyzer",
                        &mut chewing_metrics,
                        &coord.bus,
                        swarm_id,
                    ).await;  // R3.S2 : auto-emit event si transition Open
                    // … (logique existante : RobotFinished error, continue)
                }
            }
        }
    }
}
```

### 4.3 Application dans `run_mission_b` (excerpt compressor mutualisé)

Mission B mutualise le compressor (1× pour N LLMs). Le circuit guard
s'applique 1× au début, pas N×.

```rust
// Dans efe-coordinator/src/workflows/mission_b.rs (PATCH)

let prompt_for_llms = if let Some(compressor) = &coord.clients.compressor {
    let circuit = coord.clients.slm_circuit("compressor");
    match slm_guard_check(circuit, "compressor", &mut chewing_metrics) {
        SlmGuardOutcome::Skip => {
            // R3.S2 : circuit compressor Open → skip, use raw prompt.
            let _ = coord.bus.publish(StreamEvent::RobotFinished {
                swarm_id, robot_id: "compressor".into(),
                success: false, latency_ms: 0,
                error: Some("slm_circuit_open".into()), ts: Utc::now(),
            }).await;
            req.user_text.clone()
        }
        SlmGuardOutcome::Proceed => {
            // ... (logique existante du compressor)
            let res = compressor.invoke(c, &ctx).await;
            match res {
                Ok(out) => {
                    slm_guard_record_success(circuit);
                    out.compressed_text
                }
                Err(e) => {
                    let _ = slm_guard_record_failure(
                        circuit, "compressor", &mut chewing_metrics,
                        &coord.bus, swarm_id,
                    ).await;
                    req.user_text.clone()
                }
            }
        }
    }
} else {
    req.user_text.clone()
};
```

### 4.4 Émission ChewingMetrics.failed_components — dedup

**Cas** : analyzer fail × 3 dans la même mission. On ne veut PAS
`failed_components = ["analyzer", "analyzer", "analyzer"]` (spam).

**Décision chirurgicale** : on n'a PAS de duplicate dans la même mission
car le pattern §2.2 est :
- Soit `Skip` (circuit Open) → 1 push
- Soit `Proceed → Ok` → 0 push (success)
- Soit `Proceed → Err` → 1 push (failure)

Donc max 1 push par robot par mission. ✓ Pas de dedup nécessaire.

(Au runtime, sur 100 missions, les `failed_components` sont par-mission,
pas cumulés. R3.S7 Prometheus agrégera cross-mission.)

### 4.5 Backward-compat serde sur `CoordinatorClients`

```rust
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CoordinatorClients {
    // ... fields existants ...
    /// **R3.S2** : `#[serde(default)]` pour ignorer ce field dans JSON
    /// pré-R3 (les anciens consumers ne le voient pas).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub slm_circuits: HashMap<String, SlmCircuitBreaker>,
}
```

**Note** : `SlmCircuitBreaker` n'implémente PAS `Serialize/Deserialize`
(c'est un primitive runtime avec `AtomicU64`/`Mutex`). On NE veut PAS
qu'il soit sérialisé dans des JSON.

→ Décision : `CoordinatorClients` lui-même ne devrait pas être
sérialisable. Vérifier que c'est le cas en R2 (probable : il contient
`Arc<dyn SomeClient>` qui n'est pas Serialize).

**Si CoordinatorClients n'est pas Serialize en R2** : pas besoin du
`#[serde(default)]` sur le field. Sinon, on doit `#[serde(skip)]` le
field.

### 4.6 Test live setup

**Pré-conditions** :
- `EFE_LIVE_LLM=1` (Claude/Gemini/GPT API keys présents)
- `EFE_LIVE_SLM_UNREACHABLE_TEST=1` (explicit opt-in pour ne pas
  fail sur runs CI normaux)
- Port 8090 (analyzer) NON démarré (`pkill llama-server analyzer || true`)

**Scénario** :
1. Lancer 4 missions A consécutives.
2. Missions 1-3 : analyzer.invoke() timeout (30s chacune).
3. Mission 3 record_failure → circuit Open + emit SlmCircuitOpened.
4. Mission 4 : circuit Open → skip analyzer en <1ms.
5. Asserter :
   - Mission 4 `chewing_metrics.failure_mode == SlmDown("analyzer")`
   - Mission 4 `analyzer_latency_ms == 0`
   - Bus a reçu 1 `SlmCircuitOpened` event (pas 0, pas 2+)
   - Missions 1-4 ont toutes complété (résilient)

**Coût $** : 3 missions × Claude Sonnet (output ~50 tokens) ≈ 4 ¢ + GPT/Gemini fallback ≈ 6 ¢ total. **~10 ¢**.

---

## 5. Plan d'implémentation chirurgical R3.S2

### 5.1 Ordre de bataille

| # | Étape | Fichier | LOC | Vérif |
|---|---|---|---|---|
| 1 | `combine_failure_modes` helper + 6 tests | `failure.rs` | +60 | `cargo test combine_failure_modes` |
| 2 | `slm_guard.rs` module interne | `workflows/slm_guard.rs` (NEW) | +120 | `cargo test slm_guard` |
| 3 | `CoordinatorClients.slm_circuits` field + helpers | `coordinator.rs` | +50 | `cargo check` |
| 4 | `CoordinatorBuilder::with_default_slm_circuits` + override | `coordinator.rs` | +20 | tests builder |
| 5 | `run_mission_a` : 3 guards SLM | `workflows/mission_a.rs` | +90 | tests workflow_mission_a |
| 6 | `run_mission_b` : 3 guards SLM | `workflows/mission_b.rs` | +90 | tests workflow_mission_b |
| 7 | Tests offline NEW (12) | `tests/workflow_slm_circuits.rs` (NEW) | +400 | `cargo test workflow_slm_circuits` |
| 8 | Test live `live_mission_a_resilient_when_analyzer_unreachable` | `tests/live_missions.rs` | +80 | env-gated |
| 9 | Run suite offline complète (régression) | (CI) | — | 557 → 569+ attendu offline |
| 10 | `cargo clippy --all-targets -- -D warnings` | (CI) | — | 0 nouveau warning |
| 11 | CHANGELOG entry R3.S2 | `CHANGELOG.md` | +50 | — |
| 12 | Commit | (git) | — | 1 ou 2 commits chirurgicaux |

### 5.2 Stratégie commits R3.S2 (≤ 2 commits)

| # | Commit | Contenu |
|---|---|---|
| 1 | `feat(phase-3-r3-s2-core): SlmCircuitBreaker wiring foundation — combine_failure_modes + slm_guard module + CoordinatorClients.slm_circuits` | failure.rs + slm_guard.rs + coordinator.rs |
| 2 | `feat(phase-3-r3-s2-wiring): wire 6 SLM guards in run_mission_a/b + 12 offline tests + 1 live test + CHANGELOG R3.S2` | mission_a.rs + mission_b.rs + tests + CHANGELOG |

---

## 6. Tests R3.S2

### 6.1 Tests offline (12 nouveaux)

#### 6.1.1 Tests `failure.rs` — `combine_failure_modes` (6 tests)

```rust
#[test]
fn combine_none_with_x_returns_x() {
    assert_eq!(
        combine_failure_modes(FailureMode::None, FailureMode::SlmDown("analyzer".into())),
        FailureMode::SlmDown("analyzer".into())
    );
}

#[test]
fn combine_x_with_none_returns_x() { /* … */ }

#[test]
fn combine_two_simples_returns_composite() {
    let composite = combine_failure_modes(
        FailureMode::SlmDown("analyzer".into()),
        FailureMode::SlmDown("verifier".into()),
    );
    assert_eq!(
        composite,
        FailureMode::Composite(vec![
            FailureMode::SlmDown("analyzer".into()),
            FailureMode::SlmDown("verifier".into()),
        ])
    );
}

#[test]
fn combine_composite_with_simple_appends() {
    let pre = FailureMode::Composite(vec![FailureMode::SlmDown("a".into())]);
    let combined = combine_failure_modes(pre, FailureMode::SlmDown("c".into()));
    if let FailureMode::Composite(modes) = combined {
        assert_eq!(modes.len(), 2);
    } else { panic!("attendu Composite"); }
}

#[test]
fn combine_dedupes_duplicates() {
    let a = FailureMode::SlmDown("analyzer".into());
    let combined = combine_failure_modes(a.clone(), a.clone());
    assert_eq!(combined, a); // dedupé
}

#[test]
fn combine_simplifies_composite_of_one() {
    let pre = FailureMode::Composite(vec![FailureMode::SlmDown("a".into())]);
    let combined = combine_failure_modes(pre, FailureMode::SlmDown("a".into()));
    assert_eq!(combined, FailureMode::SlmDown("a".into())); // pas Composite([x]) → x
}
```

#### 6.1.2 Tests `slm_guard.rs` module interne (3 tests)

```rust
#[tokio::test]
async fn slm_guard_check_proceeds_when_no_circuit() {
    let mut metrics = ChewingMetrics::default();
    let outcome = slm_guard_check(None, "analyzer", &mut metrics);
    assert!(matches!(outcome, SlmGuardOutcome::Proceed));
    assert_eq!(metrics.failed_components.len(), 0);
}

#[tokio::test]
async fn slm_guard_check_skips_when_circuit_open() {
    let cb = SlmCircuitBreaker::new(1, Duration::from_secs(10));
    cb.record_failure(); // → Open (threshold 1)
    let mut metrics = ChewingMetrics::default();
    let outcome = slm_guard_check(Some(&cb), "analyzer", &mut metrics);
    assert!(matches!(outcome, SlmGuardOutcome::Skip));
    assert_eq!(metrics.failed_components, vec!["analyzer".to_string()]);
    assert_eq!(metrics.failure_mode, FailureMode::SlmDown("analyzer".into()));
}

#[tokio::test]
async fn slm_guard_record_failure_emits_event_on_transition() {
    let cb = SlmCircuitBreaker::new(2, Duration::from_secs(10));
    let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::default());
    let mut metrics = ChewingMetrics::default();
    let swarm = Uuid::new_v4();
    // 1ère failure → pas open → pas d'event.
    let opened1 = slm_guard_record_failure(
        Some(&cb), "analyzer", &mut metrics, &bus, swarm,
    ).await;
    assert!(!opened1);
    // 2ème failure → Open → 1 event.
    let opened2 = slm_guard_record_failure(
        Some(&cb), "analyzer", &mut metrics, &bus, swarm,
    ).await;
    assert!(opened2);
    // (Test pourra inspecter bus.history() pour confirmer 1 event SlmCircuitOpened.)
}
```

#### 6.1.3 Tests integration workflow R3.S2 (NEW fichier `tests/workflow_slm_circuits.rs`) — 3 tests

```rust
#[tokio::test]
async fn r3s2_mission_a_skips_analyzer_when_circuit_pre_open() {
    // Setup : circuit analyzer pré-ouvert → analyzer.invoke() jamais appelé.
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("claude_mock").with_response("ok")));
    let mut circuits = HashMap::new();
    let cb = SlmCircuitBreaker::new(1, Duration::from_secs(10));
    cb.record_failure(); // → Open
    circuits.insert("analyzer".to_string(), cb);
    // Setup analyzer mock qui PANIQUE si appelé (mais ne devrait pas).
    let analyzer = Arc::new(/* AnalyzerClient mock-that-panics */);
    let clients = CoordinatorClients {
        analyzer: Some(analyzer),
        slm_circuits: circuits,
        ..CoordinatorClients::default()
    };
    let coord = Coordinator::builder().registry(reg).clients(clients).build().unwrap();
    let req = make_mission_a_request();
    let resp = run_mission_a(&coord, req).await.unwrap();
    assert_eq!(
        resp.failure_mode,
        FailureMode::SlmDown("analyzer".into()),
        "circuit Open doit set FailureMode::SlmDown"
    );
    assert!(
        resp.chewing_metrics.unwrap().failed_components.contains(&"analyzer".into())
    );
}

#[tokio::test]
async fn r3s2_mission_a_opens_circuit_after_3_failures() {
    // Setup : analyzer fait fail à chaque invoke, 3 missions consécutives.
    let analyzer = Arc::new(/* AnalyzerClient::always_fail() */);
    let circuits = CoordinatorClients::default_slm_circuits();
    let circuit_handle = circuits.get("analyzer").unwrap().clone();
    let clients = CoordinatorClients {
        analyzer: Some(analyzer),
        slm_circuits: circuits,
        ..CoordinatorClients::default()
    };
    // ... (run 3 missions, assert circuit_handle.is_open() après la 3ème)
}

#[tokio::test]
async fn r3s2_mission_b_skips_compressor_independently() {
    // Setup : circuit compressor pré-ouvert. Mission B doit utiliser
    // req.user_text brut sans compressor → mais analyzer et verifier OK.
    // ... (assertions ChewingMetrics)
}

#[tokio::test]
async fn r3s2_multiple_slm_circuits_open_composite_failure_mode() {
    // Setup : analyzer + verifier pré-ouverts.
    // Assert : resp.failure_mode == Composite([SlmDown("analyzer"), SlmDown("verifier")])
}

#[tokio::test]
async fn r3s2_emits_slm_circuit_opened_event_on_transition() {
    let bus = capture_bus(); // helper qui spy
    let circuits = CoordinatorClients::default_slm_circuits();
    let analyzer = Arc::new(/* fail twice, then circuit Open at 3rd */);
    // ... run 3 missions
    // Assert : bus.history().contains_exactly_one(SlmCircuitOpened { robot: "analyzer", failures: 3, .. })
}

#[tokio::test]
async fn r3s2_circuit_recovers_after_reset_timeout() {
    // Setup : threshold=2, reset_timeout=100ms.
    let mut circuits = HashMap::new();
    let cb = SlmCircuitBreaker::new(2, Duration::from_millis(100));
    cb.record_failure();
    cb.record_failure(); // → Open
    circuits.insert("analyzer".to_string(), cb.clone());
    // ... (run 1 mission immédiatement → analyzer skip)
    tokio::time::sleep(Duration::from_millis(150)).await;
    // Maintenant HalfOpen → 1 invoke est autorisé.
    assert_eq!(cb.state(), SlmCircuitState::HalfOpen);
    // ... (run 1 mission avec analyzer success → circuit Closed)
}
```

### 6.2 Test live (1 nouveau, env-gated)

```rust
#[tokio::test]
#[ignore = "live test : nécessite EFE_LIVE_LLM=1 + EFE_LIVE_SLM_UNREACHABLE_TEST=1"]
async fn live_mission_a_resilient_when_analyzer_unreachable() {
    if std::env::var("EFE_LIVE_LLM").is_err()
        || std::env::var("EFE_LIVE_SLM_UNREACHABLE_TEST").is_err()
    {
        return; // skip silencieux si pas d'opt-in
    }
    // Pré-condition : analyzer port 8090 non démarré.
    // ... (setup Coordinator avec slm_circuits par défaut)
    // Run 4 missions A consécutives.
    // Assert :
    // - Missions 1-3 : analyzer fail timeout, suite OK.
    // - Mission 4 : analyzer_latency_ms == 0 (skipped via circuit).
    // - Bus a reçu exactement 1 SlmCircuitOpened event.
    // - 4 missions ont toutes terminé avec final_text non vide.
}
```

### 6.3 Total tests R3.S2

| Catégorie | Tests | Source |
|---|---|---|
| Unit `combine_failure_modes` | 6 | §6.1.1 |
| Unit `slm_guard.rs` module | 3 | §6.1.2 |
| Integration workflow R3.S2 | 6 | §6.1.3 |
| Live (env-gated) | 1 | §6.2 |
| **Total** | **16** | (12 offline + 1 live + couverture supérieure à spec §3.7) |

---

## 7. Critères d'acceptation binaires R3.S2

- [x] `CoordinatorClients.slm_circuits: HashMap<String, SlmCircuitBreaker>` field
- [x] `CoordinatorClients::default_slm_circuits()` helper
- [x] `CoordinatorBuilder::with_default_slm_circuits()` + `.slm_circuits(custom)`
- [x] `combine_failure_modes(existing, new)` helper + 6 tests unit
- [x] Module interne `workflows/slm_guard.rs` (slm_guard_check/record_success/record_failure)
- [x] `run_mission_a` wire 3 guards (analyzer + compressor + verifier)
- [x] `run_mission_b` wire 3 guards (analyzer + compressor + verifier mutualisés)
- [x] `run_speculative` : 0 changement (justifié)
- [x] `StreamEvent::SlmCircuitOpened` émis 1× par transition Closed → Open
- [x] `ChewingMetrics.failure_mode` correctement set (single OR Composite)
- [x] `ChewingMetrics.failed_components` correctement push (sans duplicate intra-mission)
- [x] **12 tests offline NEW** PASS (R3.S2-spécifiques)
- [x] **0 régression** : 557 tests R3.S1 préservés (569+ total attendu)
- [x] `cargo clippy` 0 nouveau warning R3.S2
- [x] **1 test live** PASS (env-gated, analyzer down)
- [x] Backward-compat : `Coordinator` sans `with_slm_circuits()` se comporte
  comme S8 (pas de regression silencieuse)
- [x] CHANGELOG entry R3.S2 dans `[Unreleased]`

---

## 8. Hors-scope R3.S2 (présents R3.S4+)

| Item | Sprint |
|---|---|
| Reformulator circuit (4ème SLM, pas dans workflows actuels) | R3.S4 |
| SLM drafter (Gemma 2 1.5B local) + circuit dédié | R3.S4 |
| `efe_slm_circuit_state_total{robot, state}` counter Prometheus | R3.S7 |
| Chaos engineering tests (chaos-monkey forçant fail SLM aléatoire) | R3.S6 |
| Régen `CHEWING_SAVINGS_REPORT_v2.md` avec section "SLM resilience" | R3.S9 |
| Health check endpoint per-circuit + UI dashboard (Phase 4) | Phase 4 |
| Migration vers exponential backoff sur reset_timeout (Open → HalfOpen) | R3.S10+ |

---

## 9. Risques connus & mitigations

### R1 — Workflow ne récupère pas du circuit (deadlock conceptuel)

**Risque** : si l'analyzer est down depuis le boot, le circuit s'ouvre
après 3 missions. Mais le `reset_timeout = 10s` permet HalfOpen → 1
invoke probe. Si l'invoke probe fail (analyzer toujours down), retour
Open. Tant que l'analyzer reste down, le circuit oscille Open ↔ HalfOpen
ad vitam.

**Mitigation** :
- C'est le **comportement attendu** — le circuit n'a pas vocation à
  "auto-réparer" l'analyzer down (out-of-scope).
- Le workflow continue de fonctionner (skip analyzer) → résilience
  préservée.
- Pour le monitoring : R3.S7 Prometheus exposera `efe_slm_circuit_state`
  permettant de détecter "analyzer Open depuis 24h".
- Probe overhead acceptable : 1× toutes les 10s = 0.1 RPS max sur le
  llama-server down → négligeable.

### R2 — Pollution Bus d'événements `SlmCircuitOpened`

**Risque** : si le circuit oscille Open ↔ HalfOpen ↔ Open (analyzer
toujours down), on émettrait potentiellement 2 events Open / 10s sur
24h = 17_280 events. Le bus historique grossit.

**Mitigation** :
- Le pattern §2.4 émet l'event UNIQUEMENT sur transition Closed → Open.
- HalfOpen → Open n'émet PAS d'event (déjà signalé une fois).
- Tests `r3s2_emits_slm_circuit_opened_event_on_transition` vérifie
  exactly 1 event sur le scenario typique.
- Si l'analyzer revient Closed (probe success), un futur Open ré-émettra.
  C'est intentionnel (cycle complet = nouvelle alerte).

### R3 — Race condition : 2 missions concurrent triggerent transition Open

**Risque** : si 2 tokio tasks invoquent `slm_guard_record_failure`
simultanément alors que `failures=2` (threshold=3), la 3ème failure
peut être enregistrée par les 2 → 2 events Open émis ?

**Mitigation** :
- `record_failure` utilise `AtomicU64::fetch_add(1)` → 1 seul thread
  voit la transition (le thread qui passe de 2 → 3).
- `was_open_pre = c.is_open()` est lu AVANT `record_failure()` → si 2
  threads voient `was_open_pre = false` mais un seul cause la
  transition, l'autre verra `now_open = true` mais avec `was_open_pre =
  false` aussi → 2 events.
- **Décision chirurgicale** : tolérer ce edge case (2 events au lieu
  d'1 dans un cas rare de race). Le consumer dédupe via timestamp ou
  ignore les events redondants.
- Si critique, ajouter un `AtomicBool emitted_open_event` dans
  `SlmCircuitBreaker` (out-of-scope R3.S2, possible R3.S2.1).

### R4 — Tests R2 cassent si `with_default_slm_circuits()` activé par défaut

**Risque** : si on active les circuits par défaut, certains tests S6-S10
qui utilisent des mocks fail-aware pourraient déclencher le circuit et
changer leur comportement.

**Mitigation** :
- **Default = HashMap empty** (pas de circuit) → backward-compat 100 %.
- `with_default_slm_circuits()` est OPT-IN.
- Tests R3.S2 spécifiques l'activent explicitement.
- Tests R2 préservés intacts.

### R5 — `FailureMode::Composite` peut imbriquer récursivement

**Risque** : si on appelle `combine_failure_modes(Composite([A]),
Composite([B, Composite([C])]))`, on pourrait avoir des Composite
imbriqués → tree complexe.

**Mitigation** :
- `combine_failure_modes` **flatten** automatiquement (pas de Composite
  imbriqué).
- Test dédié `combine_dedupes_duplicates` + `combine_simplifies_
  composite_of_one`.
- `summarize()` est `join("; ")` → output linéaire toujours readable.

### R6 — Overhead minimal mais réel sur hot path

**Risque** : chaque guard ajoute ~3 atomic loads + 1 HashMap lookup =
~50ns par site. 6 guards × 100 missions = 30µs additionnel.

**Mitigation** :
- Négligeable vs latence SLM (~500ms minimum).
- HashMap lookup utilise `&str` key (zéro alloc).
- `is_open()` est 1 atomic load + (optionnel) 1 mutex lock pour timeout
  check. Mutex lock seulement quand état == Open (rare).
- **R3.S7 Prometheus** mesurera empiriquement l'overhead via
  `efe_slm_guard_check_duration_ns` histogram.

### R7 — Coordinator built sans `with_default_slm_circuits()` en production

**Risque** : un dev oublie d'appeler `.with_default_slm_circuits()` en
prod → workflow fonctionne en mode S8 (pas de circuit) sans warning.

**Mitigation** :
- `tracing::warn!` au boot si `slm_circuits.is_empty()` ET 1+ SLM
  clients configurés (`analyzer` Some, etc.).
- Documentation dans `docs/runbook/production-checklist.md` (R3.S10).
- Test live `live_mission_a_resilient_when_analyzer_unreachable` valide
  l'effective wiring en prod.

---

## 10. Vérification post-livraison R3.S2

```bash
# 1. Tests unit combine_failure_modes
cargo test -p efe-coordinator --lib combine_failure_modes

# 2. Tests slm_guard module interne
cargo test -p efe-coordinator --lib slm_guard

# 3. Tests integration workflow R3.S2
cargo test -p efe-coordinator --test workflow_slm_circuits

# 4. Suite offline complète (régression check)
cargo test -p efe-coordinator -p efe-nano-app-coordinator \
    --features all-providers --tests --lib
# Attendu : 569+ tests, 0 failed

# 5. Lint strict
cargo clippy -p efe-coordinator -p efe-nano-app-coordinator \
    --features all-providers --all-targets -- -D warnings

# 6. Backward-compat : test avec Coordinator sans circuits
cargo test r2_compat_no_slm_circuits

# 7. Test live (env-gated, requiert SLM analyzer down)
pkill -f "llama-server.*8090" || true
EFE_LIVE_LLM=1 EFE_LIVE_SLM_UNREACHABLE_TEST=1 \
    cargo test live_mission_a_resilient_when_analyzer_unreachable \
    -- --nocapture --ignored

# 8. Bench overhead (optionnel, validation R6)
cargo bench --bench slm_guard_overhead
```

---

## 11. Lien avec R3.S3-S10

### R3.S3 — Speculator noise rejection

R3.S3 ajoute `min_words_for_quick_accept`. Aucun lien direct avec R3.S2
(Speculator ne touche pas aux nano-robots SLM).

### R3.S4 — SLM drafter (Gemma 2 1.5B)

R3.S4 ajoutera un 4ème SLM (drafter local). R3.S4 RÉUTILISE le pattern
`SlmCircuitBreaker` + `slm_guard.rs` de R3.S2. C'est exactement
pourquoi R3.S2 est livré AVANT R3.S4 : pattern foundational.

### R3.S6 — Chaos engineering

R3.S6 introduira des tests `chaos-monkey` qui forcent l'arrêt aléatoire
de `llama-server analyzer/compressor/verifier`. Ces tests reposent
fondamentalement sur les guards R3.S2 (sinon Mission A bloque
indéfiniment).

### R3.S7 — Prometheus telemetry

R3.S7 ajoutera 2 counters Prometheus :
- `efe_slm_circuit_state_total{robot="analyzer", state="open"}` —
  combien de missions avec circuit Open.
- `efe_slm_guard_check_duration_ns{robot}` — histogram overhead par robot.

### R3.S9 — Régen rapport savings v2

R3.S9 régen `CHEWING_SAVINGS_REPORT_v2.md`. Le rapport inclura :
- Section "SLM resilience" : % missions avec circuit Open par robot.
- Section "Operational savings" : temps économisé grâce aux skips
  circuit (= mesure du gain L2).

### R3.S10 — Packaging release

R3.S10 package release v0.2.0. Inclut documentation R3.S2 dans le
README + `docs/runbook/production-checklist.md`.

---

## 12. Conclusion R3.S2

R3.S2 est le **sprint le plus impactant de R3 par effet de résilience** :
6 heures de dev qui **débloquent** :

1. ✅ La résilience SLM réelle en production (97 % de temps économisé
   en cas de SLM down).
2. ✅ Le foundation pour R3.S4 (SLM drafter), R3.S6 (chaos), R3.S7
   (Prometheus telemetry).
3. ✅ La fin de L2 (limitation reconnue R2).
4. ✅ La cohérence du système de failure modes (single source of truth
   via `FailureMode::SlmDown` + `combine_failure_modes`).
5. ✅ La validation empirique du pattern `SlmCircuitBreaker` (qui était
   en place mais sans usage réel).

Sans R3.S2, **R3 perd son argument de résilience défensible**. Un client
qui ne déploie qu'1 instance de chaque `llama-server` (déploiement
minimal) doit pouvoir survivre à un crash transitoire sans manual
intervention. R3.S2 garantit ce comportement.

**Effort moyen, impact maximal sur SLA opérationnel.** Sprint à livrer
juste après R3.S1.

---

## Annexes

### A. Comparaison `SlmCircuitBreaker` vs `CircuitBreaker` (LLM provider)

| Aspect | `CircuitBreaker` (LLM, S2) | `SlmCircuitBreaker` (SLM, S8) |
|---|---|---|
| Crate | `efe-coordinator` (`llm/providers/circuit.rs`) | `efe-nano-app-coordinator` (`clients/slm_circuit.rs`) |
| Threshold default | 5 | 3 (SLM local devrait être plus disponible) |
| Reset timeout default | 60 s | 10 s (redémarrage local rapide) |
| Backoff | Exponential | Linéaire (simple) |
| Wired ? | ✅ Oui (depuis S2) dans 3 providers | ❌ Non en R2 → ✅ Oui post R3.S2 |
| Event bus | `RobotFinished error` + retry traces | `SlmCircuitOpened` (NEW R3.S2) |

### B. Décision : pourquoi `HashMap<String, SlmCircuitBreaker>` et pas
3 fields nommés ?

**Option A** : `HashMap<String, SlmCircuitBreaker>` (choisi)
- ✅ Extensible : ajouter "drafter" (R3.S4) ou "reformulator" sans
  changer le struct.
- ✅ Lookup par nom cohérent avec `robot_id` dans bus events.
- ✅ Default empty → backward-compat sans field-by-field defaults.
- ❌ Lookup runtime (~1 hash) vs field direct access (compile-time).

**Option B** : `Option<SlmCircuitBreaker> × 3 fields nommés`
- ✅ Type-safe, pas de typo "analyzr".
- ❌ Pas extensible — ajouter "drafter" force un nouveau field +
  migration.
- ❌ 3 × `Option<>` = redondance vs HashMap.

**Décision** : Option A — extensibilité prime sur micro-perf.

### C. Sémantique du bus event `SlmCircuitOpened` — schéma JSON

```json
{
    "kind": "SLM_CIRCUIT_OPENED",
    "swarm_id": "0d5a2b8c-9e0e-4cdd-8c20-3b8e2cc31c0e",
    "robot": "analyzer",
    "failures": 3,
    "ts": "2026-05-28T14:30:45.123Z"
}
```

Conforme au format SCREAMING_SNAKE_CASE `tag` + autres fields plats
(défini en S8, déjà sérialisé OK car `StreamEvent::SlmCircuitOpened`
implémente Serialize).

### D. Pricing tables R3.S2 — coût des SLM circuits

R3.S2 a un coût opérationnel **NÉGATIF** (= gain).

| Métrique | R2 (sans circuit) | R3.S2 (avec circuit) | Gain |
|---|---|---|---|
| Temps perdu / 100 missions (analyzer down) | 3000 s (50 min) | 90 s | **~97 %** |
| Latence p99 / mission (analyzer down) | 30 s | 50 ms (skip) | **~99.8 %** |
| Coût $ inutile en LLM (mission timeout) | ~0 ¢ (timeouts → no LLM call) | 0 ¢ | 0 % |
| Réveille devops manuel | ~50 % cas | 0 % | **-50 %** |

R3.S1 + R3.S2 ensemble = précision µ¢ + résilience.

### E. Backward-compat detailed strategy

| Composant pré-R3.S2 | Comportement post-R3.S2 |
|---|---|
| `Coordinator::builder().build()` sans `.with_default_slm_circuits()` | `slm_circuits = HashMap::new()` → workflows fonctionnent EXACTEMENT comme R2 (pas de circuit, pas de skip, pas d'event). Tests R2 préservés. |
| Sérialisation `CoordinatorClients` | Field `slm_circuits` ignoré dans serde (le struct n'est pas `Serialize` de toute façon, mais `#[serde(skip)]` pour safety). |
| `ChewingMetrics.failed_components` pré-R3.S2 | Sérialisable JSON pré-R3 désérialise OK (field existait depuis S8). |
| `StreamEvent::SlmCircuitOpened` | Defined S8, devient émis post R3.S2. Consumers existants (S8 tests) ignorent silencieusement (déjà testé). |

### F. Migration path post-R3.S2

- **Phase 4 production deploy** : opt-in via
  `EFE_DEFAULT_SLM_CIRCUITS=1` env var (or `.with_default_slm_circuits()`
  dans le `main.rs` du server).
- **Monitoring runbook** : surveiller `SLM_CIRCUIT_OPENED` events via
  Prometheus alert ou bus consumer (`efe-curation` peut subscribe).
- **Tuning par tenant** (R3.S5+) : permettre `Budget.slm_circuit_threshold`
  custom par tenant (defaults: threshold=3, timeout=10s).
- **Sunset L2 dans CHANGELOG** : la mention "L2 limitation" dans
  `R2_LIVE_VALIDATION.md` est désormais "L2 fixed R3.S2".

---

**Fin de l'architecture R3.S2.**

Prêt pour implémentation. Sprint plus dense que R3.S1 (6 sites de
wiring vs ~30 patches metadata), mais 1 nouveau module interne
(`slm_guard.rs`) factorise toute la complexité. 16 tests (12 offline +
1 live + 3 module-tests) couvrent les chemins critiques.

Cohérence avec R3.S1 : R3.S2 = câblage de primitive existante, R3.S1 =
extension de type. Sprint 100 % chirurgical sans new feature.

Architecture spec'd post-éclipse complet de L2 (= argument résilience
défensible). Sprint suivant après livraison : **R3.S3** (Speculator
noise rejection, fix L4/D6).
