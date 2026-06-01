# M10 — Scheduling & Continuous Learning — Architecture détaillée

> **Statut** : Draft v1.0.0 — Document de référence normatif pour le module
> de planification et d'apprentissage continu de la plateforme EFE.
> **Auteur** : rodrigue.pekar@icloud.com
> **Date** : 2026-04-21
> **Dépendances doc** : `M01_ARCHITECTURE.md` (fondations), `M03_ARCHITECTURE.md`
> (stockage), `M05_ARCHITECTURE.md` (fine-tuning), `M07_ARCHITECTURE.md`
> (rollout), `M08_ARCHITECTURE.md` (gates), `M09_ARCHITECTURE.md` (dérive),
> `MODE1_ARCHITECTURE.md` (classifier par essaim).
> **Périmètre** : l'orchestration temporelle et réactive qui transforme les
> signaux de dérive, les déclencheurs manuels et les horloges en un flux
> cohérent de *jobs* pilotant la boucle d'apprentissage continu.
> **Précision** : chirurgicale, file-par-file, FSM-par-FSM, champ-par-champ.
> **Note** : ce document décrit le **nouveau** M10 dédié au scheduling et à
> l'apprentissage continu. Le `M10_ARCHITECTURE.md` historique (couche gRPC)
> est préservé tel quel et demeure la référence pour le serveur API.

---

## Sommaire

