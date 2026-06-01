# Audit `swarm/` + `streams/` — Architecture complète, ultra-détaillée

---

**Document** : spécification d'audit chirurgical
**Modules ciblés** : `efe_core::swarm` (1 310 LOC, 7 fichiers) + `efe_core::streams` (556 LOC, 5 fichiers)
**Total scope** : **1 866 LOC**
**Sprint cible** : B (Bench-Foundation pré-requis), 3-4 jours
**Auteur** : audit & design — `rodrigue.pekar@icloud.com`
**Dépend de** : audits Sprint 1-5 (`compute/`), `crates/efe-core/src/swarm/`, `crates/efe-core/src/streams/`
**Consommé par** : Bench Foundation (Sprint Bench), Phase 2 CUDA, Mode 1 server (`efe-server`)
**Statut** : spec gelée pour implémentation — aucune ambiguïté résiduelle

> **Mission** : appliquer aux modules d'orchestration `swarm/` et de
> ressources GPU `streams/` la même rigueur audit-friendly qui a
> fermé Phase 1 du module `compute/`. Les deux modules ont une
> couverture unitaire correcte mais **aucun audit chirurgical** n'a
> jamais cherché : (1) les races/deadlocks sous concurrence, (2) les
> fuites RAII, (3) les transitions d'état machine incomplètes, (4)
> les panics latents, (5) les invariants temporels (timeouts,
> cancellation). Sans cet audit, **un benchmark publié serait sujet
> à variance non-contrôlée** : un deadlock 1/1000 sous charge fait
> exploser le p95.

---

## Table des matières

