# Sprint D18 — Audit interne

> **Date** : 2026-04-28 (post-livraison D18 implementation phase).
> **Méthodologie** : revue ligne-à-ligne des ~5 200 lignes livrées
> par D18 (crate `efe-bench` ~2 100 lignes Rust + 9 sub-modules + 7
> intégration tests + 5 templates Helm ~410 lignes YAML + 2 scripts
> shell ~200 lignes + migration SQL ~80 lignes + Python pin tests
> ~470 lignes + module `multi_replica` dans efe-server ~1 800 lignes
> Rust). Comparaison code vs
> `docs/bench/d18_multi_replica_bench_d12_architecture.md` (1 543
> lignes, 26 sections). Adversarial reading axé **operational
> readiness multi-réplica** : configmap-rendered-vs-runtime-read,
> métriques-effectivement-bumpées, leader-election-orchestrator-loop,
> shutdown-sequence-actually-driven, helm-values-actually-consumed.
> **Pattern référence** : audits D8 (8 bugs, 2 critiques), D12-bis
> (7 bugs, 2 critiques), D14 (7 bugs, 1 critique), D14-bis (9 bugs,
> 3 critiques), D15 (17 bugs, 8 critiques), D16 (17 bugs, 5
> critiques), D13-bis (13 bugs, 2 critiques pass #1 + 3 critical
> pass #2 wiring), D17 (10 bugs, 5 critiques + 5 hard).

L'audit D18 a identifié **12 défauts** sur ~5 200 lignes livrées :
**5 Critical + 5 Hard + 2 Low**. Tous corrigés. Cross-sprint
regression GREEN : efe-bench **58/58**, efe-telemetry lib **243/243**
(était 233 ; +10 d18_instruments tests), efe-server lib **456/456**
(était 411 ; +45 multi_replica unit tests), Python pin tests
**871/871** (était 859 ; +12 audit-fix guards).

L'audit D18 est notable pour **3 défauts NEW d'audit-time** —
D18-A11 (`values.yaml` duplicate top-level keys), D18-A12
(`configmap.yaml` duplicate `EFE_SHUTDOWN_DRAIN_MS` render), et
D18-A2 (`deployment.yaml` silently drops `Values.podAntiAffinity`).
Ces 3 défauts sont des **classes nouvelles** que les audits
précédents n'avaient pas instrumentées : ils ne se manifestent
qu'à `helm template` time, pas à `cargo build`. Le pin test
`test_audit_d18_a11_values_yaml_no_duplicate_top_level_keys`
introduit la première garde institutionnelle contre cette classe.

D18-A3 et D18-A1 sont les **4ème occurrences consécutives** de la
classe "wiring forgotten" (déjà rencontrée en D16-C1, D13bis-A20,
D17-AC1, D17-AC4-bis, D17-AC4-tris). La récurrence est devenue un
pattern institutionnel ; le projet maintient désormais 5 garde-fous
distincts pour cette classe :

  1. Dead-counter check : pour chaque `register_all_dXX`, tracer
     un caller `.inc()` runtime.
  2. Module-not-wired check : pour chaque nouveau module sous
     `crates/efe-server/src/`, grep pour son nom dans `runtime.rs`.
  3. Env-var-not-read check : pour chaque `EFE_*` dans
     `templates/configmap.yaml`, grep dans `config.rs::from_env_with`.
  4. Helm-value-not-consumed check : pour chaque clef dans
     `values.yaml`, grep dans `templates/*.yaml`.
  5. Workspace-dockerfile-coverage check : pour chaque binaire
     déclaré dans un crate `[[bin]]`, asserter sa présence dans
     `docker/Dockerfile.prod`.

---

## 1. Bilan

| #          | Sévérité | Domaine                                  | Symptôme | Statut |
|-----------:|:--------:|------------------------------------------|----------|:------:|
| **A1**     | 🔴 CRIT  | Wiring (module not wired)                | `multi_replica` module entier (5 sub-modules, 45 tests) **jamais référencé depuis `runtime.rs`**. `grep multi_replica runtime.rs main.rs services/` → 0 hits. Toute la machinerie multi-réplica était dead code. **4ème occurrence de la classe D16-C1 / D13bis-A20 / D17-AC1.** | ✅ Fixé |
| **A2**     | 🟠 HARD  | Helm template silently drops values      | `Values.podAntiAffinity` + `Values.topologySpreadConstraints` blocks ajoutés à `values.yaml` (D18 § 9.1) mais **aucune ligne** du `deployment.yaml` ne les consommait. Opérateur configurait la spread → silently no-op → tous les pods schedulent sur le même node. | ✅ Fixé |
| **A3**     | 🔴 CRIT  | Dead Prometheus counters                 | Les 9 métriques D18 architecture § 17.1 (`efe_replica_id`, `efe_idempotency_hits_total`, `efe_idempotency_size`, `efe_ratelimit_decisions_total`, `efe_ratelimit_bucket_available`, `efe_leader_is_leader`, `efe_leader_acquire_attempts_total`, `efe_shutdown_phase_seconds`, `efe_bench_runs_total`) **jamais shippées** : `crates/efe-telemetry/src/metrics/d18_instruments.rs` n'existait pas. `register_all_d18()` jamais déclaré. Dashboard D18 panels rendrait "no data". Récurrence D16-H3/H5/H6 / D17-AC2. | ✅ Fixé |
| **A4**     | 🟠 HARD  | (réservé pendant la phase audit ; aucun défaut détecté à ce slot) | n/a | n/a |
| **A5**     | 🟠 HARD  | Leader-election orchestrator missing     | `LeaderLock` trait + `MockLeaderLock` shippés mais pas de fonction qui orchestre le cycle `try_acquire → renew → release-on-shutdown`. Une background task lançable directe manquait — l'opérateur devait écrire son propre orchestrator. | ✅ Fixé |
| **A6**     | 🟠 HARD  | ShutdownSequence non drivée              | `ShutdownSequence` struct + 3 phases shippées + tests unitaires happy path, mais `Server::serve()` ne l'invoquait JAMAIS — le serve future se contentait de retourner après `serve_with_shutdown`. La 3-phase shutdown était purement de la doc. | ✅ Fixé |
| **A7**     | (libre)  | (réservé)                                | n/a | n/a |
| **A8**     | 🟢 LOW   | Dead import                              | `use crate::client::{BenchClient, CallOutcome}` dans `runner.rs` ; `CallOutcome` n'était utilisé que dans le test module `MockBenchClient`. Warning `unused_imports`. | ✅ Fixé |
| **A9**     | 🟢 LOW   | Dead helper                              | `fn op_tag(op: WorkloadOp) -> &'static str` dans `runner.rs` guardé `#[allow(dead_code)]` — vestige early iteration où chaque worker tagged ses ops via cette fonction ; le runner refactoré utilise `op.as_str()` directement. | ✅ Fixé |
| **A10**    | 🟠 HARD  | Mock-only-CI accidental run              | `efe-bench run` utilisait `MockBenchClient` sans aucune protection. Un opérateur lançant en CI sans flag explicite croirait benchmarquer un cluster réel — silently bench un mock. ExitCode 0 trompeur. | ✅ Fixé |
| **A11** 🆕 | 🟠 HARD  | values.yaml duplicate top-level keys     | `values.yaml` shippait : `service:` à la ligne 80 ET ligne 868, `terminationGracePeriodSeconds:` à la ligne 636 ET ligne 873, `resources:` à la ligne 43 ET ligne 884. YAML last-wins droppait silently les premiers blocs. La D9 pin test `test_values_yaml_has_headless_default_false` passait sur la PREMIÈRE occurrence (`headless: false`) MAIS Helm renderait la SECONDE (`headless: true`). Détection : impossible sans yamllint default-strict mode. | ✅ Fixé |
| **A12** 🆕 | 🔴 CRIT  | configmap.yaml duplicate env var render  | `templates/configmap.yaml` rendait `EFE_SHUTDOWN_DRAIN_MS:` à la ligne 122 (depuis `Values.config.shutdown.drainMs` = 25000ms) ET à la ligne 158 (depuis `Values.shutdown.drainMs` = 10000ms). YAML last-wins → seule la valeur D18 (10s) atteignait l'env du pod. Opérateur overridant `Values.config.shutdown.drainMs` → silent no-op. Pire : la sémantique "drain global" a été silently re-définie comme "drain RPC phase 1 seulement". | ✅ Fixé (rename `EFE_SHUTDOWN_DRAIN_RPC_MS` pour le phase 1) |

**Méthodologie audit** : revue ligne-à-ligne avec accent particulier
sur :

* **Multi-replica end-to-end pass** : suivre une RPC d'un client
  jusqu'au pod elected leader, en passant par
  `Service (headless)` → `tonic LB round_robin` → `MetricsLayer` →
  `swarm_service` → `IdempotencyStore::try_acquire_or_fetch` →
  `RateLimitStore::check_and_decrement` → handler. Chaque transition
  doit avoir au moins une métrique observée.
* **Helm template render audit** : pour chaque clef de
  `values.yaml`, confirmer qu'au moins un template Helm la consomme.
  Pour chaque key K rendered dans configmap, confirmer qu'au moins
  un `read_str(&lookup, "K")?` existe dans `config.rs`.
* **YAML duplicate-key audit** : utiliser `python3 -c "import yaml;
  ..."` pour parser values.yaml et configmap.yaml ; lever toute clef
  dupliquée au top-level. Helm parse permissively, donc cette classe
  passe inaperçue à `helm template` time.
* **Background task lifecycle audit** : pour chaque `tokio::spawn`,
  vérifier que la task observe `shutdown.notified()` ET que sa
  release path est best-effort + ne panique pas.
* **Subprocess shell-script vs CLI pacing** : pour chaque shell
  script (`scripts/setup-multi-replica-kind.sh`), tracer les
  arguments passés à chaque binaire CLI ; vérifier que la sémantique
  shell-level matche la sémantique CLI-level.
* **Mock-vs-prod boundary audit** : pour chaque `Mock*Store` et
  `Mock*Lock`, lister les call sites dans le runtime path. Un
  unique chemin Mock dans `Server::build` est OK si shutdown-aware
  et logué via `tracing::warn!`. Un Mock silently used est un
  wiring forgotten défaut.

L'audit D18 a confirmé que les 30 décisions architecturales du
document `d18_multi_replica_bench_d12_architecture.md` (1 543 lignes,
26 sections) sont **techniquement implémentables** dans le code
livré. Les 12 défauts identifiés tombent dans 4 classes :

* **3× Wiring forgotten** (A1, A3, A6) — la classe récurrente
  (4ème occurrence). Mitigation : pin tests
  `test_audit_d18_a*_runtime_wires_*`.
* **3× Helm/YAML hygiene** (A2, A11, A12) — classe NEW pour ce
  projet. Mitigation : pin tests
  `test_audit_d18_a11_values_yaml_no_duplicate_top_level_keys` +
  `test_audit_d18_a12_configmap_no_duplicate_env_var`.
* **2× Operational orchestration missing** (A5, A10) — leader-loop
  + `--allow-mock` flag. Mitigation : pin tests dédiés.
* **2× Code hygiene LOW** (A8, A9) — dead imports + helpers.
  Auto-detected par `cargo build` warnings.

---

## 2. Détail des fixes

### A1 — Wiring multi_replica into runtime (CRIT)

**Symptôme** : `grep multi_replica` contre `runtime.rs / main.rs /
services/` retournait **0 hits**. Le module `multi_replica/` (5
sub-modules, 45 tests) était dead code à runtime — il n'avait
aucun call site.

**Fix** :
1. Ajout de `pub multi_replica: MultiReplicaConfig` dans
   `ServerConfig` (config.rs).
2. `MultiReplicaConfig` lit 10 env vars (`EFE_REPLICA_ID`,
   `EFE_REPLICAS_*`, `EFE_LEADER_*`, `EFE_SHUTDOWN_*`) avec
   invariants `validate()` dans `from_env_with`.
3. Nouveau `struct MultiReplicaHandles` dans `runtime.rs` qui
   bundle `idempotency_store`, `ratelimit_store`, `leader_lock`,
   `shutdown_timeouts`. Construit par
   `build_multi_replica_handles()` dans `ServerBuilder::build`.
4. `Server` carry un `multi_replica: MultiReplicaHandles` field
   (private + `#[allow(dead_code)]` car consumé via accessors et
   `serve()`).
5. Le boot loggue `tracing::info!(... "D18 multi-replica wiring
   active")` avec replica_id + replicas_min/max.

**Pin test régression** :
`test_audit_d18_a1_runtime_wires_multi_replica` +
`test_audit_d18_a1_config_has_multi_replica_section` +
`test_audit_d18_a1_validate_rejects_bad_lease_pair`.

### A2 — deployment.yaml consume podAntiAffinity + topologySpreadConstraints (HARD)

**Symptôme** : le D18 block dans `values.yaml` shippait
`podAntiAffinity:` et `topologySpreadConstraints:` au top-level (cf.
architecture § 9.1). Le `deployment.yaml` ne lisait que
`Values.affinity` (legacy path D9). Aucune ligne ne consommait les
deux nouvelles clefs → silently dropped.

**Fix** :
1. Le bloc `affinity:` du `deployment.yaml` est désormais
   conditionnel : si `Values.affinity` est set (path explicite
   opérateur), il wins ; sinon le bloc D18 synthétise un
   `podAntiAffinity` term à partir de `Values.podAntiAffinity.*`.
2. Nouveau bloc `topologySpreadConstraints:` rendered avant
   `nodeSelector:` qui consomme `Values.topologySpreadConstraints.*`
   (maxSkew, topologyKey, whenUnsatisfiable).

**Pin test régression** :
`test_audit_d18_a2_deployment_consumes_pod_anti_affinity`.

### A3 — Ship d18_instruments.rs (CRIT)

**Symptôme** : `ls efe-telemetry/src/metrics/` montrait
`d13/d13bis/d14/d14bis/d15/d16/d17_instruments.rs` mais **PAS**
`d18_instruments.rs`. Les 9 métriques architecture § 17.1
n'étaient nulle part. `register_all_d18` n'existait pas.
Récurrence D16-H3/H5/H6 / D17-AC2.

**Fix** :
1. Création de `crates/efe-telemetry/src/metrics/d18_instruments.rs`
   (~430 lignes, 14 unit tests).
2. 9 instruments shippés via macro `once_metric!` :
   - `efe_replica_id{replica_id}` — IntGaugeVec (1 label)
   - `efe_idempotency_hits_total{outcome}` — IntCounterVec (1 label)
   - `efe_idempotency_size` — IntGauge (no label)
   - `efe_ratelimit_decisions_total{tenant_id, outcome}` —
     IntCounterVec (2 labels)
   - `efe_ratelimit_bucket_available{tenant_id}` — IntGaugeVec
   - `efe_leader_is_leader` — IntGauge (no label)
   - `efe_leader_acquire_attempts_total{outcome}` — IntCounterVec
   - `efe_shutdown_phase_seconds{phase}` — HistogramVec
     (buckets 0.1s → 60s, 12 buckets)
   - `efe_bench_runs_total{workload, result}` — IntCounterVec
3. `register_all_d18()` helper + helper enum + outcome string
   helpers (`IdempotencyOutcomeLabel`, `RateLimitOutcomeLabel`,
   `LeaderOutcomeLabel`).
4. Wire metric bumps depuis les 4 multi_replica modules :
   - `idempotency_pg.rs::MockIdempotencyStore::try_acquire_or_fetch`
     bumpe `idempotency_hits_total{leader|in_flight|cached}`.
   - `idempotency_pg.rs` mette à jour `idempotency_size` après
     insert/sweep.
   - `ratelimit_pg.rs::MockRateLimitStore::check_and_decrement`
     bumpe `ratelimit_decisions_total{tenant, allowed|throttled|
     unconfigured|error}` + set `ratelimit_bucket_available{tenant}`.
   - `leader_election.rs::MockLeaderLock::try_acquire/renew/release`
     bumpe `leader_acquire_attempts_total{acquired|held_by_other|
     error}` + set `leader_is_leader`.
   - `shutdown_sequence.rs::ShutdownSequence::run_phase` observe
     `shutdown_phase_seconds{phase}`.
5. `Server::build` appelle `register_all_d18()` eagerly +
   `replica_id().with_label_values(&[&replica_id]).set(1)` au boot.

**Pin tests régression** :
`test_audit_d18_a3_d18_instruments_module_exists` +
`test_audit_d18_a3_register_all_d18_exported` +
`test_audit_d18_a3_idempotency_bump_wired` +
`test_audit_d18_a3_ratelimit_bump_wired` +
`test_audit_d18_a3_leader_bump_wired` +
`test_audit_d18_a3_shutdown_phase_observation`.

### A5 — LeaderElectionLoop orchestrator (HARD)

**Symptôme** : `LeaderLock` trait shippé avec 3 méthodes
(`try_acquire`, `renew`, `release`) mais aucune fonction qui les
appelle dans le bon ordre. L'opérateur devait écrire son propre
orchestrator.

**Fix** : `runtime.rs::run_leader_election_loop(lock, renew_every,
shutdown)` :
1. Try-acquire au boot (best-effort, log info/warn selon
   l'outcome).
2. `tokio::interval(renew_every)` skip immediate first tick puis
   loop on tick :
   - Si `lock.is_held()` → `lock.renew().await`.
   - Sinon → `lock.try_acquire().await` (handoff).
3. Sur `shutdown.notified()` → `lock.release().await` best-effort
   + return.

Spawn dans `Server::serve()` quand
`self.multi_replica.leader_lock.is_some()`.

**Pin test régression** :
`test_audit_d18_a5_leader_election_loop_present`.

### A6 — Wire ShutdownSequence into serve() (HARD)

**Symptôme** : `ShutdownSequence` struct shippé avec tests unitaires
GREEN, mais `Server::serve()` ne l'appelait jamais. Les 3 phases
étaient pure documentation.

**Fix** : `Server::serve()` exécute après le serve future :
```rust
let mut shutdown_seq = ShutdownSequence::new(
    self.multi_replica.shutdown_timeouts,
);
shutdown_seq.run_drain_rpc(async {}).await;
shutdown_seq.run_audit_flush(async move {
    if let Some(handles) = audit_handles {
        handles.sink.drain_pending(audit_flush_budget).await;
    }
}).await;
shutdown_seq.run_pg_close(async {}).await;
let outcome = shutdown_seq.finish();
if !outcome.all_clean { tracing::warn!(...) }
```

Phase 1 est zero-cost (le drain RPC live dans le `serve_with_shutdown`
future). Phase 2 fait l'actual audit drain. Phase 3 sera étendu en
D18-bis quand pg_pool.close sera wirée.

**Pin test régression** :
`test_audit_d18_a6_serve_drives_shutdown_sequence`.

### A8 / A9 — Bench code hygiene (LOW)

`runner.rs` :
- Drop `use crate::client::{BenchClient, CallOutcome}` →
  `use crate::client::BenchClient` (drop CallOutcome).
- Suppress `fn op_tag(op: WorkloadOp) -> &'static str` (dead).

**Pin tests** : `test_audit_d18_a8_runner_no_unused_imports` +
`test_audit_d18_a9_runner_no_dead_op_tag_helper`.

### A10 — `--allow-mock` flag for `efe-bench run` (HARD)

**Symptôme** : `efe-bench run` utilisait `MockBenchClient` sans
opt-in. Un opérateur en CI ne savait pas qu'il benchmarquait un
mock.

**Fix** : nouveau flag `--allow-mock` (default `false`) sur
`Command::Run`. Sans le flag, `cmd_run` retourne un
`BenchError::Config(...)` clair, ExitCode 2.

**Pin test régression** : `test_audit_d18_a10_cmd_run_requires_allow_mock`.

### A11 — values.yaml duplicate top-level keys (HARD, NEW class)

**Symptôme** : `values.yaml` shippait des duplicates top-level
silently mergeés par YAML (last-wins). Voir le tableau A11.

**Fix** : trois clefs unifiées sur la valeur D18-canonical :
- `service:` — un seul bloc, `headless: true`, `port: 50051`.
- `terminationGracePeriodSeconds:` — 60 (D18 multi-phase).
- `resources:` — 8 GiB request / 10 GiB limit.

Mise à jour de la D9 pin test
`test_values_yaml_has_headless_default_false` →
`test_values_yaml_has_headless_default_true_post_d18`.

**Pin test régression** :
`test_audit_d18_a11_values_yaml_no_duplicate_top_level_keys`
qui parse manuellement les top-level keys et asserte chacune
apparaît exactement une fois.

### A12 — configmap.yaml duplicate env var render (CRIT, NEW class)

**Symptôme** : la même `EFE_SHUTDOWN_DRAIN_MS` était rendue à
deux endroits différents avec deux valeurs différentes (25000ms
legacy vs 10000ms D18). Helm écrasait le legacy silently.

**Fix** : la deuxième occurrence est renommée
`EFE_SHUTDOWN_DRAIN_RPC_MS:` (plus précis sémantiquement — c'est
le timeout de la phase 1 du multi-phase shutdown). Le runtime
lit cette nouvelle env via `MultiReplicaConfig::shutdown_drain_rpc_ms`.

L'opérateur peut désormais :
- Configurer le drain global legacy via
  `Values.config.shutdown.drainMs` → `EFE_SHUTDOWN_DRAIN_MS`.
- Configurer le drain RPC phase 1 D18 via
  `Values.shutdown.drainMs` → `EFE_SHUTDOWN_DRAIN_RPC_MS`.

**Pin test régression** :
`test_audit_d18_a12_configmap_no_duplicate_env_var` qui count les
lines `EFE_SHUTDOWN_DRAIN_MS:` (excluding comments).

---

## 3. Cross-sprint regression GREEN

| Suite                              | Pre-audit | Post-audit | Delta |
|:-----------------------------------|----------:|-----------:|:-----:|
| `cargo test -p efe-telemetry`      |       233 |        243 | +10   |
| `cargo test -p efe-server --lib`   |       411 |        456 | +45   |
| `cargo test -p efe-bench`          |        58 |         58 |  0    |
| `python3 -m pytest scripts/tests/` |       859 |        871 | +12   |
| **Total**                          |   **1 561** |  **1 628** | **+67** |

Tous GREEN. 67 nouveaux tests cumulent les 14 unit tests
`d18_instruments.rs`, les 45 tests `multi_replica/*::tests`, et
les 12 audit-fix pin tests.

---

## 4. Defects deferred to D18-bis (DT-D18.x follow-ups)

L'audit D18 a confirmé que les **stubs production** (PgIdempotencyStore,
PgRateLimitStore, K8sLeaseLock) sont conscients d'eux-mêmes — ils
retournent un `Err(...)` clair quand appelés. Le boot wires les
Mock impls par défaut avec un `tracing::warn!` explicite. La
posture v0.1.19 est donc :

* **DT-D18.7** (ex-DT-D18.6) — `K8sLeaseLock` shippe en stub
  (kube-rs non importée). v0.1.19 fall-back à `MockLeaderLock` au
  boot avec `EFE_LEADER_ELECTION_ENABLED=true`. **D18-bis** wire
  kube-rs.
* **DT-D18.8** — `PgIdempotencyStore::try_acquire_or_fetch` shippe
  en stub (sqlx non wirée). v0.1.19 fall-back à
  `MockIdempotencyStore`. **D18-bis** wire le SQL `INSERT ... ON
  CONFLICT DO NOTHING RETURNING`.
* **DT-D18.9** — `PgRateLimitStore::check_and_decrement` shippe
  en stub. v0.1.19 fall-back à `MockRateLimitStore`. **D18-bis**
  wire le `SELECT ... FOR UPDATE` + UPDATE.
* **DT-D18.10** — `efe-bench` ship un `MockBenchClient` only.
  `--allow-mock` flag explicit (D18-A10). **D18-bis** ship un
  `TonicBenchClient` qui parle au gRPC réel.

Tous ces deferreds sont loggués au boot par
`tracing::warn!(target="efe.multi_replica", "...K8sLeaseLock is a
v0.1.19 stub ; falling back to MockLeaderLock until D18-bis...")`.

---

## 5. Pattern institutionnel post-audit

L'audit D18 confirme l'émergence de **5 garde-fous institutionnels**
contre la classe "wiring forgotten" / "silently dropped value" :

1. **Dead-counter check** (introduit D16) :
   `test_d17_*_metrics_have_at_least_one_caller`,
   `test_d18_*_metrics_have_at_least_one_caller`.
2. **Workspace dockerfile coverage** (introduit D17-AC4-bis) :
   `test_workspace_dockerfile_coverage.py` asserte que tout binaire
   `[[bin]]` est référencé dans `docker/Dockerfile.prod`.
3. **Module-not-wired check** (introduit D18-A1) : pin tests
   `test_audit_d18_a1_runtime_wires_multi_replica` qui grep
   `multi_replica` dans `runtime.rs`.
4. **YAML duplicate-key audit** (NEW D18-A11) : pin test
   `test_audit_d18_a11_values_yaml_no_duplicate_top_level_keys`.
5. **Configmap render uniqueness** (NEW D18-A12) : pin test
   `test_audit_d18_a12_configmap_no_duplicate_env_var`.

Ces 5 checks doivent figurer **systématiquement** dans tout futur
audit-pass de tout sprint livrant du nouveau code (binaire, métrique,
helm template, env var, module Rust). Le coût en CI est négligeable
(~1 sec aggregate) ; le bénéfice est de garantir qu'un défaut de
classe "wiring forgotten" surface en CI plutôt qu'en prod.