1. [Mission de M10](#1-mission-de-m10)
2. [Positionnement dans l'architecture EFE](#2-positionnement-dans-larchitecture-efe)
3. [Principes directeurs](#3-principes-directeurs)
4. [Invariants structurels](#4-invariants-structurels)
5. [Acteurs et topologie](#5-acteurs-et-topologie)
6. [Vue d'ensemble architecturale](#6-vue-densemble-architecturale)
7. [Sous-système 10.1 — **Scheduler engine**](#7-sous-système-101--scheduler-engine)
8. [Sous-système 10.2 — **Triggers réactifs**](#8-sous-système-102--triggers-réactifs)
9. [Sous-système 10.3 — **Jobs et runs**](#9-sous-système-103--jobs-et-runs)
10. [Sous-système 10.4 — **Retry & dead-letter**](#10-sous-système-104--retry--dead-letter)
11. [Sous-système 10.5 — **Continuous Learning Pipeline**](#11-sous-système-105--continuous-learning-pipeline)
12. [Sous-système 10.6 — **Stockage et persistance**](#12-sous-système-106--stockage-et-persistance)
13. [Sous-système 10.7 — **API REST de contrôle**](#13-sous-système-107--api-rest-de-contrôle)
14. [Sous-système 10.8 — **Observabilité**](#14-sous-système-108--observabilité)
15. [Sous-système 10.9 — **Sécurité et isolation multi-tenant**](#15-sous-système-109--sécurité-et-isolation-multi-tenant)
16. [Budget de performance](#16-budget-de-performance)
17. [Taxonomie d'erreurs](#17-taxonomie-derreurs)
18. [Stratégie de tests](#18-stratégie-de-tests)
19. [Plan de sprints](#19-plan-de-sprints)
20. [Critères Go / No-Go spécifiques à M10](#20-critères-go--no-go-spécifiques-à-m10)
21. **Annexes**
    - A. [FSM complète des schedules](#annexe-a--fsm-complète-des-schedules)
    - B. [FSM complète des jobs et runs](#annexe-b--fsm-complète-des-jobs-et-runs)
    - C. [Grammaire cron étendue](#annexe-c--grammaire-cron-étendue)
    - D. [Domain-separation tags HMAC](#annexe-d--domain-separation-tags-hmac)
    - E. [Schéma tabulaire Postgres](#annexe-e--schéma-tabulaire-postgres)
    - F. [Matrice de risques M10](#annexe-f--matrice-de-risques-m10)
    - G. [Glossaire](#annexe-g--glossaire)

---

## §1. Mission de M10

### §1.1 Énoncé opérationnel

M10 est **le seul composant de la plateforme EFE autorisé à produire un job
d'apprentissage**. Son rôle unique et opposable est :

> *Étant donnés (a) un calendrier déclaratif de tâches récurrentes, (b) un
> flux de signaux réactifs provenant de M09 (dérive) et de M08 (gates), et
> (c) des déclenchements manuels opérateur, produire un flux totalement
> ordonné de `JobSpec` persistés, sans doublons, sans fire manqué au-delà du
> budget de tolérance, et exécuter ces jobs jusqu'à un état terminal explicite.*

Ce mandat se décompose en sept responsabilités non-chevauchantes :

1. **Résolution temporelle** — convertir un `ScheduleSpec` (cron, intervalle,
   RRULE, one-shot) en un flux de `fire_time` exact.
2. **Idempotence de fire** — garantir qu'un même `(schedule_id, fire_time)`
   produit au plus un `JobSpec`, même en présence de redémarrages multiples.
3. **Matérialisation** — instancier un `JobSpec` signé, traçable, routable.
4. **Exécution** — tirer le job depuis une file, l'exécuter dans un worker,
   publier un `JobRun` par tentative.
5. **Retry** — réessayer selon une politique explicite jusqu'à un budget
   terminal, sans débordement (`failed_permanent`, `dead_letter`).
6. **Décision d'apprentissage** — sur la base des signaux M09 / M08 / SLO,
   décider si et quand déclencher baseline refresh, retraining, canary,
   promotion ou rollback.
7. **Observabilité** — émettre des métriques Prometheus, un audit JSONL
   canonique et des événements NATS sur le bus interne.

### §1.2 Ce que M10 **n'est pas**

- **Pas un entraîneur** : M10 déclenche l'entraînement mais délègue
  systématiquement à M05 (`efe_ft`) pour l'exécution effective du fine-tuning.
- **Pas un moteur de dérive** : M10 consomme des `DriftResult` produits par
  M09 ; il n'en génère aucun.
- **Pas un orchestrateur de rollout** : M10 peut déclencher une promotion
  canari, mais la mécanique de trafic split reste dans M07 (`efe_rollout`).
- **Pas un alerting transport** : M10 publie des événements d'état interne ;
  la diffusion vers Slack/Email/Webhook relève de l'alerting historique.
- **Pas une API de tâches arbitraires** : un `JobSpec` doit avoir un
  `JobKind` enregistré statiquement dans le `JobRegistry`. Aucun code
  utilisateur arbitraire ne peut être soumis à l'exécution.

### §1.3 Promesses externes

M10 garantit, contractuellement :

| Promesse                                           | Valeur opposable |
|----------------------------------------------------|------------------|
| Résolution de *fire* à l'échelle seconde           | ± 1 s en régime normal, ± 30 s sous charge saturée |
| Aucun fire manqué sur une fenêtre de 24 h          | Rattrapage (*catch-up*) au redémarrage |
| Idempotence `(schedule_id, fire_time)`             | Déduplication par clé HMAC persistée |
| Propagation d'une trace (`request_id`) bout-en-bout | Dans chaque `JobSpec` et chaque log |
| Terminalité de tout job en ≤ (retry_budget + 1)    | Borné dans le temps et en tentatives |
| Pas de course `(schedule, policy)` parallèle       | Verrou d'exclusion mutuelle par `(tenant, task, policy_kind)` |

---

## §2. Positionnement dans l'architecture EFE

### §2.1 Vue de graphe des dépendances

```
                ┌──────────┐    drift result    ┌───────────┐
                │   M09    │───────────────────▶│  M10      │
                │ Dérive   │                    │ Scheduler │
                └──────────┘                    └────┬──────┘
                                                     │
 wall-clock ────────────────────────────────────────▶│
 operator API ──────────────────────────────────────▶│
 M08 gate fail ─────────────────────────────────────▶│
                                                     │
                                 ┌───────────────────┴─────────────────┐
                                 │                                     │
                          JobSpec ▾                              JobSpec ▾
                       ┌──────────┐                       ┌─────────────┐
                       │  M05     │                       │  M07        │
                       │ Fine-tune│                       │ Rollout     │
                       └──────────┘                       └─────────────┘
                             │                                     │
                       artifact ▾                            canary ▾
                       ┌──────────┐                       ┌─────────────┐
                       │  M03     │                       │  M06        │
                       │ Registry │                       │ Inférence   │
                       └──────────┘                       └─────────────┘
```

M10 est donc un **sommet de flux** : il consomme des signaux en amont (M09,
M08, calendrier, opérateur) et produit des jobs pour l'aval (M05, M07, M03).
Il ne descend jamais en dessous de la couche L3 (services métier) et ne
parle jamais directement aux couches L1/L2 des modules consommateurs.

### §2.2 Couches EFE

| Couche | Présence de M10 | Raison |
|--------|-----------------|--------|
| L1 Fondations | Imports seulement (`efe_platform.clock`, `efe_platform.hashing`, `efe_platform.ids`) | Respect du DAG d'imports |
| L2 Persistance | Implémente un `ScheduleStore`, `JobStore`, `RunStore` SQL async | Aucune dépendance inverse |
| L3 Services | Scheduler engine, triggers, retry, learning policy | Cœur métier |
| L4 Orchestration | `SchedulerDaemon`, `WorkerPool`, `ContinuousLearningController` | Processus long-running |
| L5 API / UI | FastAPI routes `/v1/schedules`, `/v1/jobs`, `/v1/triggers`, `/v1/policies` | Surface publique |

### §2.3 Isolation vis-à-vis des autres modules

M10 **n'a jamais** besoin d'importer :

- ni `efe_drift.detection` (il ne calcule pas de score) ;
- ni `efe_ft.trainer` (il ne pilote pas l'entraînement, il appelle une API) ;
- ni `efe_rollout.controller` (il émet un événement, le controller réagit).

L'unique surface partagée avec M09 est la forme canonique de `DriftResult`
(lecture seule, via `efe_drift.records.DriftResult`). L'unique surface
partagée avec M05 est le contrat d'API `POST /v1/ft/jobs` (gRPC ou REST,
selon l'environnement).

---

## §3. Principes directeurs

### §3.1 Principes opposables en revue PR

1. **P1 — Déclaratif d'abord.** Tout comportement temporel ou réactif est
   décrit par un document `ScheduleSpec` ou `TriggerSpec` persisté. Aucune
   logique d'ordonnancement n'est câblée en code applicatif.
2. **P2 — FSM explicite.** Toute transition d'état (schedule, job, run) est
   listée exhaustivement dans une table `ALLOWED_TRANSITIONS: set[tuple[State,
   State]]`. Toute autre transition lève `InvalidStateTransition`.
3. **P3 — CAS partout.** Toute écriture sur un agrégat (schedule, job, run)
   est conditionnée à un `state_version` attendu ; la mise à jour
   incrémente cette version atomiquement.
4. **P4 — Clocks injectés.** Aucun accès direct à `datetime.now()` ou
   `time.monotonic()` dans les modules métier. Seul `efe_scheduling.clock`
   a le droit d'appeler ces primitives.
5. **P5 — Randomness injectée.** Le jitter, la retry, la stratification
   s'appuient toutes sur un `Randomness` Protocol pour la reproductibilité
   des tests.
6. **P6 — Jobs signés.** Chaque `JobSpec` persisté porte un `hmac_seal`
   vérifiable à la reprise, pour détecter toute mutation silencieuse en
   base.
7. **P7 — Terminalité bornée.** `retry_budget_max = 10`, `retry_duration_max
   = 24 h`. Au-delà, le job bascule en `dead_letter` et cesse toute
   tentative.
8. **P8 — Catch-up limité.** Au redémarrage, le scheduler rattrape au plus
   `catch_up_window_h = 6` h de fires manqués, pour éviter les tempêtes.
9. **P9 — Un seul *writer* par clé.** Un verrou (SQL advisory lock ou Redis
   Redlock) garantit qu'une seule instance du scheduler actif *writes* pour
   une clé `(tenant, task, kind)` à un instant donné.
10. **P10 — Observable par défaut.** Chaque composant métier émet au moins
    une métrique Prometheus et un événement structuré par opération.

### §3.2 Ce que M10 refuse d'implémenter

- **Cron secondes** — le plus petit tick officiel est la **minute**. Un
  besoin sub-minute relève du module d'ingestion, pas du scheduling.
- **Code utilisateur arbitraire** — chaque `JobKind` est enregistré
  statiquement ; pas de `eval`, pas de `pickle`, pas d'injection dynamique.
- **Fire par référence implicite** — un schedule référence toujours un
  `JobKind` par son nom stable, jamais par un callable Python.
- **Reconfiguration à chaud de la FSM** — la liste des transitions est
  figée à la compilation ; un changement requiert une PR et une migration.
- **File d'attente prioritaire gourmande en mémoire** — la priorité est
  portée par un entier borné `{0, 10, 20, 30, 40}` et un champ
  `priority_seq` ; le tri SQL suffit.

---

## §4. Invariants structurels

Ces invariants sont vérifiés par des tests property-based Hypothesis et par
des *contract tests* en CI. Une violation = blocage de merge.

| Code | Nom | Énoncé formel |
|------|-----|---------------|
| **I1** | Unicité de fire | Pour tout `(schedule_id, fire_time)`, `count(jobs) ≤ 1` |
| **I2** | Au-moins-une exécution | Pour tout `job` en état `queued`, il existe une séquence finie de `run`s terminant soit en `succeeded`, soit en `failed_permanent`, soit en `dead_letter` |
| **I3** | Indépendance wall-clock | Un saut NTP de Δt ≤ 24 h ne crée aucun fire fantôme |
| **I4** | Déterminisme policy | Pour des entrées identiques `(state, inputs, cfg)`, la policy renvoie un verdict identique |
| **I5** | FSM whitelist | Toute transition non-listée lève `InvalidStateTransition` |
| **I6** | Domain-tag isolation | Chaque HMAC porte un *domain tag* unique ; aucune collision cross-canal |
| **I7** | Budget de retry monotone | `attempts_left` est strictement décroissant ; jamais restauré |
| **I8** | Monotonicité temporelle | `fire_time[n+1] ≥ fire_time[n]` pour tout schedule actif |
| **I9** | Intégrité du sceau | `verify_hmac(job.spec_json, job.seal) = true` pour tout job persisté |
| **I10** | Isolation tenant | Aucun agrégat n'est lisible ou modifiable sans `tenant_id` prouvé dans le contexte |

---

## §5. Acteurs et topologie

### §5.1 Acteurs logiques

| Acteur | Rôle | Déploiement |
|--------|------|-------------|
| **Opérateur** | Crée / édite / désactive schedules et policies via l'API | Console admin |
| **SchedulerDaemon** | Boucle principale : évalue schedules et triggers, enqueue jobs | K8s `Deployment` (1 réplica actif + N réplicas standby via leader election) |
| **Worker** | Tire un job, l'exécute, publie runs | K8s `Deployment` scalable horizontalement |
| **M09** | Publie des `DriftResult` sur NATS `efe.drift.result` | Consommé par les triggers |
| **M08** | Publie des `GateVerdict` sur NATS `efe.gate.verdict` | Idem |
| **API consumer** | CRUD schedules, triggers manuels | FastAPI derrière gateway |
| **M05 / M07 / M03** | Cibles des jobs (consommateurs aval) | Services downstream |

### §5.2 Topologie physique minimale

```
                ┌─────────────┐
                │ Operator UI │
                └──────┬──────┘
                       │ REST
                       ▼
                ┌─────────────┐
                │ FastAPI     │
                │ (read+CRUD) │
                └──────┬──────┘
                       │
                       ▼
           ┌───────────────────────┐
           │ Postgres (schedules,  │
           │ jobs, runs, policies) │
           └────────┬──────────────┘
                    │
   ┌────────────────┼───────────────┐
   │                │               │
   ▼                ▼               ▼
┌──────────┐  ┌──────────┐   ┌──────────┐
│Scheduler │  │ Worker 1 │   │ Worker N │
│ Daemon   │  │          │   │          │
└────┬─────┘  └────┬─────┘   └────┬─────┘
     │             │              │
     └─────────────┴──────────────┘
                   │
                   ▼
               ┌───────┐
               │ NATS  │  events + subscriptions (drift, gate)
               └───────┘
```

La leader election du SchedulerDaemon est réalisée via `pg_try_advisory_lock(
hash('efe-sched-leader/v1/<tenant>'))`. Un seul leader par tenant à l'instant
présent ; les standbys attendent la libération du verrou (réveil toutes les 5 s).

### §5.3 Modèle de concurrence

| Composant | Nombre d'instances | Partage d'état ? |
|-----------|--------------------|-------------------|
| SchedulerDaemon | 1 leader / tenant | Non — lit/écrit SQL atomique |
| Worker | N par pool | Non — chaque worker tire un job à la fois via `SELECT ... FOR UPDATE SKIP LOCKED` |
| HTTP API | N | Aucun état serveur ; stateless |
| Learning policy | Appel synchrone dans le worker | Pure — pas de partage d'état |

---

## §6. Vue d'ensemble architecturale

### §6.1 Arborescence des sous-modules

```
efe_scheduling/
├── __init__.py                 -- exposure API publique
├── clock.py                    -- Clock Protocol + SystemClock + FakeClock
├── errors.py                   -- taxonomie d'exceptions
├── randomness.py               -- Randomness Protocol + SystemRandom + FakeRandom
├── types.py                    -- dataclasses frozen (ScheduleSpec, JobSpec, JobRun, ...)
├── fsm.py                      -- tables de transitions + validateur CAS
│
├── scheduler/                  -- §7
│   ├── __init__.py
│   ├── cron.py                 -- parser de grammaire cron étendue (minute–year)
│   ├── interval.py             -- schedule à intervalle fixe + jitter optionnel
│   ├── rrule.py                -- adapter RFC 5545 RRULE
│   ├── one_shot.py             -- fire unique à un datetime précis
│   ├── calendar.py             -- composition (union, exclusion, business days)
│   ├── engine.py               -- SchedulerEngine (boucle principale)
│   ├── catch_up.py             -- stratégie de rattrapage au démarrage
│   ├── fire_key.py             -- idempotence HMAC
│   └── leader.py               -- leader election via advisory lock
│
├── triggers/                   -- §8
│   ├── __init__.py
│   ├── base.py                 -- Trigger Protocol + TriggerContext
│   ├── drift.py                -- DriftThresholdTrigger
│   ├── metric.py               -- MetricTrigger (pull-based)
│   ├── manual.py               -- ManualTrigger (API-driven)
│   ├── composite.py            -- AllOf, AnyOf, Not
│   └── debouncer.py            -- anti-flap (hystérésis & suppression)
│
├── jobs/                       -- §9 + §10
│   ├── __init__.py
│   ├── registry.py             -- JobRegistry + JobKind enregistrement
│   ├── queue.py                -- JobQueue Protocol + impl SQL
│   ├── executor.py             -- JobExecutor (dispatch vers handlers)
│   ├── worker.py               -- Worker (boucle pull / execute / ack)
│   ├── retry.py                -- RetryPolicy + backoff (fixed, exp, decorrelated jitter)
│   ├── lease.py                -- gestion des leases (crash recovery)
│   └── dead_letter.py          -- politique de dead-letter (quarantaine)
│
├── learning/                   -- §11
│   ├── __init__.py
│   ├── pipeline.py             -- ContinuousLearningPipeline (orchestrateur)
│   ├── baseline_refresh.py     -- décision de rafraîchissement M09
│   ├── retrain.py              -- décision de relance d'entraînement
│   ├── curator.py              -- curation du dataset d'entraînement
│   ├── training_job.py         -- déclencheur M05 (via API)
│   ├── validation.py           -- évaluation qualité (perplexity, ROUGE, etc.)
│   ├── canary.py               -- suivi de canari
│   ├── promotion.py            -- décision de promote à 100 %
│   └── rollback.py             -- décision de rollback
│
├── storage/                    -- §12
│   ├── __init__.py
│   ├── protocols.py            -- Protocols ScheduleStore, JobStore, RunStore, PolicyStore
│   ├── inmemory.py             -- impl mémoire (tests)
│   ├── sql.py                  -- impl SQLAlchemy 2.0 async
│   └── migrations/             -- scripts Alembic
│       └── 001_init.py
│
├── api/                        -- §13
│   ├── __init__.py
│   ├── schemas.py              -- Pydantic v2 (extra='forbid')
│   ├── http_app.py             -- FastAPI build_app(deps)
│   ├── deps.py                 -- conteneur de dépendances
│   └── errors.py               -- ProblemDetails RFC 7807
│
└── telemetry/                  -- §14
    ├── __init__.py
    ├── metrics.py              -- Prometheus counters / histograms
    ├── events.py               -- bus d'événements internes
    └── audit.py                -- audit JSONL append-only
```

### §6.2 Flux de traitement — séquence nominale

```
                       time ────▶
 T+0   T+60  T+120 T+180 T+240 T+300
  │    │    │    │    │    │
  ▼    ▼    ▼    ▼    ▼    ▼
 tick tick tick tick tick tick   (SchedulerDaemon, toutes les 1 s)
  │     │
  │     └───────────── compute fire_times ∈ (last_tick, now]
  │                     for each ScheduleSpec enabled
  │
  ├─── pour chaque fire_time distincte :
  │     ├─ derive fire_key = HMAC(DOMAIN_TAG_SCHED_FIRE, schedule_id || fire_time)
  │     ├─ INSERT INTO jobs (fire_key, ...) ON CONFLICT DO NOTHING
  │     └─ si insertion réussie → publish NATS efe.sched.job.enqueued
  │
  └─── pour chaque DriftResult consommé sur NATS :
        ├─ match contre TriggerSpec enabled (via tenant, task, method)
        ├─ vérifier debouncer (n breaches consécutives + suppression window)
        └─ si décision ⇒ enqueue JobSpec (fire_key derived from drift_result_id)

 Worker side (continu) :
  │
  ├─ BEGIN; SELECT FROM jobs WHERE lease_expires_at < now
  │    ORDER BY priority, run_at LIMIT 1 FOR UPDATE SKIP LOCKED;
  ├─ UPDATE jobs SET lease_expires_at = now + lease_ttl,
  │                  attempts_left = attempts_left - 1,
  │                  state_version = state_version + 1;
  ├─ COMMIT;
  ├─ call handler(JobSpec) -> JobResult
  ├─ BEGIN; INSERT INTO runs (...); UPDATE jobs SET state = <next>;
  ├─ si erreur retryable ET attempts_left > 0 : state = queued; run_at = now + backoff
  ├─ si erreur non-retryable OU attempts_left = 0 : state = failed_permanent
  ├─ si succès : state = succeeded
  └─ COMMIT; publish NATS efe.sched.run.completed
```

### §6.3 Flux bout-en-bout — cas d'école *Drift-triggered retraining*

1. **T+0** — M09 détecte PSI=0.42 sur `feature=latency_ms`, émet un
   `DriftResult` sur NATS.
2. **T+0.5** — Le `DriftThresholdTrigger` de M10 consomme l'événement, compare
   au seuil (`score ≥ 0.30` et `severity ∈ {major, critical}`), vérifie le
   debouncer (3 breaches consécutives, 30 min depuis dernier fire).
3. **T+0.6** — Décision positive : génère un `JobSpec(kind="retrain_decision",
   tenant, task, window)` avec `fire_key = HMAC(DOMAIN_TAG_SCHED_FIRE,
   drift_result_id || "retrain_decision")`.
4. **T+0.6** — `INSERT INTO jobs ... ON CONFLICT DO NOTHING` — unique grâce
   à `fire_key`.
5. **T+0.7** — Worker tire le job, exécute `RetrainDecision.decide(
   state=CurrentState, inputs=DriftContext, cfg=PolicyConfig)` et reçoit
   `Decision.RETRAIN` avec motif.
6. **T+0.8** — Le handler publie un `JobSpec(kind="training_job")` pour M05 ;
   le job courant passe à `succeeded`.
7. **T+0.9** — Nouveau worker tire le `training_job`, POST vers
   `efe_ft` API ; réponse avec `artifact_draft_id`.
8. **T+X** (quelques minutes plus tard) — M05 termine, publie un
   `ArtifactReady` sur NATS ; le trigger `ArtifactReadyTrigger` enqueue un
   `JobSpec(kind="validation_job")`.
9. … et ainsi de suite jusqu'à la promotion ou au rollback.

---

## §7. Sous-système 10.1 — **Scheduler engine**

### §7.1 Modèle `ScheduleSpec`

Un `ScheduleSpec` est un sum-type (Rust-like) portant un discriminant
`kind ∈ {cron, interval, rrule, one_shot, composite}` et un dictionnaire
kind-spécifique. Chaque variante implémente :

```text
Protocol Schedule:
    id: str                 # UUIDv7
    tenant_id: str
    task_id: str | None     # optionnel : schedule global si None
    job_kind: str           # clé du JobRegistry
    payload: dict[str, Any] # sérialisé en canonical JSON pour le seal
    enabled: bool
    created_at, updated_at: datetime (UTC)

    def iter_fires(
        start: datetime, end: datetime, *, limit: int | None = None,
    ) -> Iterable[datetime]: ...

    def next_fire_after(t: datetime) -> datetime | None: ...
```

### §7.2 Variantes

#### §7.2.1 `CronSchedule`

Grammaire étendue : 6 champs standards (`minute hour dayOfMonth month
dayOfWeek year`) + alias symboliques `@hourly`, `@daily`, `@weekly`,
`@monthly`, `@yearly`. Parsing réalisé par `cron.py` ; représentation
interne en `BitSet` par champ (booléen de possession) pour un `iter_fires`
en O(n) sur la fenêtre.

Champs :

- `expression: str`
- `timezone: ZoneInfo` — par défaut `UTC`. La conversion en UTC pour
  persistance se fait à la sortie de `iter_fires`, jamais à l'entrée.
- `jitter_s: int = 0` — décalage aléatoire maximal (borne supérieure).

#### §7.2.2 `IntervalSchedule`

Schedule périodique. Champs :

- `period_s: int` — ≥ 60.
- `phase_s: int = 0` — offset par rapport à l'époque Unix.
- `jitter_s: int = 0`.

`iter_fires(start, end)` génère `{t | t ≥ start ∧ t ≤ end ∧ (t - phase) mod
period = 0}`. O(1) par fire.

#### §7.2.3 `RRuleSchedule`

Adapter autour de `python-dateutil rrule`. Permet des patterns riches
(« le 2e mardi du mois, sauf jours fériés »). Le parsing est paresseux,
la compilation de la règle est mise en cache par `rrule_text_hash`.

#### §7.2.4 `OneShotSchedule`

Single-fire à `at: datetime`. Après le fire, le schedule passe à l'état
`completed` (voir Annexe A).

#### §7.2.5 `CompositeSchedule`

Union ou exclusion déclarative :

```text
composite = {
  op: "union" | "exclude",
  left: ScheduleSpec,
  right: ScheduleSpec,
}
```

Exemple : `{cron: @daily} exclude {weekdays: sat, sun}`.

### §7.3 `SchedulerEngine`

Boucle principale, pilotée par l'injection d'un `Clock`, un `Randomness`,
un `ScheduleStore`, un `JobStore` et un `LeaderElection`.

#### §7.3.1 Paramètres

| Champ | Type | Défaut | Rôle |
|-------|------|--------|------|
| `tick_period_s` | float | 1.0 | Fréquence d'évaluation |
| `lookahead_s` | int | 60 | Horizon de pré-calcul des fires |
| `max_batch` | int | 500 | Taille max d'un batch enqueue |
| `catch_up_h` | int | 6 | Fenêtre de rattrapage au démarrage |
| `leader_ttl_s` | int | 30 | TTL du lock leader (renouvelé à chaque tick) |

#### §7.3.2 Pseudo-algorithme d'un tick

```
fn tick():
    if not leader: return
    lease = now()
    schedules = store.list_enabled(tenant, last_tick, now)
    for s in schedules:
        for fire_time in s.iter_fires(s.last_evaluated_at, now, limit=max_batch):
            key = HMAC(DOMAIN_TAG_SCHED_FIRE, s.id || iso(fire_time))
            spec = JobSpec.from_schedule(s, fire_time, key)
            try:
                job_store.enqueue(spec, on_conflict=do_nothing)
                metrics.inc("sched_fire_total", tenant, s.job_kind)
            except DuplicateFireKey:
                metrics.inc("sched_fire_duplicate_total", tenant, s.job_kind)
        store.mark_evaluated(s.id, until=now, expected_version=s.state_version)
    self.last_tick = now
    leader.renew(leader_ttl_s)
```

#### §7.3.3 Stratégie de rattrapage

Au démarrage, si `now - last_tick > catch_up_h * 3600`, le rattrapage est
**limité** à `catch_up_h` heures avant *now*. Les fires antérieurs sont
écartés (métriques `sched_fire_dropped_total`) et un event d'audit
`sched.catchup.truncated` est émis.

Ce choix prévient l'effet *thundering herd* à la remise en service d'une
flotte de schedulers après une panne longue.

### §7.4 Leader election

Le verrou est obtenu via `pg_try_advisory_lock(hashtext('efe-sched-leader/' ||
tenant_id))`. Le renouvellement est implicite (la session tient le lock
tant qu'elle vit). Un heartbeat SQL (`SELECT 1`) toutes les 5 s détecte la
rupture réseau ; à défaut, le daemon se met en mode *standby* et
abandonne toutes ses opérations en cours.

### §7.5 Drift de temps et DST

- Toutes les dates persistées sont en UTC.
- L'évaluation d'un `CronSchedule` dans un fuseau local est **explicite** :
  le champ `timezone` est obligatoire si l'expression dépend de l'heure
  locale (ex. `30 3 * * *` en `Europe/Paris` tolère les transitions
  été/hiver correctement via `pytz`).
- Aux transitions DST :
  - *Spring-forward* (heure sautée) : les fires tombant dans le trou sont
    **reportés** à la première minute valide après le saut, sauf si
    `skip_missing: true` dans le schedule.
  - *Fall-back* (heure répétée) : la convention `prefer: "first"`
    (défaut) fire une seule fois ; `prefer: "both"` fire les deux.

---

## §8. Sous-système 10.2 — **Triggers réactifs**

### §8.1 Contrat commun

Tous les triggers implémentent :

```text
Protocol Trigger:
    id: str
    tenant_id: str
    task_id: str | None
    job_kind: str
    enabled: bool

    def should_fire(ctx: TriggerContext) -> Decision: ...
    # Decision = Fire(payload: dict) | Hold | Reset
```

Un `TriggerContext` porte :

- Le signal entrant (DriftResult, GateVerdict, MetricSample, ...).
- L'état du debouncer associé (`consecutive_count`, `last_fired_at`,
  `last_seen_state`).
- Le `Clock` et le `Randomness` injectés.

### §8.2 Variantes

#### §8.2.1 `DriftThresholdTrigger`

Fire si :

- `DriftResult.feature_name == match.feature` (ou glob `*`),
- `DriftResult.method == match.method`,
- `DriftResult.score ≥ match.threshold` (avec orientation du détecteur),
- `consecutive_breaches ≥ N` (hystérésis, défaut 3),
- `now - last_fired_at ≥ suppression_s` (défaut 1800 s).

Paramètres configurables via `DriftTriggerSpec` persisté.

#### §8.2.2 `MetricTrigger`

Tire un échantillon de Prometheus (`query: promql`, `threshold`, `op: {<,
≤, ≥, >}`). Pollé par le scheduler toutes les `poll_s` secondes (par
défaut 60 s) ; fire si la condition est vraie et que le debouncer
l'autorise.

#### §8.2.3 `ManualTrigger`

Déclenché par un appel REST `POST /v1/triggers/:id/fire`. Doit porter un
`idempotency_key` fourni par l'appelant ; la clé est persistée 24 h pour
bloquer les doublons.

#### §8.2.4 Composition

`AllOfTrigger(children: list[Trigger])` — fire ssi tous les enfants
fireraient dans la même fenêtre de temps `window_s`. `AnyOfTrigger` — fire
si au moins un. `NotTrigger` — fire si l'enfant ne firerait pas.

### §8.3 Debouncer

Le debouncer est une simple machine d'états par clé
`(trigger_id, feature_id)` :

```
       ┌─────┐  breach   ┌─────────┐  breach   ┌──────┐  breach   ┌──────┐
       │ idle│──────────▶│count=1  │──────────▶│count2│──────────▶│ fire │
       └─────┘           └─────────┘           └──────┘           └──────┘
         ▲                   │                     │                  │
         │ normal            │ normal              │ normal           │ suppression
         └───────────────────┴─────────────────────┴──────────────────┘
```

Le `suppression_s` est appliqué à la sortie de l'état `fire` : le debouncer
ignore les breaches pendant ce délai avant de retomber en `idle`.

---

## §9. Sous-système 10.3 — **Jobs et runs**

### §9.1 Modèle `JobSpec`

Un `JobSpec` est un dataclass frozen (immuable) portant :

| Champ | Type | Contrainte |
|-------|------|------------|
| `job_id` | `str` (UUIDv7) | Monotone croissant |
| `fire_key` | `bytes(32)` | Unique (contrainte SQL) |
| `tenant_id` | `str` | Obligatoire |
| `task_id` | `str \| None` | Optionnel |
| `kind` | `JobKind` enum | Doit être enregistré dans le `JobRegistry` |
| `payload` | `dict[str, Any]` | Sérialisé en canonical JSON |
| `priority` | `JobPriority` enum | `{lowest=0, low=10, normal=20, high=30, critical=40}` |
| `priority_seq` | `int` | UUIDv7 du job, pour tie-break FIFO |
| `run_at` | `datetime (UTC)` | Earliest eligible time |
| `deadline` | `datetime \| None` | Hard cutoff ; si dépassé → `failed_permanent` |
| `max_attempts` | `int` (1..10) | Définit le retry budget initial |
| `retry_policy` | `RetryPolicy` (enum) | `{fixed, exponential, decorrelated}` |
| `retry_base_ms` | `int` | Paramètre du backoff |
| `retry_max_ms` | `int` | Plafond |
| `hmac_seal` | `bytes(32)` | `HMAC(DOMAIN_TAG_JOB_SEAL, canonical_json(spec - seal))` |
| `origin` | `enum` | `{schedule, trigger, manual, chained}` |
| `origin_ref` | `str` | Identifiant de la source (schedule_id, trigger_id, ...) |
| `created_at` | `datetime` | UTC |

### §9.2 Modèle `JobRun`

Un `JobRun` est une tentative d'exécution (1..N par job) :

| Champ | Type | Description |
|-------|------|-------------|
| `run_id` | `str` (UUIDv7) | |
| `job_id` | `str` | FK vers `JobSpec` |
| `attempt_idx` | `int` | 1-based |
| `worker_id` | `str` | Identité du worker ayant tiré le job |
| `started_at` | `datetime` | UTC |
| `finished_at` | `datetime \| None` | UTC ; null si en cours |
| `status` | `JobRunStatus` | `{running, succeeded, failed, cancelled, timeout}` |
| `error_code` | `str \| None` | Code machine (aligné errors.py) |
| `error_message` | `str \| None` | Message humain tronqué à 1 kB |
| `output_digest` | `bytes(32) \| None` | `HMAC` de l'output pour audit |
| `telemetry` | `dict[str, float]` | `{cpu_s, mem_mb, io_mb, ...}` |

### §9.3 `JobRegistry`

Registre statique des `JobKind` supportés. Un `JobKind` enregistre :

- Un nom stable (`"baseline_refresh"`, `"retrain_decision"`,
  `"training_job"`, `"validation_job"`, `"canary_evaluate"`,
  `"promotion_decision"`, `"rollback"`, ...).
- Un handler `async def handle(spec: JobSpec, deps: Deps) -> JobResult`.
- Un schéma Pydantic du `payload` (validation à l'enqueue, pas au run).
- Un `default_retry_policy` et un `default_priority`.
- Une annotation `idempotent: bool` (les jobs non-idempotents voient
  leur `max_attempts` plafonné à 1).

L'enregistrement est effectué via un décorateur `@job_kind("<name>",
schema=...)` à l'import du module handler. L'import de tous les handlers
se fait à l'initialisation du service via `load_handlers(plugins_dir)`.

### §9.4 FSM `JobState`

Voir Annexe B pour le diagramme complet. En bref :

```
 queued ──pick──▶ running ──ok──▶ succeeded (terminal)
   │                │
   │                └──err(retryable)──▶ queued (run_at = now + backoff)
   │                │
   │                └──err(non-retryable)──▶ failed_permanent (terminal)
   │                │
   │                └──budget_exhausted──▶ failed_permanent (terminal)
   │                │
   │                └──timeout──▶ queued (attempts_left -= 1) ou failed_permanent
   │
   └──cancel──▶ cancelled (terminal)
   └──ttl_expired──▶ dead_letter (terminal)
```

### §9.5 `JobExecutor` et `Worker`

Le **Worker** exécute la boucle suivante en permanence :

1. `SELECT ... FOR UPDATE SKIP LOCKED` le prochain job éligible.
2. Prend un lease (`lease_expires_at = now + lease_ttl`) et décrémente
   `attempts_left`.
3. Appelle `JobExecutor.execute(spec)` qui :
   - Résout le handler via `JobRegistry.get(spec.kind)`.
   - Valide le payload contre le schéma Pydantic.
   - Lance le handler dans une tâche `asyncio` avec `wait_for(deadline)`.
   - Collecte l'exception éventuelle.
4. Selon le résultat, applique la transition FSM adéquate (succeeded /
   failed_retryable / failed_permanent / timeout).
5. Publie un `JobRun` correspondant.
6. `COMMIT`.

Le lease est renouvelé périodiquement par un heartbeat (toutes les
`lease_ttl / 3` secondes) pour couvrir les handlers longs.

### §9.6 Crash recovery

Au démarrage d'un worker, un balayage `SELECT ... WHERE lease_expires_at <
now AND state = 'running'` récupère les jobs orphelins. Leur `attempts_left`
n'est pas décrémenté (la tentative est considérée comme non consommée) ;
`state` repasse à `queued` avec `run_at = now + lease_recovery_backoff`.

---

## §10. Sous-système 10.4 — **Retry & dead-letter**

### §10.1 Politiques disponibles

| Nom | Formule | Usage |
|-----|---------|-------|
| `fixed` | `base_ms` | Rare, cas où l'intervalle est métier |
| `exponential` | `base_ms * 2^(attempt-1)` plafonné à `max_ms` | Default pour I/O |
| `decorrelated_jitter` | `rand(base_ms, min(max_ms, prev * 3))` | Recommandé sous forte contention (évite le *herd*) |
| `linear` | `base_ms * attempt` | Peu utilisé, mais supporté |

Jitter additionnel optionnel : `jitter_ms ∈ [0, ±rate * delay]`.

### §10.2 Budget de retry

Le budget est double :

- **Attempts** : `max_attempts` ≤ 10.
- **Wall-clock** : `retry_wall_budget_s` ≤ 86 400 (24 h).

Dépassement de l'un ou l'autre ⇒ état `failed_permanent`.

### §10.3 Dead-letter

Un job en `failed_permanent` est copié dans la table `dead_letters` avec
le dernier `JobRun` en référence. Une métrique `sched_dead_letter_total`
labelisée par `(tenant, kind)` alerte au-delà d'un seuil
(`dead_letter_burst_1h > 5` → alerte critique).

Un endpoint `POST /v1/jobs/:id/requeue` permet à un opérateur de requeuer
manuellement un job dead-letter (avec `attempts_left = reset`), tracé dans
l'audit (`sched.dead_letter.requeue` + `actor`).

---

## §11. Sous-système 10.5 — **Continuous Learning Pipeline**

### §11.1 Vue d'ensemble

La pipeline est composée de **six décisions pures** plus **trois jobs
d'exécution** :

```
                      ┌────────────────────┐
            ┌────────▶│ BaselineRefresh    │─── job: baseline_refresh
            │         └────────────────────┘
            │                  │
 schedule ──┤                  ▼
 @daily     │         ┌────────────────────┐
            ├────────▶│ RetrainDecision    │── decide → noop / retrain
            │         └────────────────────┘
 drift ─────┤                  │ retrain
            │                  ▼
            │         ┌────────────────────┐
            ├────────▶│ DataCurator        │─── job: data_curation
            │         └────────────────────┘
            │                  │ dataset_id
            │                  ▼
            │         ┌────────────────────┐
            └────────▶│ TrainingJob (M05)  │─── job: training_job
                      └────────────────────┘
                               │ artifact_id
                               ▼
                      ┌────────────────────┐
                      │ ValidationJob      │─── job: validation_job
                      └────────────────────┘
                               │ val_report
                               ▼
                      ┌────────────────────┐
                      │ Canary (M07)       │─── job: canary_evaluate
                      └────────────────────┘
                               │ canary_report
                 ┌─────────────┼──────────────┐
                 ▼             ▼              ▼
        ┌────────────┐ ┌────────────┐ ┌────────────┐
        │ Promotion  │ │ Hold       │ │ Rollback   │
        └────────────┘ └────────────┘ └────────────┘
```

Chaque décision est **pure** : `decide(state, inputs, cfg) -> Decision`,
testable isolément par table de vérité. Les *jobs d'exécution* (baseline
refresh, data curation, training, validation, canary) sont des effets de
bord orchestrés via les JobKinds du registre.

### §11.2 `BaselineRefresh`

Rafraîchit périodiquement les baselines M09 :

- Déclencheur : `CronSchedule("@daily 03:15 UTC")` par défaut + trigger
  `drift severity=critical`.
- Décision : `{refresh, skip_inactive, skip_recently_refreshed}`.
- Job : calcule un nouveau snapshot, le signe via HMAC M09, le promote
  en *active* si la *sanity check* passe (sample_count ≥ min_samples,
  variance_ratio ≤ threshold).
- Idempotence : `fire_key = HMAC(DOMAIN_TAG_SCHED_FIRE, feature_id ||
  window_start || window_end)`.

### §11.3 `RetrainDecision`

Stateful mais déterministe. Input :

- `drift_summary: dict[feature, latest_score_by_method]`
- `last_retrain_at: datetime | None`
- `cost_last_7d: float`
- `accuracy_delta_last_7d: float`
- `cfg: PolicyConfig`

Output :

```
enum Decision:
  - retrain(dataset_hint: DatasetHint, reason: str)
  - hold(reason: str)
  - freeze(reason: str)  # suspension temporaire (cool-down post-incident)
```

Règles :

1. **Freeze** si `cool_down_until > now` (initié par un rollback récent).
2. **Retrain** si :
   - `max_severity = critical` sur au moins N features distinctes
     (seuil par défaut N=3), **ou**
   - `retrain_overdue` (`now - last_retrain_at > cfg.max_stale_d`), **ou**
   - `accuracy_delta < cfg.accuracy_delta_alarm` et
     `dataset_coverage_ok = true`.
3. **Hold** sinon.

### §11.4 `DataCurator`

Construit un dataset d'entraînement à partir des événements ingérés :

- Fenêtre : `cfg.training_window_d` (défaut 30 j).
- Stratification : équilibrage par `task_id` et `label` si supervision.
- Déduplication : par `hash(prompt_text)` + `hash(response_text)`.
- PII scrub : ré-application du linter M04 (NFKC + depth-limit).
- Shuffle déterministe avec `seed = HMAC(DOMAIN_TAG_DATASET_SEED,
  training_cycle_id)`.
- Sortie : un `DatasetManifest` signé (`HMAC(DOMAIN_TAG_DATASET_SEAL, ...)`).

### §11.5 `TrainingJob`

Délègue à M05 via `POST /v1/ft/jobs`. Le handler M10 :

1. Lit `DatasetManifest`.
2. Construit `TrainingConfig` (hyperparamètres depuis `PolicyConfig`).
3. POST ; récupère `artifact_draft_id`.
4. Poll M05 toutes les 30 s jusqu'à `state ∈ {ready, failed}`, ou
   timeout `cfg.training_timeout_h`.
5. Si `ready` → enqueue `validation_job`. Si `failed` → back-off ; une
   deuxième tentative est autorisée si la cause est transitoire
   (OOM, preemption).

### §11.6 `ValidationJob`

Évalue l'artefact contre un *holdout* stratifié et un *golden dataset* :

- Métriques génériques : perplexity, ROUGE-L, BLEU, exact-match.
- Métriques métier par tâche (registrées via `MetricRegistry`).
- Seuils d'acceptation définis par `cfg.validation_thresholds`.
- Sortie : `ValidationReport` signé.

### §11.7 `CanaryEvaluate`

Délègue à M07 pour le trafic split. Le handler M10 :

1. POST `/v1/rollout/canary` avec `artifact_id` et `split_ppm = 10_000`.
2. Poll toutes les 5 min pendant `cfg.canary_observation_h` (défaut 2 h).
3. Collecte : `latency_p99_delta`, `error_rate_delta`, `drift_score_delta`.
4. Sortie : `CanaryReport`.

### §11.8 `PromotionDecision`

Pure function. Input :

- `CanaryReport` (métriques delta).
- `ValidationReport`.
- `GateVerdicts` (M08).
- `cfg.promotion_thresholds`.

Output :

```
enum Decision:
  - promote(to_split_ppm: int = 1_000_000)  # 100%
  - hold(reason: str)
  - rollback(reason: str, rollback_to: ArtifactId)
```

Règles :

- `rollback` si `error_rate_delta > cfg.max_error_rate_delta` ou
  `latency_p99_delta > cfg.max_latency_delta`.
- `hold` si `gate_verdict.any(major)` ou `canary_coverage < cfg.min_coverage`.
- `promote` sinon et si `canary_duration ≥ cfg.canary_observation_h`.

### §11.9 `RollbackDecision`

Réactive sur alarme runtime (pas seulement canary) : métriques live
dépassent les seuils configurés → rollback immédiat à la dernière
stable. Le job `rollback` réécrit le split M07 à 0 % canary + enqueue un
`PromotionFreeze` (cool-down 24 h).

### §11.10 Orchestration (`ContinuousLearningPipeline`)

Classe orchestratrice qui implémente :

- `async def on_drift(result: DriftResult) -> list[JobSpec]`
- `async def on_gate_verdict(v: GateVerdict) -> list[JobSpec]`
- `async def on_scheduled_tick(now: datetime) -> list[JobSpec]`
- `async def on_job_succeeded(run: JobRun) -> list[JobSpec]`
  — chaînage : un `training_job` succeeded enqueue un `validation_job`,
  etc.

Cette classe est la **glue** ; elle n'impose aucune logique de décision —
elle délègue aux six décisions pures décrites ci-dessus.

---

## §12. Sous-système 10.6 — **Stockage et persistance**

### §12.1 Schéma SQL

Quatre tables principales (voir Annexe E pour DDL complet) :

- `schedules(schedule_id, tenant_id, task_id, job_kind, spec_json, enabled,
  state_version, last_evaluated_at, created_at, updated_at)`
- `triggers(trigger_id, tenant_id, task_id, job_kind, spec_json, enabled,
  state_version, debouncer_state, last_fired_at, created_at, updated_at)`
- `jobs(job_id, fire_key UNIQUE, tenant_id, task_id, kind, payload_json,
  priority, priority_seq, run_at, deadline, state, state_version,
  attempts_left, lease_expires_at, hmac_seal, origin, origin_ref,
  created_at)`
- `runs(run_id, job_id, attempt_idx, worker_id, started_at, finished_at,
  status, error_code, error_message, output_digest, telemetry_json)`
- `dead_letters(job_id, moved_at, last_run_id, reason)`
- `policy_configs(policy_id, tenant_id, task_id, kind, cfg_json,
  state_version, enabled_at, disabled_at)`

Index :

- `jobs(state, priority DESC, run_at)` partiel sur
  `state IN ('queued','running')` — feed des workers.
- `jobs(fire_key)` unique — idempotence I1.
- `schedules(tenant_id, enabled)` partiel — feed du scheduler.
- `runs(job_id, started_at)` — audit.

### §12.2 Protocols d'accès

```text
ScheduleStore:
    async def create(spec: ScheduleSpec) -> Schedule
    async def get(schedule_id: str) -> Schedule
    async def list_enabled(tenant, evaluated_before: datetime) -> list[Schedule]
    async def mark_evaluated(schedule_id, until: datetime, expected_version: int) -> int
    async def transition(schedule_id, from_state, to_state, expected_version) -> int
    async def disable(schedule_id, expected_version) -> int

JobStore:
    async def enqueue(spec: JobSpec) -> Job     # ON CONFLICT DO NOTHING
    async def lease_next(worker_id, tenant=None) -> Job | None
    async def extend_lease(job_id, new_ttl: timedelta, expected_version: int) -> int
    async def transition(job_id, from_state, to_state, expected_version, fields: dict) -> int
    async def list_eligible(tenant, limit: int) -> list[Job]

RunStore:
    async def append(run: JobRun) -> None
    async def list_for_job(job_id) -> list[JobRun]

PolicyStore:
    async def get_active(tenant, task, kind) -> PolicyConfig | None
    async def upsert(cfg: PolicyConfig, expected_version: int) -> PolicyConfig
```

Tous les verbes mutants acceptent `expected_version` → CAS (I7).

### §12.3 Impl SQL (SQLAlchemy 2.0 async)

- `ORM = registry(mapper=Mapped)` ; tables déclaratives.
- Transactions : `async with session.begin(): ...`.
- `SELECT ... FOR UPDATE SKIP LOCKED` pour `lease_next` (Postgres
  ≥ 9.5 requis).
- Migrations Alembic (`alembic revision --autogenerate`).

### §12.4 Impl in-memory

Pour les tests : dict-backed store reproduisant la sémantique CAS,
`SKIP LOCKED` simulé via un mutex par ligne. Utilisé par tous les tests
unitaires de la couche scheduler / worker / learning.

---

## §13. Sous-système 10.7 — **API REST de contrôle**

### §13.1 Routes publiques

| Méthode | Chemin | Description |
|---------|--------|-------------|
| `POST`  | `/v1/schedules` | Créer un schedule |
| `GET`   | `/v1/schedules` | Lister (pagination cursor) |
| `GET`   | `/v1/schedules/{id}` | Lire détail |
| `PATCH` | `/v1/schedules/{id}` | Désactiver / éditer (CAS via `If-Match`) |
| `DELETE`| `/v1/schedules/{id}` | Soft-delete |
| `POST`  | `/v1/triggers` | Créer un trigger |
| `GET`   | `/v1/triggers` | Lister |
| `POST`  | `/v1/triggers/{id}/fire` | Trigger manuel (idempotency-key requise) |
| `GET`   | `/v1/jobs` | Lister jobs (filtres state, kind, tenant) |
| `GET`   | `/v1/jobs/{id}` | Détail d'un job + runs |
| `POST`  | `/v1/jobs/{id}/cancel` | Annuler un job queued / running |
| `POST`  | `/v1/jobs/{id}/requeue` | Requeue un dead-letter |
| `GET`   | `/v1/policies/{kind}` | Lire la config de policy active |
| `PUT`   | `/v1/policies/{kind}` | Mettre à jour la policy (audit + CAS) |
| `GET`   | `/health` | Liveness |
| `GET`   | `/ready` | Readiness (vérifie leader + store accessible) |
| `GET`   | `/metrics` | Exposition Prometheus |

### §13.2 Contrats Pydantic

Tous les schémas utilisent `ConfigDict(extra="forbid")`.

Exemple du schema de création de schedule :

```text
ScheduleCreate:
  tenant_id: str
  task_id: str | None
  job_kind: str  # validated against registry
  kind: Literal["cron","interval","rrule","one_shot","composite"]
  spec: CronSpec | IntervalSpec | RRuleSpec | OneShotSpec | CompositeSpec
  payload: dict[str, Any]  # validated against JobKind schema
  enabled: bool = true
```

### §13.3 Authentification et autorisation

- JWT signé par l'IdP interne (cf. M11).
- Scopes : `sched:read`, `sched:write`, `sched:admin`, `sched:requeue`.
- RLS Postgres : toute requête filtre sur `tenant_id = current_setting(
  'efe.tenant_id')`.

### §13.4 Idempotence

- `POST /v1/triggers/{id}/fire` requiert un header `Idempotency-Key`
  (RFC draft). Persisté 24 h en table `idempotency_tokens`.
- `POST /v1/schedules` accepte aussi un `Idempotency-Key` pour éviter
  les doublons sur retry client.

### §13.5 Pagination

Pagination *cursor-based* : `cursor = base64(last_created_at ||
last_id)`. Ordre stable sous insertions concurrentes.

---

## §14. Sous-système 10.8 — **Observabilité**

### §14.1 Métriques Prometheus

Noms sous l'espace `efe_sched_*` :

| Métrique | Type | Labels | Description |
|----------|------|--------|-------------|
| `efe_sched_fire_total` | Counter | tenant, kind | Fires enqueued |
| `efe_sched_fire_duplicate_total` | Counter | tenant, kind | Idempotence touches |
| `efe_sched_fire_dropped_total` | Counter | tenant, reason | Catch-up truncation, disabled, etc. |
| `efe_sched_tick_duration_seconds` | Histogram | | Durée d'un tick |
| `efe_sched_job_enqueue_latency_seconds` | Histogram | kind | Fire → queued |
| `efe_sched_job_wait_seconds` | Histogram | kind | Queued → running |
| `efe_sched_job_run_seconds` | Histogram | kind | Running → terminal |
| `efe_sched_job_state_total` | Counter | kind, to_state | Transitions |
| `efe_sched_retry_total` | Counter | kind, reason | Retries |
| `efe_sched_dead_letter_total` | Counter | tenant, kind | Dead-letters |
| `efe_sched_worker_lease_gauge` | Gauge | worker_id | Nombre de jobs détenus |
| `efe_sched_trigger_fire_total` | Counter | tenant, trigger_kind | Fires des triggers |
| `efe_sched_learning_decision_total` | Counter | policy, decision | Verdicts de decision |

### §14.2 Événements structurés

Émis sur NATS `efe.sched.*` :

- `efe.sched.schedule.created`
- `efe.sched.schedule.disabled`
- `efe.sched.job.enqueued`
- `efe.sched.job.started`
- `efe.sched.job.succeeded`
- `efe.sched.job.failed`
- `efe.sched.job.dead_lettered`
- `efe.sched.policy.decision`

Chaque événement est un `JSON canonical` avec `schema_version: 1`, un
`request_id` et un `seal` HMAC (`DOMAIN_TAG_SCHED_EVENT_V1`).

### §14.3 Audit JSONL

Journal append-only persisté dans un bucket objet (S3-compatible). Un
événement par ligne, JSON canonical, les champs sensibles (payload user,
prompts) sont remplacés par leur digest HMAC.

Rétention : 365 jours par défaut, configurable par tenant jusqu'à 7 ans
(conformité AI Act / RGPD journal des décisions algorithmiques).

### §14.4 Traces OpenTelemetry

Un span racine par *tick* ; un span par job ; un span par run. Propagation
du `traceparent` dans chaque événement NATS.

---

## §15. Sous-système 10.9 — **Sécurité et isolation multi-tenant**

### §15.1 Authentification des acteurs internes

- Workers et SchedulerDaemon s'authentifient via mTLS (cert de pod,
  rotation via cert-manager).
- Connexions Postgres : rôle dédié `efe_sched_role` avec GRANT strict
  sur tables M10.
- NATS : credentials dédiés, subjects autorisés listés explicitement
  (`efe.sched.>`).

### §15.2 Séparation d'univers HMAC

Tags utilisés par M10 (cf. Annexe D) :

```
DOMAIN_TAG_SCHED_FIRE        = b"efe-sched/fire/v1\x00"
DOMAIN_TAG_JOB_SEAL          = b"efe-sched/job-seal/v1\x00"
DOMAIN_TAG_SCHED_EVENT_V1    = b"efe-sched/event/v1\x00"
DOMAIN_TAG_DATASET_SEED      = b"efe-sched/dataset-seed/v1\x00"
DOMAIN_TAG_DATASET_SEAL      = b"efe-sched/dataset-seal/v1\x00"
DOMAIN_TAG_PROMOTION_SEAL    = b"efe-sched/promotion-seal/v1\x00"
DOMAIN_TAG_IDEMPOTENCY_TOKEN = b"efe-sched/idempotency/v1\x00"
```

Aucun tag n'est partagé avec les autres modules. Toute valeur signée
avec un tag M10 n'est acceptable comme sceau que pour le rôle M10
correspondant.

### §15.3 PII

- Aucune donnée utilisateur brute n'entre dans un `JobSpec.payload`.
- Les prompts / responses sont référencés par `digest` ; le contenu
  vit dans l'object store chiffré (M03) et n'est récupéré par le worker
  que dans la fenêtre de l'exécution.
- Le `audit JSONL` ne contient que des digests pour les champs
  sensibles.

### §15.4 RBAC

| Scope | Lecture | Écriture | Admin |
|-------|---------|----------|-------|
| `schedule` | `sched:read` | `sched:write` | `sched:admin` |
| `trigger` | idem | idem | idem |
| `job` | idem | — (écriture interne seulement) | `sched:admin` (cancel/requeue) |
| `policy` | `sched:read` | `sched:admin` | `sched:admin` |
| `dead_letter` | `sched:read` | `sched:requeue` | `sched:admin` |

### §15.5 Rate-limit côté API

- Global : `60 req/s/tenant` via token bucket.
- `POST /v1/triggers/{id}/fire` : `10 req/min/tenant` (anti abuse opérateur).
- `PUT /v1/policies/*` : `1 req/min/tenant` (anti thrash).

---

## §16. Budget de performance

| Axe | Cible | Conditions |
|-----|-------|------------|
| Latence d'un tick scheduler | p50 < 50 ms, p99 < 250 ms | 10 000 schedules enabled |
| Enqueue d'un job | p99 < 5 ms | SQL en local, pool saturé 30 % |
| Pull d'un job (`lease_next`) | p99 < 8 ms | Postgres, index chaud |
| Throughput worker | ≥ 500 jobs/s/pod | handlers no-op, network idle |
| Latence fire → worker pick | p99 < 1 s | queue depth < 1 000 |
| Catch-up 6 h, 10 000 schedules | < 60 s | Parallélisation par tenant |
| Mémoire SchedulerDaemon | ≤ 512 MiB | 10 000 schedules |
| Mémoire Worker | ≤ 256 MiB | 50 jobs concurrents |
| Drift → fire (réactif) | p99 < 500 ms | NATS ack ≤ 20 ms |

Un bench reproductible (`pytest-benchmark` + `locust`) est exécuté à
chaque PR touchant `scheduler/engine.py`, `jobs/worker.py` ou
`storage/sql.py`.

---

## §17. Taxonomie d'erreurs

### §17.1 Arbre

```
SchedulingError (root)
├── ConfigError
├── ValidationError
├── NotFound
│   ├── ScheduleNotFound
│   ├── JobNotFound
│   ├── TriggerNotFound
│   └── PolicyNotFound
├── ScheduleError
│   ├── InvalidCronExpression
│   ├── InvalidRRule
│   ├── TimezoneNotFound
│   └── ScheduleDisabled
├── TriggerError
│   ├── InvalidTriggerSpec
│   ├── DebouncerCorrupt
│   └── TriggerSuppressed
├── JobError
│   ├── DuplicateFireKey
│   ├── UnknownJobKind
│   ├── PayloadSchemaViolation
│   ├── JobSealInvalid
│   ├── JobTerminal
│   └── JobBudgetExhausted
├── WorkerError
│   ├── LeaseLost
│   ├── LeaseHeartbeatFailed
│   └── HandlerCrashed
├── StateVersionConflict (CAS)
├── StorageError
└── SecurityError
    ├── TenantMismatch
    ├── ScopeMissing
    └── IdempotencyReplay
```

Chaque classe porte un `code` stable, un `http_status` par défaut, et un
`retryable_hint` boolean exploitable par les clients REST et les workers.

### §17.2 Mapping vers ProblemDetails (RFC 7807)

Toute exception `SchedulingError` non capturée par un handler explicite
est sérialisée par le middleware FastAPI en :

```json
{
  "type": "urn:efe:sched:error:<code>",
  "title": "<TypeName>",
  "status": 409,
  "detail": "<message>",
  "retryable": true,
  "request_id": "<uuid>"
}
```

---

## §18. Stratégie de tests

### §18.1 Matrice

| Couche | Type | Outil | Volume cible |
|--------|------|-------|---------------|
| `clock`, `randomness` | Unit | `pytest` | 100 % branches |
| `cron.py` | Property-based | `hypothesis` | 10 000 cas générés |
| `rrule.py` | Unit + fixtures réelles | `pytest` | 50 RRULEs connues |
| `engine.py` | Integration | `pytest-asyncio` + FakeClock | 30 scénarios |
| `triggers/*` | Unit + property-based | `hypothesis` | hystérésis stochastique |
| `jobs/worker.py` | Integration | `pytest-asyncio` + `testcontainers(postgres)` | 20 scénarios |
| `retry.py` | Property-based | `hypothesis` | convergence des backoffs |
| `learning/*.py` | Unit (table de vérité) | `pytest` parametrize | 100 % branches |
| `storage/sql.py` | Integration | `testcontainers(postgres)` | tous CAS |
| `api/*` | Contract tests | `httpx.AsyncClient` + schemathesis | OpenAPI diff-ed |

### §18.2 Tests de robustesse

- **Chaos** : kill aléatoire du leader pendant un tick → aucun fire perdu,
  aucun fire dupliqué.
- **Fuzz** : envoyer 100 000 `DriftResult` générés avec Hypothesis au
  `DriftThresholdTrigger` → aucun crash, aucun état incohérent du
  debouncer.
- **Longueur** : faire tourner 48 h en boucle sur un cluster réduit,
  comparer le nombre de fires attendu/observé à ± 0.
- **Concurrence** : 50 workers en parallèle sur 1 000 jobs → aucun job
  n'est tiré deux fois.

### §18.3 Couverture et gates CI

- Couverture **line ≥ 92 %**, **branch ≥ 88 %**.
- `mypy --strict` : 0 erreur, 0 `type: ignore` non motivé.
- `ruff check` règle ALL avec dérogations motivées.
- `schemathesis` sur l'OpenAPI : 0 régression contractuelle.
- Bench : régression p99 ≤ 10 % toléré ; au-delà, PR bloquée.

---

## §19. Plan de sprints

### §19.1 Découpage par sprints (2 semaines chacun)

| Sprint | Thème | Livrables | DoD |
|--------|-------|-----------|-----|
| **S1** | Fondations | `clock.py`, `errors.py`, `randomness.py`, `types.py`, `fsm.py`, base Pydantic schemas | 100 % couverture, mypy ok |
| **S2** | Scheduler engine | `scheduler/{cron, interval, one_shot, engine, fire_key, leader}.py` | 30 tests engine verts |
| **S3** | Jobs & workers | `jobs/{registry, queue, executor, worker, retry, lease, dead_letter}.py` | chaos + fuzz verts |
| **S4** | Storage SQL | `storage/{protocols, inmemory, sql}.py` + migrations Alembic | testcontainers green |
| **S5** | Triggers | `triggers/{base, drift, metric, manual, composite, debouncer}.py` | integration M09 stub |
| **S6** | Learning policies | `learning/{baseline_refresh, retrain, curator, promotion, rollback}.py` | tables de vérité 100 % |
| **S7** | Learning jobs | `learning/{training_job, validation, canary, pipeline}.py` | intégration M05 + M07 |
| **S8** | API | `api/{schemas, http_app, deps, errors}.py` | OpenAPI publié, contract tests verts |
| **S9** | Observabilité | `telemetry/{metrics, events, audit}.py` + dashboards Grafana | audit JSONL vérifié |
| **S10** | Sécurité + RBAC | JWT scopes, RLS, rate-limit, idempotency | tests sécu verts |
| **S11** | Chaos + longévité | chaos suite, 48 h run | 0 perte, 0 duplicate |
| **S12** | Release GA | documentation, runbook, release notes | Go/No-Go §20 |

### §19.2 Jalons transverses

- **J1 (fin S3)** — Data-path : schedule → enqueue → worker pick → success,
  no-op handler.
- **J2 (fin S6)** — Boucle réactive : drift → retrain decision → training
  job enqueued. Démo end-to-end avec M09 en mode shadow.
- **J3 (fin S9)** — Observabilité : dashboards, alertes Prometheus,
  audit interrogeable (`jq`).
- **J4 (fin S12)** — GA : release v1.0, runbook, post-mortem template.

---

## §20. Critères Go / No-Go spécifiques à M10

### §20.1 Go si

- Toutes les invariants I1..I10 sont testés en CI et verts.
- Les 3 gates (pytest, mypy, ruff) passent sans dérogation non motivée.
- Le bench régression ≤ 10 % sur les 5 métriques de §16.
- La matrice de risques (Annexe F) a 0 risque critique ouvert.
- Un opérateur (pas l'auteur) a exécuté le runbook « incident M10 » de
  bout en bout dans un environnement de staging.
- L'audit JSONL contient, pour 24 h de trafic, une ligne par job et par
  run, exploitable par `jq`.

### §20.2 No-Go si

- Un test chaos produit un fire perdu ou dupliqué (I1 ou I2 cassé).
- Une transition FSM non-whitelistée est détectée en production (même
  une fois, même sans impact).
- La fuite mémoire > 5 % sur un run 48 h (daemon ou worker).
- Un handler non-idempotent est enregistré avec `idempotent=true` (scan
  des décorateurs en CI).
- Un `JobSpec.hmac_seal` invalide est détecté après reprise
  (I9 cassé).

---

## Annexe A — FSM complète des schedules

```
   ┌─────┐ create    ┌─────────┐  enable   ┌─────────┐
   │ nil ├──────────▶│ disabled├──────────▶│ enabled │
   └─────┘           └─────────┘           └────┬────┘
                         ▲                      │
                         │ disable              │ tick(iter_fires)
                         └──────────────────────┘
                                                │
                         ┌──────────────────────┼─────────┐
                         │                      ▼         ▼
                     (one-shot)           ┌──────────┐ ┌─────────┐
                     ┌──────────┐         │completed │ │ errored │
                     │completed │         └──────────┘ └─────────┘
                     └──────────┘
```

Transitions whitelistées :

- `nil → disabled` (création disabled par défaut si `enabled=false` dans
  la requête).
- `nil → enabled` (création enabled=true).
- `disabled ↔ enabled`.
- `enabled → completed` (one-shot consumed).
- `* → errored` (faute fatale, ex. expression cron invalide détectée à
  l'évaluation).

## Annexe B — FSM complète des jobs et runs

Jobs :

```
       ┌───────┐ enqueue   ┌────────┐ lease   ┌─────────┐ ok   ┌─────────┐
       │ nil   ├──────────▶│ queued ├────────▶│ running ├─────▶│succeeded│ (T)
       └───────┘           └────┬───┘         └────┬────┘      └─────────┘
                                │                   │
                                │                   │ err(retryable) & budget
                                │◀──────────────────┘
                                │
                                │                   │ err(non-retryable)
                                │                   ▼
                                │            ┌─────────────────┐
                                │            │ failed_permanent│ (T)
                                │            └────────┬────────┘
                                │                     │ move
                                │                     ▼
                                │             ┌──────────────┐
                                │             │ dead_letter  │ (T)
                                │             └──────────────┘
                                │
                                │ cancel
                                ▼
                        ┌──────────┐
                        │cancelled │ (T)
                        └──────────┘
```

Runs :

```
       ┌──────┐ start  ┌────────┐ finish(ok)   ┌──────────┐
       │ nil  ├───────▶│running ├─────────────▶│succeeded │ (T)
       └──────┘        └────┬───┘              └──────────┘
                            │
                            │ finish(err)
                            ├─────────▶ ┌──────────┐
                            │           │ failed   │ (T)
                            │           └──────────┘
                            │
                            │ cancel
                            ├─────────▶ ┌──────────┐
                            │           │cancelled │ (T)
                            │           └──────────┘
                            │
                            │ timeout
                            └─────────▶ ┌──────────┐
                                        │ timeout  │ (T)
                                        └──────────┘
```

## Annexe C — Grammaire cron étendue

```
expr    = minute SP hour SP dom SP month SP dow [ SP year ] | alias
alias   = "@yearly" | "@monthly" | "@weekly" | "@daily" | "@hourly"
minute  = range | list | "*" | "*/n"        where n ∈ [1,59]
hour    = range | list | "*" | "*/n"        where n ∈ [1,23]
dom     = range | list | "*" | "*/n" | "L"  where n ∈ [1,30]
month   = range | list | "*" | "*/n" | month_name
dow     = range | list | "*" | "*/n" | day_name | "#" occurrence day_name
year    = range | list | "*" | "*/n"        where n ∈ [1,10]
range   = number "-" number
list    = value ("," value)+
```

Extensions non standards implémentées :

- `L` — dernier jour du mois (pour `dom`).
- `#K DAY` — K-ème DAY du mois (ex. `#2 TUE`).

## Annexe D — Domain-separation tags HMAC

| Constante | Valeur | Usage |
|-----------|--------|-------|
| `DOMAIN_TAG_SCHED_FIRE` | `b"efe-sched/fire/v1\x00"` | Idempotence fire |
| `DOMAIN_TAG_JOB_SEAL` | `b"efe-sched/job-seal/v1\x00"` | Intégrité JobSpec |
| `DOMAIN_TAG_SCHED_EVENT_V1` | `b"efe-sched/event/v1\x00"` | Sceau des events NATS |
| `DOMAIN_TAG_DATASET_SEED` | `b"efe-sched/dataset-seed/v1\x00"` | Seed shuffle dataset |
| `DOMAIN_TAG_DATASET_SEAL` | `b"efe-sched/dataset-seal/v1\x00"` | Sceau manifest dataset |
| `DOMAIN_TAG_PROMOTION_SEAL` | `b"efe-sched/promotion-seal/v1\x00"` | Sceau décision promo |
| `DOMAIN_TAG_IDEMPOTENCY_TOKEN` | `b"efe-sched/idempotency/v1\x00"` | Idempotency-Key API |

Règle d'or : toute nouvelle utilisation de HMAC dans M10 crée **son
propre** tag. La revue PR rejette toute collision.

## Annexe E — Schéma tabulaire Postgres (DDL condensé)

```
CREATE TABLE schedules (
    schedule_id         UUID PRIMARY KEY,
    tenant_id           TEXT NOT NULL,
    task_id             TEXT,
    job_kind            TEXT NOT NULL,
    spec_json           JSONB NOT NULL,
    enabled             BOOLEAN NOT NULL DEFAULT TRUE,
    state               TEXT NOT NULL CHECK (state IN
                          ('disabled','enabled','completed','errored')),
    state_version       INTEGER NOT NULL DEFAULT 1,
    last_evaluated_at   TIMESTAMPTZ,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ON schedules (tenant_id, enabled) WHERE enabled;

CREATE TABLE jobs (
    job_id              UUID PRIMARY KEY,
    fire_key            BYTEA NOT NULL UNIQUE,
    tenant_id           TEXT NOT NULL,
    task_id             TEXT,
    kind                TEXT NOT NULL,
    payload_json        JSONB NOT NULL,
    priority            INTEGER NOT NULL,
    priority_seq        TEXT NOT NULL,
    run_at              TIMESTAMPTZ NOT NULL,
    deadline            TIMESTAMPTZ,
    state               TEXT NOT NULL CHECK (state IN
                          ('queued','running','succeeded',
                           'failed_permanent','cancelled','dead_letter')),
    state_version       INTEGER NOT NULL DEFAULT 1,
    attempts_left       INTEGER NOT NULL,
    lease_expires_at    TIMESTAMPTZ,
    hmac_seal           BYTEA NOT NULL,
    origin              TEXT NOT NULL,
    origin_ref          TEXT NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ON jobs (state, priority DESC, run_at)
  WHERE state IN ('queued','running');

CREATE TABLE runs (
    run_id              UUID PRIMARY KEY,
    job_id              UUID NOT NULL REFERENCES jobs(job_id),
    attempt_idx         INTEGER NOT NULL,
    worker_id           TEXT NOT NULL,
    started_at          TIMESTAMPTZ NOT NULL,
    finished_at         TIMESTAMPTZ,
    status              TEXT NOT NULL CHECK (status IN
                          ('running','succeeded','failed',
                           'cancelled','timeout')),
    error_code          TEXT,
    error_message       TEXT,
    output_digest       BYTEA,
    telemetry_json      JSONB
);
CREATE INDEX ON runs (job_id, started_at);

CREATE TABLE dead_letters (
    job_id              UUID PRIMARY KEY REFERENCES jobs(job_id),
    moved_at            TIMESTAMPTZ NOT NULL,
    last_run_id         UUID REFERENCES runs(run_id),
    reason              TEXT NOT NULL
);

CREATE TABLE policy_configs (
    policy_id           UUID PRIMARY KEY,
    tenant_id           TEXT NOT NULL,
    task_id             TEXT,
    kind                TEXT NOT NULL,
    cfg_json            JSONB NOT NULL,
    state_version       INTEGER NOT NULL DEFAULT 1,
    enabled_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    disabled_at         TIMESTAMPTZ,
    UNIQUE (tenant_id, task_id, kind) WHERE disabled_at IS NULL
);
```

Toutes les tables sont RLS-enabled (`ALTER TABLE ... ENABLE ROW LEVEL
SECURITY`) avec un policy
`USING (tenant_id = current_setting('efe.tenant_id'))`.

## Annexe F — Matrice de risques M10

| Id | Risque | Probabilité | Impact | Mitigation |
|----|--------|-------------|--------|------------|
| R1 | Double-fire sous partition SQL | Faible | Haut | Fire_key UNIQUE + tests chaos |
| R2 | Leader split-brain | Moyenne | Haut | pg_try_advisory_lock + heartbeat 5 s |
| R3 | Handler en boucle infinie | Moyenne | Moyen | Timeout par deadline + retry budget |
| R4 | Surcharge NATS sur tempête de drift | Moyenne | Moyen | Debouncer + suppression_s |
| R5 | Lease expiration en rafale | Faible | Moyen | Heartbeat lease_ttl/3 + détection de clock skew |
| R6 | Policy maligne → cascade rollback | Faible | Haut | Freeze cool-down 24 h après rollback |
| R7 | HMAC tag collision (régression) | Très faible | Haut | Test contract CI scannant les constantes |
| R8 | Fuite PII via payload | Faible | Haut | Linter PII + digest-only |
| R9 | Perte catastrophique Postgres | Très faible | Critique | WAL + replica standby + backups chiffrés |

## Annexe G — Glossaire

| Terme | Définition |
|-------|------------|
| **Fire** | Instance d'évaluation d'un schedule à un instant donné |
| **Fire-key** | Digest HMAC unique `(schedule_id, fire_time)` |
| **Lease** | Droit temporaire détenu par un worker sur un job en cours |
| **Dead-letter** | État terminal d'un job épuisé ou toxique |
| **Debouncer** | Machine d'états limitant la cadence d'un trigger |
| **Hysteresis** | Exigence de N breaches consécutives avant fire |
| **Freeze** | Suspension temporaire d'une policy après rollback |
| **Cool-down** | Période pendant laquelle une action est refusée |
| **Canary** | Artefact sous trafic minoritaire avant promotion |
| **RRULE** | Règle récurrente au format RFC 5545 iCalendar |
| **CAS** | Compare-And-Swap : écriture optimiste |
| **FSM** | Finite-State Machine : graphe des transitions permises |

---

*Fin du document. Relecture trimestrielle par l'équipe plateforme et
mise à jour en cas de rupture d'invariant, d'ajout de JobKind ou
d'évolution du schéma SQL. Toute modification requiert une ADR.*