1. [Mission, justification et critères de sortie](#1-mission-justification-et-critères-de-sortie)
2. [État AS-IS — couverture courante cartographiée](#2-état-as-is--couverture-courante-cartographiée)
3. [État TO-BE — vision cible](#3-état-to-be--vision-cible)
4. [Inventaire chirurgical des zones critiques](#4-inventaire-chirurgical-des-zones-critiques)
5. [Plan d'audit en 9 phases (B1–B9)](#5-plan-daudit-en-9-phases-b1b9)
6. [Stratégie de tests](#6-stratégie-de-tests)
7. [Outils de vérification](#7-outils-de-vérification)
8. [Critères d'acceptation par phase](#8-critères-dacceptation-par-phase)
9. [Plan jour-par-jour (3-4 jours)](#9-plan-jour-par-jour-3-4-jours)
10. [Risques et mitigations](#10-risques-et-mitigations)
11. [Préparation au benchmark](#11-préparation-au-benchmark)
12. [Annexes](#12-annexes)

---

## 1. Mission, justification et critères de sortie

### 1.1 Mission

Délivrer pour `swarm/` + `streams/` :

* Une **suite de tests d'audit** qui couvre les 5 dimensions critiques
  d'un système concurrent : correctness, concurrency, RAII, error
  propagation, timeouts.
* Un **rapport de défauts** identifiés et corrigés (à la manière des
  6 bugs trouvés à l'audit S5).
* Une **base figée** pour le benchmark : sans audit préalable, un
  benchmark publié sur orchestration buggée donnerait des chiffres
  faux ou non-reproductibles.

### 1.2 Justification stratégique (5 raisons)

1. **0 test E2E pour `streams/`**. Il y a des unit tests in-module
   (12 tests), mais aucun fichier `tests/streams_*.rs`. Aucun test
   stress, aucun test de fuite, aucun test de race.

2. **`controller_roundtrip.rs` (490 LOC) couvre `swarm/` mais ne
   stresse pas**. Il mock chaque stratégie et déroule des scénarios
   séquentiels. Aucun test : (a) concurrent `execute()` sur le même
   contrôleur, (b) panic dans une stratégie, (c) deadline pendant
   genesis, (d) clone_handle thread-safety, (e) telemetry finalisée
   en cas de drop précoce.

3. **L'écart compute/ ↔ swarm/streams est démesuré**. Compute a 1 370
   tests, 34 goldens, 42 properties. Swarm+streams ont ~26 tests, 0
   golden, 0 property. Cette asymétrie est un risque architectural
   majeur : la chaîne complète n'est aussi solide que son maillon
   le plus faible.

4. **Le bench end-to-end repose sur le contrôleur**. Si
   `SwarmController::execute` a un deadlock 1/10000, le p99 mesuré
   sera infini ou alterné. Toute conclusion perf serait invalide.

5. **Phase 2 CUDA héritera des bugs**. Les `streams/` sont la voie
   d'accès aux streams CUDA Phase 2. Un bug RAII silencieux qui
   passe en MVP (semaphore = no-op) deviendra un crash CUDA en Phase
   2 (cudaStreamDestroy sur un stream actif).

### 1.3 Critères de sortie

| # | Critère | Mesure |
|---|---------|:------:|
| C1 | Tous les chemins du contrôleur (Pending → Genesis → Fusion → Dissolution → Done **et** Failed) ont au moins un test | Audit transitions par grep + 10 tests dédiés |
| C2 | `streams::StreamPool` passe un test stress de 1000 acquires concurrents sans fuite ni deadlock | `tokio::test` avec 64 workers × 16 itérations, asserts metrics finales |
| C3 | Toute panic dans une stratégie est rattrapée (controller ne crash pas) | 4 panic-tests : `genesis::spawn`, `fusion::run_round`, `dissolver::extract`, `alignment::project` |
| C4 | Toute transition d'état est observée par le `TelemetryGuard` (jamais oubliée) | Property test : 100 random séquences génèrent des transitions monotones |
| C5 | Les budgets (wall-clock, energy) sont enforced à chaque phase + entre rounds | 5 tests : budget=0, budget atteint en genesis/fusion/dissolution, energy growing |
| C6 | RAII : `StreamHandle::drop` réinjecte toujours dans le pool (même sous panic) | Test `panic = abort`-safe avec `std::panic::catch_unwind` |
| C7 | `clone_handle` produit des contrôleurs concurrent-safe | Test 32 workers × 4 requêtes = 128 réponses non-corrompues |
| C8 | `tokio::test` propre : pas de tâche orphan, pas de timer pending | Compteur `tokio::runtime::Handle::current().metrics()` |
| C9 | Aucune `unwrap` / `expect` dans le chemin chaud du contrôleur | grep + revue manuelle (40 sites identifiés) |
| C10 | `cargo test --features stress` ajoute 30+ tests d'audit, tous verts | CI |
| C11 | `clippy --all-targets -- -D warnings` reste à 0 sur les 2 modules | CI |
| C12 | Le **rapport d'audit** documente N défauts trouvés + corrigés | `docs/audits/swarm_streams_audit_report.md` |

---

## 2. État AS-IS — couverture courante cartographiée

### 2.1 `streams/` (556 LOC, 5 fichiers)

| Fichier | LOC | Tests in-module | Couverture | Risques non couverts |
|---------|----:|:---------------:|------------|----------------------|
| `mod.rs` | 21 | 0 | re-exports seulement | n/a |
| `config.rs` | 91 | 3 | defaults_validate, rejects_zero_size, flags_default | manque : size huge (overflow ?), flags exhaustivité |
| `metrics.rs` | 87 | 2 | zero_on_fresh, mean_computed | manque : ordering atomic, race acquire/release/snapshot |
| `handle.rs` | 88 | **0** | aucun ! | RAII drop, double-drop, panic-safety, Send + Sync |
| `pool.rs` | 269 | 7 | acquire, fast_path, try_acquire_fail, synchronize, release, concurrent×8, size | stress 1000+, deadlock, abandoned permits, free_list contention |
| **Total** | **556** | **12** | bonne base unit | **0 stress, 0 RAII negative, 0 cancel** |

### 2.2 `swarm/` (1 310 LOC, 7 fichiers)

| Fichier | LOC | Tests in-module | Couverture | Risques non couverts |
|---------|----:|:---------------:|------------|----------------------|
| `mod.rs` | 52 | 0 | re-exports + doc-test | n/a |
| `arbiter.rs` | 255 | 6 | within_budget, wall_clock, energy_unavailable/available/below, remaining_ms | manque : energy NaN, deadline+budget interaction, drop-during-check |
| `controller.rs` | 448 | **0** | aucun in-module ! | tout le `drive()` (state machine, retries, recovery) |
| `request.rs` | 197 | 3 | for_tests, default_decode_mode, decoded_output_views | manque : trace_context propagation, deadline clamping |
| `state.rs` | 74 | 2 | labels stable, terminal_states | manque : transition graph, no-skip property |
| `strategies.rs` | 216 | 1 | null_telemetry compiles | tous les traits + impls Null |
| `verdict.rs` | 68 | 2 | codes stable, continue_not_terminal | manque : equality semantics |
| **Total** | **1 310** | **14** | maigre pour le contrôleur | **0 panic recovery, 0 stress, 0 RAII telemetry** |

### 2.3 Couverture E2E externe (`tests/`)

| Fichier | Cible | LOC | État |
|---------|-------|----:|------|
| `controller_roundtrip.rs` | swarm/controller | 490 | ✅ scénarios séquentiels (happy path + 7 edge cases) |
| ❌ aucun | streams/ | 0 | **gap critique** |

### 2.4 Synthèse

```
                          UNIT     E2E      STRESS   PROPERTY  FAILURE-INJ
streams/                   12       0         0         0          0
swarm/                     14     ~10         0         0         ~3 (mocks)
─────────────────────────────────────────────────────────────────────────
SCORE GLOBAL              26      10         0         0          3
                                              ↑
                            TROU CRITIQUE pour benchmark fiable
```

**Verdict** : la base unitaire est correcte, mais **l'audit chirurgical
exigé pour publier un benchmark n'existe pas**. C'est exactement le
gap que cet audit comble.

---

## 3. État TO-BE — vision cible

### 3.1 Couverture cible après audit

```
                          UNIT     E2E      STRESS   PROPERTY  FAILURE-INJ
streams/                   12     +5         +6         +4         +3
swarm/                     14     +8         +4         +6         +5
─────────────────────────────────────────────────────────────────────────
TOTAL APRÈS AUDIT          26     ~23       10        10         11
DELTA AUDIT                       +13       +10       +10        +8
```

**+41 nouveaux tests** distribués sur 5 fichiers nouveaux. Total
swarm+streams après audit : ~67 tests.

### 3.2 Invariants vérifiés post-audit

| ID | Invariant | Vérifié par |
|----|-----------|:-----------:|
| **S1** | `StreamPool` ne fuit jamais de stream (toute acquire a un drop matching) | I1 stress test |
| **S2** | `StreamPool` ne deadlock jamais sous N×M acquires concurrents | I1 stress + loom |
| **S3** | `StreamHandle::drop` est panic-safe (ne crash pas si une `drop` user panic) | RAII negative test |
| **S4** | Métriques `active_streams` ≤ `pool.size()` à tout instant | Property test |
| **S5** | `metrics.acquire_count = fast + contended` (invariant arithmétique) | Property test |
| **W1** | Toute transition de `SwarmState` est observée par `TelemetryGuard` | Property test mock telemetry |
| **W2** | Une panic dans une stratégie n'empêche jamais `telemetry.finalize()` | Failure injection × 4 strategies |
| **W3** | `execute()` ne consomme jamais plus que `budget.max_wall_clock_ms` | Property test sur scénarios |
| **W4** | `clone_handle` produit des handles concurrent-safe | Stress test 32 workers |
| **W5** | Mode collapse recovery converge OU émet `EfeError::ModeCollapse` | Property test |
| **W6** | `Verdict::Continue` après `max_rounds` ⟹ `EfeError::ConvergenceTimeout` | Test direct |
| **W7** | `Verdict::Diverging` à round k ⟹ `EfeError::Divergence { rounds: k }` | Test direct |
| **W8** | Alignment delta non-fini ⟹ `EfeError::AlignmentBreach` | Audit-existant `alignment_nan_delta_is_treated_as_breach` |

### 3.3 Patrons d'audit appliqués

Pour chaque fichier audité, on applique 6 axes :

1. **Coverage**: tous les `match` exhaustifs ont un test par bras
2. **Negative**: tous les `Err` paths ont un test
3. **Concurrency**: chaque `Arc<Mutex>`, `Arc<Semaphore>` a un test stress
4. **RAII**: chaque `Drop` a un test happy + panic-safe
5. **Property**: chaque invariant algébrique est testé avec proptest
6. **Failure injection**: chaque `await` peut renvoyer `Err` — test sur tous

---

## 4. Inventaire chirurgical des zones critiques

### 4.1 `streams/` — points d'attention

#### 4.1.1 `pool.rs::StreamPool::acquire` (lignes 90-125)

**Risque 1 — semaphore close interaction** (ligne 117) :

```rust
.acquire_owned().await
.map_err(|_| EfeError::Internal("stream pool semaphore closed"))?;
```

* `Semaphore::close()` n'est appelé nulle part dans le code MVP. Mais
  si Sprint 2 introduit shutdown gracieux, le path `Err` doit déjà
  être testé.
* **Test à ajouter** : `acquire_after_close_returns_internal`

**Risque 2 — `take_stream` retourne `u32::MAX` silencieusement** (ligne 49) :

```rust
guard.pop_front().unwrap_or_else(|| {
    tracing::error!("stream free_list empty while permit granted");
    Stream { id: u32::MAX }
})
```

* Cas "should not happen" → on log et continue avec un id sentinel.
* Mais si jamais cet état arrive, le code downstream qui utilise
  `id() == u32::MAX` ne sait pas distinguer ça d'un id valide.
* **Test à ajouter** : property test stress 1000 cycles, vérifier
  que aucune `id == u32::MAX` n'est jamais retournée.
* **Fix candidat** : utiliser `Option<Stream>` ou `Result` au lieu
  de la sentinel.

**Risque 3 — race entre `take_stream` et `return_stream`** :

* L'ordre des opérations à l'acquire :
  1. `slots.try_acquire_owned()` (semaphore décrément)
  2. `take_stream()` (free_list pop)
* À la release :
  1. `return_stream()` (free_list push)
  2. `_permit` drops → semaphore incrément
* **Question** : entre 1 et 2 du release, un autre thread peut-il
  entrer entre 1 et 2 de l'acquire et trouver une free_list vide ?
* **Réponse** : non, parce que le permit n'est libéré qu'**après**
  `return_stream`. Mais ce raisonnement repose sur l'ordre de drop
  Rust (`stream` avant `_permit`) — voir handle.rs ligne 71-79.
* **Test à ajouter** : `stress_acquire_release_ordering` avec
  vérification que `active_streams` ≤ `pool.size()` à chaque snapshot.

**Risque 4 — `acquire_count` augmenté avant `try_acquire`** (ligne 92-94) :

```rust
self.inner.metrics.acquire_count.fetch_add(1, Ordering::AcqRel);
if let Ok(permit) = self.inner.slots.clone().try_acquire_owned() {
```

* Si try_acquire_owned échoue (saturé), on a quand même incrémenté
  `acquire_count`. Mais on incrémente AUSSI `contended_acquire_count`
  ensuite. Donc l'invariant `acquire_count ≥ fast + contended` tient.
* **Test à ajouter** : property `acquire_count == fast + contended`.

#### 4.1.2 `handle.rs::Drop` (lignes 70-80)

**Risque 5 — double-drop semantics** :

```rust
impl Drop for StreamHandle {
    fn drop(&mut self) {
        if let Some(stream) = self.stream.take() { ... }
    }
}
```

* `self.stream.take()` met l'option à None → un éventuel deuxième
  appel à drop (impossible en safe Rust mais via `mem::forget` puis
  manual...) serait no-op. Bon design.
* **Test à ajouter** : RAII negative — vérifier que `drop` appelé deux
  fois n'incrémente `active_streams` que d'un -1.

**Risque 6 — Weak::upgrade() returns None** (ligne 73) :

```rust
if let Some(pool) = self.pool.upgrade() { ... }
```

* Si le `StreamPool` est drop avant le `StreamHandle`, l'upgrade
  retourne None et on **ne réinjecte pas** le stream. Cela peut
  mener à une fuite logique : le permit, lui, est toujours libéré,
  donc le pool peut accepter de nouvelles acquisitions sur un état
  incohérent.
* **Mais** : si le pool est dropped, ses `inner` est aussi dropped
  → la free_list est dropped → pas de fuite réelle.
* **Test à ajouter** : `handle_outlives_pool_no_panic`.

#### 4.1.3 `metrics.rs` — atomicité de `snapshot`

**Risque 7 — snapshot non-atomique** :

```rust
pub fn snapshot(&self) -> StreamMetricsSnapshot {
    let acquires = self.acquire_count.load(...);
    let wait_ns = self.acquire_wait_ns_total.load(...);
    ...
}
```

* Les loads sont individuellement atomiques mais pas globalement.
  Entre deux loads, un autre thread peut faire avancer les
  compteurs.
* **Conséquence** : `mean_acquire_wait_ns = wait_ns / acquires`
  peut donner un résultat déphasé. Pas catastrophique pour des
  metrics, mais documenter.
* **Action** : ajouter un commentaire `# Atomicity` dans `snapshot()`.

### 4.2 `swarm/` — points d'attention

#### 4.2.1 `controller.rs::drive` (lignes 156-298)

**Risque 8 — telemetry NOT finalized on early return** :

* Ligne 174 : `genesis.spawn().await?` — le `?` propage l'erreur
  en sortant de `drive()`. **Cela évite `telemetry.finalize`** car
  `finalize` est dans `execute()` après `drive()`.
* **Mais** : `inspect_err(|e| telemetry.record_phase_error(...))`
  est appelé avant le `?`. Donc on record l'erreur, mais on ne
  finalise pas avec stats.
* **Question** : `execute()` (ligne 145) appelle `record_phase_error`
  sur le retour Err mais pas `finalize`. Est-ce le contrat voulu ?
* **Test à ajouter** : `error_in_genesis_records_phase_error_no_finalize`,
  vérifier le mock telemetry.

**Risque 9 — collapse_retries reset entre rounds** :

* Ligne 186 : `let mut collapse_retries = 0u32;`
* Ligne 222 : `collapse_retries += 1;`
* Le compteur est local au `for round`, donc il s'incrémente
  globalement sur tous les rounds. **Mais** : si un round produit
  Continue puis Continue puis ModeCollapse, les retries sont
  cumulés. Est-ce le comportement voulu ?
* **Action** : documenter explicitement la sémantique : "retries
  accumulés sur toute l'exécution, pas par round-burst".

**Risque 10 — ELS lost on collapse recovery** :

* Ligne 223 : `self.recover_from_collapse(&mut els, &mut batch, round, policy).await?;`
* Si `inject_noise` succède mais `respawn_diversify` échoue,
  l'ELS a été modifiée mais le batch est partiellement
  inconsistant. `?` propage l'erreur, `els` et `batch` sont
  drop dans le scope. Pas de fuite mais l'utilisateur reçoit
  une erreur sans contexte sur l'état partiel.
* **Action** : envelopper dans un `match` qui restaure ou logue.

**Risque 11 — `final_stats` lost on convergence + then divergence** :

* Si round 1 emit `Converged { c: 0.7 }`, on met `final_stats =
  Some(stats)` et on `break`. OK.
* Mais : on `break` AU MILIEU du `match`. Si le `match` était
  `Converged → break; Diverging → return Err`, on n'atteint pas
  Diverging. OK aussi. Mais une lecture rapide laisse penser que
  `final_stats` peut être écrasé.
* **Action** : documentation de l'invariant "break sort de la boucle,
  pas de re-entrée".

**Risque 12 — `t0.elapsed()` baseline inconsistency with arbiter** :

* Ligne 118 : `let t0 = Instant::now();`
* Ligne 124 : `let arbiter = self.inner.arbiter.new_session(req.spec.budget);`
* `new_session` appelle `Instant::now()` aussi. Donc l'arbiter
  start ≠ t0. Sur une machine lente, cette différence peut être
  ~100µs. Si le budget est 100ms et le test timing-sensitive,
  cela peut causer des flaky tests.
* **Action** : passer `t0` à `new_session` ou utiliser une seule
  référence temporelle.

**Risque 13 — alignment.project peut produire NaN** (ligne 252) :

* Audit existant `audit.rs::alignment_nan_delta_is_treated_as_breach`
  vérifie que NaN delta → AlignmentBreach. Bien.
* **Mais** : si `project` retourne `Err` directement (par ex.
  ComputeError::NumericalInstability), le `?` propage cette erreur
  — pas un AlignmentBreach. Le contrat documenté à ligne 124-129
  de `strategies.rs` dit "implementations should not raise an error
  themselves for exceeding the threshold" — mais d'autres erreurs
  sont autorisées.
* **Action** : test pour les deux variantes (Err vs Ok+NaN).

#### 4.2.2 `controller.rs::recover_from_collapse` (lignes 300-325)

**Risque 14 — `respawn_n` cast to u32 underflow** :

```rust
let respawn_n = ((batch.count() as f32) * policy.respawn_fraction).round() as u32;
```

* Si `respawn_fraction < 0`, `round() as u32` donne 0 (saturating
  cast pour negative).
* Si `respawn_fraction > 1.0`, possible que `respawn_n > batch.count()`
  → UB potentiel dans `respawn_diversify`.
* **Action** : valider `policy.respawn_fraction ∈ [0, 1]` au
  builder ; ajouter un test.

#### 4.2.3 `controller.rs::execute` (lignes 105-151)

**Risque 15 — `tracing::Span::current()` may not be the instrumented span** :

* L'attribut `#[tracing::instrument]` injecte une span à l'entrée,
  mais si le caller a déjà sa propre span, `Span::current()` peut
  pointer vers une span externe.
* **Action** : documenter ou ajouter un `let _e = self.span.enter()`.

#### 4.2.4 `arbiter.rs::ArbiterSession`

**Risque 16 — energy meter NaN** :

* `if now_wh - start > self.budget.max_energy_wh` (ligne 101).
* Si `now_wh - start = NaN`, `NaN > x` est `false`. Donc on
  n'émet pas BudgetExhausted. Mais un meter buggé qui retourne
  NaN devrait être détecté.
* **Action** : ajouter `if !((now_wh - start).is_finite())` →
  log warning + best-effort skip.

**Risque 17 — `start_energy_wh` snapshot at session start** :

* La snapshot est prise à `new_session`. Si le meter cumulatif
  reset entre temps, `now - start` peut être négatif. Ne déclenche
  pas BudgetExhausted, mais comportement non-documenté.
* **Action** : test `meter_resets_to_zero_no_panic`.

#### 4.2.5 `state.rs::SwarmState`

**Risque 18 — Failed never assigned** :

* `SwarmState::Failed` existe dans l'enum, mais dans le contrôleur,
  on ne l'assigne **jamais** explicitement. Les transitions sont
  Pending → Genesis → Fusion → Dissolution → Done. Si une erreur
  survient, on retourne `Err` mais `tracing::Span::current().record`
  n'est jamais mis à jour à Failed.
* **Action** : ajouter un finalizer qui record `state = "failed"`
  sur le path Err.

### 4.3 Synthèse — 18 défauts identifiés

| ID | Sévérité | Module | Fix |
|----|:---:|--------|-----|
| 1 | 🟡 | streams/pool | Test `acquire_after_close` |
| 2 | 🔴 | streams/pool | Replace `u32::MAX` sentinel par `Result` ou `panic!` |
| 3 | 🟡 | streams/pool | Stress test active_streams ≤ size |
| 4 | 🟢 | streams/pool | Property `acquire_count = fast + contended` |
| 5 | 🟢 | streams/handle | Test double-drop |
| 6 | 🟡 | streams/handle | Test handle outlives pool |
| 7 | 🟢 | streams/metrics | Doc snapshot atomicity |
| 8 | 🟡 | swarm/controller | Test telemetry on error path |
| 9 | 🟢 | swarm/controller | Doc collapse_retries semantics |
| 10 | 🟡 | swarm/controller | Wrap recover_from_collapse error |
| 11 | 🟢 | swarm/controller | Doc final_stats invariant |
| 12 | 🟡 | swarm/controller | Unify `t0` and arbiter start |
| 13 | 🟡 | swarm/controller | Test alignment Err vs NaN delta |
| 14 | 🔴 | swarm/controller | Validate respawn_fraction ∈ [0,1] |
| 15 | 🟢 | swarm/controller | Doc tracing::Span::current() |
| 16 | 🟡 | swarm/arbiter | Validate energy is_finite |
| 17 | 🟢 | swarm/arbiter | Test meter resets to zero |
| 18 | 🟡 | swarm/state | Record Failed on Err path |

**Total : 18 défauts**, dont **2 critiques** (🔴), **9 moyens** (🟡),
**7 mineurs** (🟢).

---

## 5. Plan d'audit en 9 phases (B1–B9)

### B1 — Stress tests `streams/` (0.5 jour)

**Fichier** : `tests/streams_stress.rs` (NEW, ~250 LOC)

* `stress_1000_acquires_no_leak` : 1 000 acquire/release sur pool
  de taille 4 → `active_streams` final = 0.
* `stress_concurrent_acquires_under_contention` : 64 workers ×
  16 acquires sur pool de taille 8.
* `stress_acquire_release_metric_invariants` : à chaque snapshot,
  `acquire_count == fast + contended`.
* `stress_handle_drop_on_panic_unwind` : `catch_unwind` un closure
  qui panic après acquire → vérifier que pool refcount cohérent.
* `stress_clone_pool_handles_share_state` : 4 clones, chacun
  acquire/release indépendamment.
* `stress_size_huge_does_not_oom` : pool de taille 10⁶ alloue OK.

**Tests à corriger / fixer** :
* Bug 2 : remplacer `u32::MAX` sentinel — voir B6 pour le fix.

### B2 — RAII negative tests `streams/handle` (0.5 jour)

**Fichier** : `tests/streams_handle_raii.rs` (NEW, ~200 LOC)

* `handle_drop_returns_to_pool` : avant + après drop, snapshot
  diff = -1 active_streams.
* `handle_outlives_pool_no_panic` : keep handle alive, drop pool,
  drop handle → no panic, no leak.
* `handle_double_drop_via_mem_replace_is_safe` : utiliser
  `mem::replace` pour simuler double-drop.
* `handle_id_unique_per_acquire` : 8 acquires successifs, all ids
  distinct.
* `handle_synchronize_increments_metric` : chaque appel +1.

### B3 — Property tests `streams/` (0.5 jour)

**Fichier** : `tests/streams_properties.rs` (NEW, ~150 LOC)

* `prop_acquire_count_is_sum_of_fast_and_contended` : 100 cas
  random workloads.
* `prop_active_streams_bounded_by_pool_size` : à tout instant,
  ≤ size.
* `prop_metrics_monotonic_under_random_workload` : tous les
  compteurs sont monotones non-décroissants.
* `prop_pool_size_eq_initial_after_full_drain` : si tous les
  handles sont dropped, on peut acquire `size` à nouveau.

### B4 — State machine tests `swarm/state.rs` (0.25 jour)

**Fichier** : `tests/swarm_state_machine.rs` (NEW, ~120 LOC)

* `state_transitions_are_monotonic` : Pending → Genesis →
  Fusion → Dissolution → {Done | Failed}, jamais en arrière.
* `state_failed_recorded_on_error` : provoque Err à chaque phase,
  vérifier que `Failed` est recorded.
* `state_done_recorded_on_success` : happy path → Done.
* `state_label_format` : property tous les labels sont
  `[a-z_]+`.

### B5 — Concurrency tests `swarm/controller.rs` (0.5 jour)

**Fichier** : `tests/swarm_concurrency.rs` (NEW, ~250 LOC)

* `clone_handle_is_concurrent_safe` : 32 workers × 4 requêtes,
  toutes les `Response.request_id` correspondent à leurs
  `Request.id`.
* `execute_does_not_share_telemetry_across_requests` : chaque
  appel ouvre son propre guard.
* `concurrent_executes_do_not_deadlock` : 16 requêtes en parallèle,
  toutes terminent avant timeout.
* `panic_in_one_strategy_does_not_kill_others` : 1 stratégie
  panic à round 5, autres requêtes continuent.

### B6 — Failure injection tests `swarm/controller.rs` (0.5 jour)

**Fichier** : `tests/swarm_failure_injection.rs` (NEW, ~300 LOC)

Étendre les `MockGenesis`, `MockFusion`, `MockDissolver`, `MockAligner`
existants dans `tests/mocks.rs` avec des variantes panic / err :

* `genesis_panic_does_not_corrupt_telemetry`
* `fusion_panic_at_round_2_records_phase_error_then_propagates`
* `dissolver_extract_returns_err_propagates_with_record`
* `dissolver_decode_returns_err_propagates`
* `alignment_returns_nan_delta_treated_as_breach` (existe déjà
  dans audit.rs)
* `alignment_returns_err_propagates_not_as_breach`
* `telemetry_panic_in_record_phase_does_not_kill_request`
* `consensus_detector_panic_propagates`

### B7 — Property tests `swarm/` (0.5 jour)

**Fichier** : `tests/swarm_properties.rs` (NEW, ~200 LOC)

* `prop_max_rounds_respected` : pour tout `max_rounds ∈ [1, 32]`,
  le contrôleur exécute ≤ `max_rounds` (sauf collapse retries).
* `prop_telemetry_finalize_called_on_success` : sur 100 happy
  paths random, `RecordingTelemetry::finalize_count == 1`.
* `prop_total_ms_within_budget` : `Response.total_ms ≤
  budget.max_wall_clock_ms` (≤ avec marge 5%).
* `prop_alignment_breach_when_delta_exceeds_max` : random delta
  values, vérifier que > max → AlignmentBreach.
* `prop_collapse_retries_bounded_by_policy` : random traces
  ModeCollapse, vérifier ≤ `max_retries + 1` total tentatives.

### B8 — Application des 18 fixes (1 jour)

Chaque défaut identifié en §4.3 reçoit un patch + son test de
non-régression :

| ID | Patch | Test de non-régression |
|----|-------|------------------------|
| 1 | rien (test seulement) | `streams_acquire_after_close_returns_internal` |
| 2 | `take_stream → Result<Stream, EfeError::Internal>` | `stress_no_sentinel_id_ever_returned` |
| 3 | rien (test seulement) | `prop_active_streams_bounded` |
| 4 | rien (test seulement) | `prop_acquire_count_invariant` |
| 5 | rien (test seulement) | `handle_double_drop_safe` |
| 6 | rien (test seulement) | `handle_outlives_pool` |
| 7 | doc seulement | n/a |
| 8 | rien (test seulement) | `error_path_records_no_finalize` |
| 9 | doc seulement | n/a |
| 10 | wrap recover_from_collapse Err | `collapse_recovery_partial_failure` |
| 11 | doc seulement | n/a |
| 12 | passer `t0` à `new_session` | `t0_consistent_with_arbiter_start` |
| 13 | rien (test seulement) | `alignment_err_distinguished_from_nan_delta` |
| 14 | builder validation | `controller_rejects_respawn_fraction_out_of_range` |
| 15 | doc seulement | n/a |
| 16 | `if !((now-start).is_finite())` | `arbiter_skips_nan_energy_with_warn` |
| 17 | rien (test seulement) | `arbiter_meter_resets_to_zero_no_panic` |
| 18 | record_state on Err path | `state_failed_recorded_on_genesis_err` |

### B9 — Rapport d'audit + clôture (0.25 jour)

**Fichier** : `docs/audits/swarm_streams_audit_report.md` (NEW)

Format identique au pattern Sprint 5 :
* Synthèse résultats (tests +N, defects fixed)
* Détail des 18 défauts (avant/après)
* Métriques de couverture (proxy : test count, edge case count)
* Recommandations Phase 2 CUDA (qu'est-ce qui change quand les
  streams deviennent vrais CUDA streams)

---

## 6. Stratégie de tests

### 6.1 Trois niveaux de tests

| Niveau | Outil | Volume | Granularité |
|--------|-------|:------:|-------------|
| **L1** Unit | `#[test]` in-module | +0 (existant) | scalaire |
| **L2** Integration | `tests/*.rs` | +41 (audit) | fichier × scénario |
| **L3** Property | `proptest::proptest!` | ~10 propriétés × 100 cas = 1 000 cas | invariants |
| **L4** Stress | `tokio::test` + workers | 6 tests × N=1000+ | concurrence |
| **L5** Failure injection | mocks panic | 8 tests | error propagation |

### 6.2 Pattern stress test type

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stress_concurrent_acquires_under_contention() {
    let pool = Arc::new(StreamPool::new(StreamPoolConfig::default().with_size(8)).unwrap());
    let mut handles = Vec::new();
    for worker in 0..64 {
        let p = pool.clone();
        handles.push(tokio::spawn(async move {
            for i in 0..16 {
                let h = p.acquire().await.expect("acquire");
                tokio::time::sleep(std::time::Duration::from_micros(50 + worker as u64 % 5)).await;
                drop(h);
                if i % 4 == 0 {
                    let snapshot = p.metrics();
                    assert!(snapshot.active_streams <= 8,
                        "worker {worker} iter {i}: active={} > size=8",
                        snapshot.active_streams);
                }
            }
        }));
    }
    for h in handles { h.await.unwrap(); }
    let final_snapshot = pool.metrics();
    assert_eq!(final_snapshot.active_streams, 0);
    assert_eq!(final_snapshot.acquire_count,
               final_snapshot.fast_acquire_count + final_snapshot.contended_acquire_count);
}
```

### 6.3 Pattern property test type

```rust
proptest! {
    #[test]
    fn prop_max_rounds_respected(max_rounds in 1u32..32) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let controller = build_test_controller(MockFusion::always_continue(), max_rounds);
            let req = Request::for_tests(spec_with_max_rounds(max_rounds));
            let result = controller.execute(req).await;
            // After max_rounds with all `Continue`, should be ConvergenceTimeout
            prop_assert!(matches!(result.unwrap_err(), EfeError::ConvergenceTimeout { rounds, .. } if rounds == max_rounds));
        });
    }
}
```

### 6.4 Pattern failure injection mock

```rust
struct PanickingFusion {
    panic_at_round: u32,
}

#[async_trait]
impl FusionStrategy for PanickingFusion {
    async fn run_round(&self, _: &mut Els, _: &Batch, round: u32) -> Result<FusionStats> {
        if round == self.panic_at_round {
            panic!("intentional panic at round {round}");
        }
        Ok(FusionStats::synthetic_converging(round, 4))
    }
    async fn inject_noise(&self, _: &mut Els, _: f32) -> Result<()> { Ok(()) }
}
```

---

## 7. Outils de vérification

### 7.1 Outils natifs Rust

| Outil | Usage | Phase |
|-------|-------|-------|
| `cargo test` | Tests fonctionnels | B1-B7 |
| `cargo test --release` | Tests sous optimisations | B1, B5 |
| `cargo clippy --all-targets -- -D warnings` | Lint strict | B8, B9 |
| `cargo miri test` | Détection UB / data races | B5 (optionnel) |
| `loom` | Exhaustive concurrency exploration | B5 (option premium) |

### 7.2 Outils externes (bonus, non bloquants)

| Outil | Usage | Optionnel ? |
|-------|-------|:-----------:|
| `cargo fuzz` | Fuzzing du contrôleur | ✅ Optionnel |
| `valgrind --tool=helgrind` | Race detection | ✅ Optionnel |
| `tokio-console` | Debugging tokio runtime | ✅ Optionnel |
| `flamegraph` | Profiling perf des tests stress | ✅ Optionnel |

### 7.3 Mocks — extension nécessaire

`tests/mocks.rs` (existant) doit être étendu avec :

* `PanickingGenesis`, `PanickingFusion`, `PanickingDissolver`,
  `PanickingAligner`, `PanickingDetector`
* `SlowGenesis(Duration)` pour budget tests
* `EnergyGrowingMeter` (déjà dans arbiter.rs::tests, à factoriser)
* `RecordingTelemetry` (existe — vérifier qu'il enregistre toutes
  les méthodes du trait)

---

## 8. Critères d'acceptation par phase

| Phase | Tests +Δ | LOC tests | Defects résolus | Durée |
|-------|:--------:|:---------:|:---------------:|:-----:|
| B1 | +6 | ~250 | 0 (tests preventifs) | 0.5 j |
| B2 | +5 | ~200 | 0 | 0.5 j |
| B3 | +4 | ~150 | 0 | 0.5 j |
| B4 | +4 | ~120 | 0 | 0.25 j |
| B5 | +4 | ~250 | 0 | 0.5 j |
| B6 | +8 | ~300 | 0 | 0.5 j |
| B7 | +5 | ~200 | 0 | 0.5 j |
| B8 | +10 (régression) | ~150 | **18 fixes** | 1 j |
| B9 | 0 | doc | — | 0.25 j |
| **Total** | **+46** | **~1 620** | **18** | **~4 j** |

---

## 9. Plan jour-par-jour (3-4 jours)

### Jour 1 — Streams/

| Slot | Phase | Livrable |
|------|-------|----------|
| Matin | B1 | `tests/streams_stress.rs` (6 tests stress) |
| Après-midi | B2 + B3 | `tests/streams_handle_raii.rs` + `tests/streams_properties.rs` (9 tests) |

**Fin J1** : streams couvert L4 (stress) + L3 (property) + L2 (RAII).
+15 tests verts.

### Jour 2 — Swarm state + concurrency

| Slot | Phase | Livrable |
|------|-------|----------|
| Matin | B4 + B5 (1/2) | `tests/swarm_state_machine.rs` (4 tests) + start `swarm_concurrency.rs` |
| Après-midi | B5 (2/2) | Finir `swarm_concurrency.rs` (4 tests stress) |

**Fin J2** : +8 tests, controller éprouvé sous concurrence.

### Jour 3 — Failure injection + properties

| Slot | Phase | Livrable |
|------|-------|----------|
| Matin | B6 | `tests/swarm_failure_injection.rs` (8 tests panic / err) + extension `mocks.rs` |
| Après-midi | B7 | `tests/swarm_properties.rs` (5 propriétés × 100 cas) |

**Fin J3** : +13 tests. Couverture L5 (failure) + L3 (property)
complète.

### Jour 4 — Fixes des 18 défauts + clôture

| Slot | Phase | Livrable |
|------|-------|----------|
| Matin | B8 (1/2) | Fix défauts critiques 🔴 (#2, #14) + leurs tests |
| Après-midi | B8 (2/2) + B9 | Fix défauts moyens 🟡 (9 fixes) + rapport d'audit |

**Fin J4** : +46 tests cumulés, **18 défauts corrigés**, rapport
publié.

### Buffer

Le plan inclut un buffer implicite : J1+J2+J3+J4 = 4 jours, mais
les phases B1-B7 cumulent 3.25 jours. **0.75 jour de buffer** pour
absorber les imprévus (typiquement, un défaut critique #2 qui
remonte un autre bug pendant la correction).

---

## 10. Risques et mitigations

| # | Risque | P | Impact | Mitigation |
|---|--------|:-:|:------:|------------|
| R1 | Stress test flaky sous CI lente | Moyen | Moyen | Sleep paramétrable, retries × 3 sur tests stress |
| R2 | `tokio::test` runtime contention sur CI 1 vCPU | Moyen | Faible | `worker_threads = 2` au lieu de 4 |
| R3 | Loom explosion combinatoire si activé | Faible | Faible | Loom optionnel via `--features loom` ; pas en CI standard |
| R4 | Mock panic ne propage pas comme prévu (rust unwind) | Moyen | Élevé | Test sur `panic = unwind` ET `panic = abort` strategies |
| R5 | Fix #2 (sentinel u32::MAX) casse signature publique | Faible | Élevé | Garder `id() -> u32`, faire le fix interne seulement |
| R6 | Property tests trop lents (> 60s) | Moyen | Faible | Limiter `cases = 100` (defaut proptest = 256) |
| R7 | Bug #10 fix (wrap recover) peut introduire double-error | Faible | Moyen | Test dédié `collapse_recovery_partial_failure_no_double_error` |
| R8 | `clone_handle` test révèle bug Send/Sync | Moyen | Élevé | Si trouvé, ouvrir Phase B8.5 d'urgence |

---

## 11. Préparation au benchmark

À l'issue de cet audit, le **bench end-to-end** peut être conçu
sans risque de variance non-contrôlée :

### 11.1 Garanties post-audit pour le benchmark

1. **Pas de deadlock** sous concurrence → p99 borné, pas de mesure
   infinie
2. **Pas de fuite de stream** → memory profile stable sur 10⁶ cycles
3. **Telemetry toujours finalisé** → métriques fiables
4. **Budget enforcé** → SLO `p95 < 2s` mesurable
5. **Recovery déterministe** → reproducibilité bit-stable des runs
6. **Panics rattrapés** → pas de crash mid-benchmark

### 11.2 Métriques bench cibles déblocables après audit

| Métrique bench | Pré-requis audit |
|----------------|------------------|
| `swarm_round_p50/p95/p99` | B5 (concurrent stress sans deadlock) |
| `streams_acquire_throughput` | B1 (stress 1000+ cycles) |
| `controller_request_throughput` | B5 + B6 (panic-safe) |
| `collapse_recovery_overhead` | B6 + B8 #10 |
| `budget_enforcement_overhead` | B8 #12 (`t0` consistency) |

### 11.3 Pont vers Sprint Bench Foundation

Le rapport B9 conclura par une section **« Ready for benchmark »**
qui valide que les 12 critères de §1.3 sont remplis. Sans cette
validation, le `bench Foundation` ne démarre pas.

---

## 12. Annexes

### 12.1 Annexe A — Liste exhaustive des fichiers touchés

**Nouveaux** (4 j de travail) :
```
tests/streams_stress.rs                 NEW [~250 LOC]
tests/streams_handle_raii.rs            NEW [~200 LOC]
tests/streams_properties.rs             NEW [~150 LOC]
tests/swarm_state_machine.rs            NEW [~120 LOC]
tests/swarm_concurrency.rs              NEW [~250 LOC]
tests/swarm_failure_injection.rs        NEW [~300 LOC]
tests/swarm_properties.rs               NEW [~200 LOC]
docs/audits/swarm_streams_audit_report.md NEW [~600 LOC]
```

**Modifiés** (B8 — 18 fixes) :
```
crates/efe-core/src/streams/pool.rs            [#2: ~30 LOC]
crates/efe-core/src/streams/metrics.rs         [#7: doc]
crates/efe-core/src/swarm/controller.rs        [#10, #12, #14, #15: ~50 LOC]
crates/efe-core/src/swarm/arbiter.rs           [#16: ~10 LOC]
crates/efe-core/src/swarm/state.rs             [#18: ~15 LOC]
crates/efe-core/tests/mocks.rs                 [+ panic mocks: ~200 LOC]
```

**Total** : ~2 075 LOC d'audit/tests, ~305 LOC de fixes.

### 12.2 Annexe B — Pattern type d'un test stress

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stress_<scope>_<scenario>() {
    // 1. Setup deterministic
    let pool = ...;
    let workers = 64;
    let iters_per_worker = 16;

    // 2. Spawn workers
    let mut joins = Vec::with_capacity(workers);
    for w in 0..workers {
        let p = pool.clone();
        joins.push(tokio::spawn(async move {
            for i in 0..iters_per_worker {
                // body
                // periodic invariant check
                if i % 4 == 0 { assert_invariant(&p); }
            }
        }));
    }

    // 3. Join all
    for j in joins { j.await.unwrap(); }

    // 4. Final invariants
    let snap = pool.metrics();
    assert_eq!(snap.active_streams, 0);
    assert_eq!(snap.acquire_count, snap.fast + snap.contended);
}
```

### 12.3 Annexe C — Mocks à étendre dans `tests/mocks.rs`

```rust
/// Variant qui panic à un round donné
pub struct PanickingFusion {
    pub panic_at_round: u32,
    pub stats: FusionStats,
}

/// Variant qui retourne Err(EfeError::*) à un round donné
pub struct FailingFusion {
    pub fail_at_round: u32,
    pub error: EfeError,
}

/// Variant qui ralentit délibérément (pour test budget)
pub struct SlowGenesis {
    pub delay: Duration,
    pub spec: SwarmSpec,
}

/// Recording telemetry — vérifie que toutes les phases sont
/// observées exactement une fois.
pub struct StrictRecordingTelemetry {
    pub phases_completed: AtomicU32,
    pub phases_errored: AtomicU32,
    pub finalize_called: AtomicBool,
    pub rounds_recorded: AtomicU32,
    pub events: Mutex<Vec<(u32, String)>>,
}
```

### 12.4 Annexe D — Décisions de design clés

| # | Décision | Alternative rejetée | Raison |
|---|----------|---------------------|--------|
| D1 | Tests dans `tests/*.rs` (intégration) | Tests in-module | Permet stress + tokio::test multi-thread + isolation modules |
| D2 | `proptest` 100 cas (au lieu de 256 default) | Default | CI 60s budget |
| D3 | `loom` optionnel via feature flag | Loom partout | Loom × tokio = lourd ; benefit/coût défavorable pour MVP |
| D4 | Fix #2 (sentinel) en INTERNE seulement | API publique change | Préserve compat ABI |
| D5 | 8 mocks panic au lieu de 1 generic | 1 generic param | Lisibilité du test |
| D6 | Audit publié dans `docs/audits/` | Pas de doc | Doit être greppable comme S5 |
| D7 | 4 jours hardcoded, pas 5 | 5 jours | Buffer 0.75 j déjà inclus |
| D8 | Stress = 1000 cycles, pas 10⁶ | 10⁶ | CI duration ; stress > 1000 en bench, pas en test |

### 12.5 Annexe E — Questions ouvertes (à traiter en B9)

1. Si l'audit de `swarm/` révèle un bug critique dans l'arbitrage
   du budget énergie (qui dépend d'une dépendance externe NVML),
   faut-il ouvrir une Phase B8.5 ou reporter à un Sprint suivant ?
2. Le test `clone_handle_is_concurrent_safe` doit-il vérifier
   l'identité des `Response.swarm_id` (tous différents) ou seulement
   leur unicité ? → décision : unicité (uuid v4 garanti).
3. Faut-il ajouter un benchmark intra-test (`criterion` minimal) à
   l'audit ou réserver tout ça au Sprint Bench Foundation ? →
   décision : RÉSERVER. L'audit produit des **garanties**, pas des
   chiffres.

---

## Conclusion — pourquoi cette architecture

Cette architecture optimise simultanément **trois axes** :

1. **Profondeur** : 41 nouveaux tests sur 4 niveaux (unit, integration,
   property, stress, failure-injection) ferment les 5 dimensions
   critiques d'un système concurrent.

2. **Précision chirurgicale** : 18 défauts identifiés nominativement
   avec patches ciblés. Chacun a son test de non-régression.

3. **Préparation benchmark** : à la fin de l'audit, les 12 critères
   d'acceptation §1.3 sont les **prérequis durs** pour publier un
   benchmark. Sans audit, pas de bench fiable.

À l'issue de ces 3-4 jours, swarm/streams passent de **26 tests
+ 0 stress + 0 property + 0 failure-injection** à **67 tests + 10
stress + 10 property + 8 failure-injection**, avec **18 défauts
corrigés**. La Phase 2 CUDA et le Sprint Bench Foundation peuvent
alors démarrer sur une base saine.

— *Fin du document Audit `swarm/` + `streams/`.*

---

*Pour démarrer l'implémentation, voir le commit suivant qui ouvre
la Phase B1 avec les 6 tests stress de `streams/`.*
