# M08 — Rollout &amp; A/B Testing
## Architecture complète, ultra-détaillée
### (expérimentation statistique rigoureuse + bandits + guardrails + multi-layer orthogonality)

---

**Document** : spécification d'architecture
**Module Python** : `efe_rollout` (paquet) + sous-paquets `assignment/`, `metrics/`, `analysis/`, `bandits/`, `guardrails/`, `layers/`, `power/`, `api/`, `db/`, `nats/`
**Sprint ciblé** : Sprint 8 (semaines 15-16) du plan MVP
**Dépend de** : M05 (Gate : feu vert offline pour lancer l'expé), M06 (Routing : applique les décisions d'assignation), M07 (Inference : émet les events consommés par M08), M10 (Fairness : reçoit les analyses segmentées)
**Consommé par** : M06 (canary controller peut consulter un verdict bayésien), M10 (fairness), M11 (API client), M12 (dashboards observability)
**Statut** : spec gelée pour implémentation — aucune ambiguïté subsistante
**Invariants non-négociables** :
  - Assignation déterministe par `(user_id, experiment_id)` — même utilisateur, même variant, pour toute la durée de l'expé
  - Orthogonalité stricte inter-layers : aucune corrélation induite entre expés sur layers différents
  - Sequential testing valide : α-spending Pocock ou O'Brien-Fleming, pas de peeking naïf
  - Pre-registration obligatoire : hypothèse, metric primaire, sample size, stopping rules scellés avant exposition
  - Zéro exposure sans log : toute décision d'assignation est loggée avec HMAC
  - CUPED variance reduction proposée par défaut quand covariate disponible
  - Guardrails auto-abort : 3 breaches consécutifs → expé tuée + rollback M06

---

## Table des matières

1. [Mission et positionnement](#1-mission-et-positionnement)
2. [Scope MVP — ce qui est IN et OUT](#2-scope-mvp--ce-qui-est-in-et-out)
3. [Théorie : expérimentation rigoureuse](#3-théorie--expérimentation-rigoureuse)
4. [Vue d'ensemble — 8 plans](#4-vue-densemble--8-plans)
5. [Cycle de vie d'une expérience](#5-cycle-de-vie-dune-expérience)
6. [Modèle de données](#6-modèle-de-données)
7. [Assignation de variants](#7-assignation-de-variants)
8. [Collection d'exposures &amp; outcomes](#8-collection-dexposures--outcomes)
9. [Analyse statistique](#9-analyse-statistique)
10. [Multi-armed bandits](#10-multi-armed-bandits)
11. [Guardrails &amp; auto-abort](#11-guardrails--auto-abort)
12. [Orthogonalité multi-layer](#12-orthogonalité-multi-layer)
13. [Segment analysis &amp; HTE](#13-segment-analysis--hte)
14. [Meta-analysis &amp; knowledge base](#14-meta-analysis--knowledge-base)
15. [NATS topics et flows](#15-nats-topics-et-flows)
16. [Interfaces publiques](#16-interfaces-publiques)
17. [Stratégie de tests](#17-stratégie-de-tests)
18. [Observabilité](#18-observabilité)
19. [Plan de sprint M08](#19-plan-de-sprint-m08)
20. [Critères d'acceptation](#20-critères-dacceptation)
21. [Risques et mitigations](#21-risques-et-mitigations)
22. [Annexes](#22-annexes)

---

## 1. Mission et positionnement

### 1.1 Mission fonctionnelle

M08 est la **plateforme d'expérimentation statistique** de l'EFE. Elle
permet de :

1. **Tester une hypothèse en production** avec un protocole rigoureux :
   pre-registration, power analysis, randomisation stable, collecte
   d'exposures + outcomes, analyse fréquentiste ou bayésienne, stopping
   rule.
2. **Allouer dynamiquement le trafic** via multi-armed bandits
   (Thompson Sampling, UCB1, epsilon-greedy) pour des cas où le coût
   de l'exploration symmetric 50/50 est prohibitif.
3. **Empiler plusieurs expés en parallèle** sans interférence
   statistique via un système de **layers orthogonaux** (Google's
   overlapping experiment infrastructure, 2010).
4. **Surveiller les guardrails** (latence, erreur, qualité implicite)
   et tuer automatiquement une expé qui régresse.
5. **Cross-tenant knowledge base** : archiver les résultats pour
   informer les décisions futures.

M08 **complète M06** : là où M06 canary fait un ramp-up progressif
pour **basculer** (objectif opérationnel), M08 A/B fait un split
contrôlé pour **mesurer et décider** (objectif scientifique).

### 1.2 Distinction avec modules adjacents

| Module | Objectif | Trafic | Décision |
|---|---|---|---|
| **M05 Gate** | feu vert offline (pré-prod) | 0 % user | McNemar / bootstrap sur golden set |
| **M06 Canary** | bascule progressive | 1→5→25→100 % | SLO + quality drop |
| **M07 Shadow** | évaluation zéro-impact | 100 % baseline + 10 % shadow lecture | scoring offline |
| **M08 A/B** | test scientifique | 50 % A / 50 % B (ou N arms) | significance + effect size |
| **M08 Bandit** | optim continue | allocation dynamique | regret minimisation |

Ces modes ne sont pas mutuellement exclusifs : un challenger peut
passer M05 (offline) → M07 Shadow (zéro-impact) → M08 A/B (mesure) →
M06 Canary (bascule). Chaque étape filtre les mauvais candidats.

### 1.3 Position dans le pipeline EFE

```
                          ┌─────────────────────────┐
                          │  M11 Gateway (AuthN/Z)  │
                          └───────────┬─────────────┘
                                      │ /v1/chat ...
                                      ▼
                  ┌───────────────────────────────────┐
                  │   M07 Inference Adapter           │
                  │  (polyglot)                        │
                  └─────┬─────────────────────────┬───┘
                        │                         │
                        │ emit exposure +         │ call model
                        │ outcome to M08          │ via M06 routing
                        ▼                         ▼
           ┌────────────────────────┐    ┌──────────────────────┐
           │  M08 Rollout Service   │    │  M06 Model Registry  │
           │  - Assigner            │◀── │  - TenantRouter       │
           │  - MetricsIngester     │ get│  - Canary              │
           │  - Analyzer            │ variant decision              │
           │  - BanditArm           │    │                        │
           │  - GuardrailWatcher    │    │                        │
           └─────────┬──────────────┘    └──────────────────────┘
                     │
                     ▼
           ┌──────────────────────┐
           │   Postgres + Redis   │
           │ experiments,         │
           │ exposures, outcomes, │
           │ analyses, decisions  │
           └──────────────────────┘

  Amont : M05 Gate publie "model X est promotable"
  Aval  : M10 Fairness ingère les analyses segmentées
          M06 peut lire un verdict bayésien pour piloter canary
          M12 dashboards Grafana
```

### 1.4 Les deux boucles temporelles de M08

**Boucle longue (expé classique A/B)** :

```
 Design → Power analysis → Pre-register → Deploy → Collect → Analyze → Decide
   ⏱ 1 h         1 h           30 min     14 j     14 j     2 h     — 
```

Durée typique d'un A/B : 1 à 4 semaines. Le **temps** est une
ressource critique : on ne peut pas raccourcir arbitrairement
l'exposition sous peine de violer la power analysis.

**Boucle courte (bandit)** :

```
   Observation batch (5 min) → Policy update → New allocation → repeat
```

Le bandit met à jour ses poids toutes les 5 min à partir des outcomes
observés ; il converge naturellement vers l'arm optimal sans décision
humaine.

Les deux boucles cohabitent dans M08 avec des APIs distinctes.

### 1.5 Contraintes non-négociables

| Contrainte | Valeur | Preuve |
|---|---|---|
| Assignation stable | même (user, experiment) → même variant toujours | test unitaire + audit quotidien |
| Pre-registration scellée | hypothesis + metric + N avant 1re exposition | hash SHA-256 persisté, log audit |
| Sequential testing valide | α-spending function (Pocock / OBF) | test contre résultats théoriques |
| No p-hacking | stopping rules dans le pre-register, blocage peek avant N attendu | flag `early_stop_allowed` |
| Guardrails bloquants | auto-abort si 3 breaches successifs | métrique + alerte P1 |
| Orthogonalité inter-layers | corrélation ≤ 0.05 entre assignations de 2 layers | job quotidien de vérification |
| Exposure logging complet | chaque call inference → 1 exposure row | counter == count(distinct) |
| Holdout global 1 % | 1 % user jamais exposé à aucune expé (baseline long-terme) | exclusion rule au top de l'assigner |

---

## 2. Scope MVP — ce qui est IN et OUT

### 2.1 IN au MVP (sprint 8, 2 semaines)

**Types d'expériences** :

- **Fixed-horizon A/B/n** (2-N variants), sample size calculé,
  arrêt prévu à date / N atteint.
- **Sequential A/B** avec α-spending (Pocock et O'Brien-Fleming).
- **Multi-armed bandit** — Thompson Sampling bêta-binomial, UCB1,
  ε-greedy avec ε décroissant.
- **Holdout global** : fraction fixée (1 % default) jamais exposée à
  aucune expé — baseline long-terme + détection drift.

**Primitive statistiques** :

- Test de différence de proportions (Wald + Newcombe).
- Test de différence de moyennes (Welch t-test + bootstrap paired CI).
- Test du chi-carré.
- Mann-Whitney U (non-paramétrique).
- CUPED variance reduction (Deng et al., 2013) avec une covariate.
- Cluster-robust standard errors (user-level randomisation).
- Bonferroni / Benjamini-Hochberg pour corrections multiples.

**Multi-layer** :

- 3 layers prédéfinis (``prompt``, ``model``, ``post-process``) non-
  chevauchants par construction.
- Hash-based orthogonal assignment (murmur3 x tenant-salt x layer-salt).

**Guardrails** :

- Metrics de santé : latence p95, error rate 5xx, cost per request,
  quality implicite (thumbs-down).
- Seuils relatifs : ratio candidate/baseline dépassé pendant N fenêtres
  de 5 min.
- Auto-abort → dispatch NATS `efe.rollout.aborted` → M06 rollback.

**Pre-registration &amp; audit** :

- Hash SHA-256 du plan figé (hypothesis, metric, sample size,
  stopping rule) signé HMAC, stocké dans `experiment_preregistrations`.
- Toute modification post-lancement → nouvelle expé (ID différent).

**API** :

- REST : `POST /experiments`, `GET /experiments/{id}`,
  `POST /experiments/{id}/decisions`, `GET /experiments/{id}/analysis`,
  `POST /experiments/{id}/abort`.
- SDK client : `efe_rollout.get_variant(user_id, experiment_id)` avec
  cache local + fallback.

### 2.2 OUT au MVP

| Fonctionnalité | Raison | Jalon |
|---|---|---|
| Contextual bandits (LinUCB, neural bandits) | complexité vs gain pour use-cases MVP | v1.2 |
| Interleaving (search ranking) | très spécifique retrieval | v1.1 |
| Simpson's paradox auto-detection | nécessite toolkit plus profond | v1.2 |
| Switchback experiments (marketplace) | pas notre cas d'usage | — |
| Causal graphs (DAGs de causalité) | chercheurs avancés seulement | v2 |
| Federated A/B (multi-cluster) | un cluster suffit MVP | v1.1 |
| Firebase Remote Config-style | overlap M08/M06 ; éviter | — |

### 2.3 Roadmap post-MVP

- **v1.1** : interleaving pour retrieval, switchback simplifié,
  federated A/B cross-région.
- **v1.2** : contextual bandits, Simpson's paradox detector,
  causal inference helpers.
- **v2.0** : DAGs causaux, experimentation-as-code (Starlark DSL).

---

## 3. Théorie : expérimentation rigoureuse

### 3.1 Le protocole scientifique en 7 étapes

**E1 — Hypothèse** : formulée avant toute donnée, sous forme
*"le variant B améliore la métrique X d'au moins δ avec significance
0.05 et power 0.8"*.

**E2 — Metric primaire &amp; guardrails** : exactement UNE métrique
primaire (pas de "overall evaluation criterion" flou), et 3-5
guardrails fixés à l'avance.

**E3 — Design &amp; power analysis** : calcul du sample size N tel
que si l'effet réel est ≥ δ, la probabilité de le détecter est ≥ 0.8.

**E4 — Randomisation** : stable par user, hash-based, orthogonale aux
autres expés actives.

**E5 — Exposition logging** : toute décision d'assignation est
persistée avec HMAC, même si le user n'appelle jamais l'endpoint
expérimentalement concerné (intention-to-treat).

**E6 — Analyse** : sous le protocole pré-enregistré, avec intervalle
de confiance et p-value (fréquentiste) ou posterior distribution
(bayésien).

**E7 — Décision** : respecter la stopping rule. Publier les résultats,
même négatifs, dans la knowledge base M08.

### 3.2 Invariants statistiques (I1-I8)

**I1 — Randomisation stable**
> Pour toute paire `(user_id, experiment_id)` et tout moment t ≤ end_at,
> `assigner.get_variant(user_id, experiment_id)` renvoie le même variant.

**I2 — Unbiased estimator**
> La moyenne de la métrique par variant converge vers la vraie moyenne
> de la population exposée à ce variant (intention-to-treat respectée).

**I3 — Valid p-values under sequential testing**
> Pour toute expé séquentielle correctement configurée, P(rejeter H0 |
> H0 vraie) ≤ α cumulé, quel que soit le nombre de peeks.

**I4 — Orthogonalité multi-layer**
> Pour deux expés E1, E2 sur layers différents, et toute partition du
> cohort users, `corr(assign_E1, assign_E2) ≤ 0.05`.

**I5 — Exposition logée**
> Pour tout outcome observé pour un (user, experiment), il existe au
> moins une exposition antérieure pour ce (user, experiment).

**I6 — Pre-registration scellée**
> Toute analyse publie un hash du plan pré-enregistré ; si ce hash
> diffère du hash au moment du lancement, l'analyse est marquée
> "post-hoc" (traçabilité).

**I7 — Guardrails bloquants**
> Toute expé qui déclenche N breaches consécutifs passe en
> `state=aborted` en < 60 s. Aucune assignation vers le variant
> défaillant au-delà de l'abort.

**I8 — Holdout permanent**
> Un sous-ensemble fixé de users (default 1 %) n'est jamais assigné
> à aucun variant — ils restent sur le champion "reference".

### 3.3 Power analysis — formules

Pour un test de différence de proportions binaires (e.g., CTR) :

```
    n_per_variant = ((z_{α/2} + z_β)² × (p₁(1-p₁) + p₂(1-p₂))) / (p₂ - p₁)²
```

Avec `p₁` la proportion baseline, `p₂ = p₁ × (1 + relative_lift)`,
`z_{α/2} = 1.96` (α=0.05), `z_β = 0.84` (power=0.8).

Exemple : baseline CTR = 5 %, lift minimal détectable = 10 % relatif
→ n ≈ **31 000 users par variant**.

Pour des moyennes (e.g., latency), remplacer variances proportion par
variances estimées via un pilot.

### 3.4 Variance reduction : CUPED

CUPED (Deng et al., 2013) réduit la variance de l'estimateur en
soustrayant une covariate pré-experience non-corrélée au traitement :

```
    Y_adj = Y - θ × (X - E[X])
```

avec `θ = Cov(Y, X) / Var(X)`. Gain typique : réduction 30-50 % de la
variance → même power avec 30-50 % de sample size en moins.

Covariates recommandées : métrique pré-experiment même-utilisateur,
features agrégées M01.

### 3.5 Cluster-robust SE (Huber-White sandwich)

Quand la randomisation est par user mais les outcomes sont au niveau
request (un user fait plusieurs requêtes), les SE naïfs sous-estiment
l'incertitude. On utilise des SE cluster-robust :

```
    Var_cluster(β̂) = (X'X)⁻¹ × (∑_g X_g' û_g û_g' X_g) × (X'X)⁻¹
```

où `g` indexe les clusters (users). Implémenté via `statsmodels.OLS`
avec `cov_type='cluster'`.

### 3.6 Multi-armed bandits : Thompson Sampling

Pour K bras bêta-binomiaux (cas binaire : clic/pas-clic) :

1. Chaque bras `k` a un prior `Beta(α_k, β_k)`, initialisé à (1, 1).
2. À chaque request :
   - Sample `θ_k ~ Beta(α_k, β_k)` pour chaque bras.
   - Choisir le bras `argmax(θ_k)`.
3. Observer outcome `y ∈ {0, 1}`.
4. Mettre à jour : si `y=1` → `α_k += 1`; si `y=0` → `β_k += 1`.

Regret : `O(√(K·T·log T))` — meilleur que ε-greedy en pratique.

### 3.7 Budget temporel des opérations

| Opération | Budget | Notes |
|---|---|---|
| `assigner.get_variant` | p99 < 2 ms | Redis HGET + hash ; zéro round-trip backend |
| `record_exposure` | p99 < 5 ms | NATS publish fire-and-forget |
| `record_outcome` | p99 < 5 ms | idem |
| `bandit.update` batch | 5 min | tous les 5 min sur les outcomes accumulés |
| `analyze` on-demand | p95 < 10 s | calcul sur agrégats pré-computés |
| `guardrail.tick` | 1 min | rolling check |
| `auto_abort` décision → M06 rollback | < 60 s | I7 |

---

## 4. Vue d'ensemble — 8 plans

### 4.1 Experiment Lifecycle Plane

- `ExperimentSpec` : immutable specification (hash-sealed).
- `ExperimentLifecycle` : FSM 9 états, 12 transitions.
- `PreRegistration` : signature HMAC du plan.

### 4.2 Assignment Plane

- `HashAssigner` : murmur3-based, stable, layer-aware.
- `HoldoutFilter` : 1 % universal holdout.
- `ForcedAssignment` : liste d'overrides (QA, founders).
- `AssignmentCache` : Redis HSET par `(experiment_id, user_id)`.

### 4.3 Metrics Ingestion Plane

- `ExposureRecorder` : écrit `exposures` table.
- `OutcomeRecorder` : écrit `outcomes` table.
- `ExposureEnricher` : joint avec user segments.

### 4.4 Analysis Plane

- `FrequentistAnalyzer` : Wald, Welch t-test, bootstrap CI.
- `BayesianAnalyzer` : bêta-binomial, normal-normal posteriors.
- `CupedAdjuster` : applique CUPED si covariate disponible.
- `HteAnalyzer` : effets hétérogènes par segment.

### 4.5 Bandit Plane

- `BanditArm` : state Beta(α, β) par variant.
- `ThompsonSampler`, `Ucb1Sampler`, `EpsilonGreedySampler`.
- `BanditUpdater` : batch periodic updates.

### 4.6 Guardrails Plane

- `GuardrailSpec` : metric + threshold + window + consecutive_breaches.
- `GuardrailWatcher` : daemon polling every 60 s.
- `AutoAbortExecutor` : coordone avec M06 pour rollback.

### 4.7 Layers Plane

- `LayerRegistry` : 3 layers prédéfinis + API pour custom.
- `LayerAssigner` : hash-based orthogonal routing.
- `LayerAllocation` : current active experiment per `(layer, tenant)`.

### 4.8 Observability Plane

- Metrics Prometheus (exposures, outcomes, abort count, posterior
  probabilities).
- Logs structurés (audit trail pré-reg, abort reasons).
- Dashboards Grafana dédiés.

---

## 5. Cycle de vie d'une expérience

### 5.1 Machine à états (9 états, 12 transitions)

```
      ┌─────────┐        design complete            ┌────────────┐
      │ DRAFT   ├───────────────────────────────▶  │ DESIGNED   │
      └────┬────┘                                   └──────┬─────┘
           │                                               │ pre-register
           │ abort draft                                   ▼
           │                                       ┌─────────────┐
           │                                       │ REGISTERED  │
           │                                       └──────┬──────┘
           │                                              │ deploy
           │                                              ▼
           │                                       ┌─────────────┐
           │                                       │  RUNNING    │◀──┐
           │                                       └──────┬──────┘   │ resume
           │                    pause ──────┐            │ │   │
           │                                ▼            │ │   │
           │                          ┌─────────┐        │ │   │
           │                          │ PAUSED  │────────┘ │   │
           │                          └────┬────┘          │   │
           │                               │               │   │
           ▼                               ▼               ▼   │
      ┌──────────┐                    ┌─────────┐  analysis done
      │ CANCELED │                    │ ABORTED │     │
      └──────────┘                    └─────────┘     ▼
                                                ┌───────────┐
                                                │ ANALYZED  │
                                                └─────┬─────┘
                                                      │ decision
                                           ┌──────────┼──────────┐
                                           ▼          ▼          ▼
                                      ┌─────────┐ ┌─────────┐ ┌─────────┐
                                      │SHIPPED  │ │ROLLBACK │ │INCONCL. │
                                      └─────────┘ └─────────┘ └─────────┘
```

**Transitions autorisées** (strict whitelist):

| Depuis | Vers | Garde | Acteur |
|---|---|---|---|
| DRAFT | DESIGNED | power analysis OK | owner |
| DRAFT | CANCELED | — | owner/admin |
| DESIGNED | REGISTERED | hash scellé + HMAC | owner |
| REGISTERED | RUNNING | deploy to M06/M07 OK | owner |
| RUNNING | PAUSED | — | owner/admin |
| RUNNING | ABORTED | guardrail breach | auto/admin |
| RUNNING | ANALYZED | N ≥ planned OR time_up | scheduler |
| PAUSED | RUNNING | — | owner |
| PAUSED | ABORTED | admin | admin |
| ANALYZED | SHIPPED | decision=ship | owner |
| ANALYZED | ROLLBACK | decision=rollback | owner |
| ANALYZED | INCONCL. | decision=inconclusive | owner |

**Terminaux** : CANCELED, ABORTED, SHIPPED, ROLLBACK, INCONCLUSIVE.

### 5.2 Effets de bord par transition

- `DESIGNED → REGISTERED` : hash SHA-256 du spec sealed ; HMAC signature
  stockée dans `experiment_preregistrations`.
- `REGISTERED → RUNNING` : routing appliqué à M06 (`start_canary`-like
  mais avec logic M08), NATS `efe.rollout.started` publié.
- `RUNNING → ABORTED` : NATS `efe.rollout.aborted` publié, M06 appelle
  `Rollback.execute()`, exposure recording **continue** pour audit mais
  toute assignation future retourne control variant.
- `ANALYZED → SHIPPED` : NATS `efe.rollout.shipped`, M06 promeut le
  winner au 100 %.
- `ANALYZED → ROLLBACK` : NATS `efe.rollout.rollback_requested`.

### 5.3 Idempotence de transitions

Toute transition utilise CAS (Compare-And-Swap) sur `state_version`.
Deux opérateurs ne peuvent pas transiter la même expé en même temps.
Schéma identique à `efe_registry`.

---

## 6. Modèle de données

### 6.1 DDL Postgres (DDL complet)

```sql
-- ============================================================
-- experiments — specification + lifecycle
-- ============================================================
CREATE TABLE experiments (
    experiment_id    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id        TEXT NOT NULL,
    task_id          TEXT NOT NULL,
    name             TEXT NOT NULL,
    layer            TEXT NOT NULL,       -- 'prompt' | 'model' | 'postproc'
    kind             TEXT NOT NULL CHECK (kind IN (
                       'ab_fixed', 'ab_sequential', 'bandit', 'holdout')),
    state            TEXT NOT NULL CHECK (state IN (
                       'draft','designed','registered','running','paused',
                       'aborted','analyzed','shipped','rollback',
                       'inconclusive','canceled')),
    state_version    BIGINT NOT NULL DEFAULT 0,
    owner            TEXT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    designed_at      TIMESTAMPTZ,
    registered_at    TIMESTAMPTZ,
    started_at       TIMESTAMPTZ,
    ended_at         TIMESTAMPTZ,
    planned_end_at   TIMESTAMPTZ,
    planned_sample_size INTEGER,
    hypothesis       TEXT NOT NULL,
    primary_metric   TEXT NOT NULL,
    alpha            DOUBLE PRECISION NOT NULL DEFAULT 0.05,
    power_target     DOUBLE PRECISION NOT NULL DEFAULT 0.8,
    minimum_detectable_effect DOUBLE PRECISION NOT NULL,
    stopping_rule    TEXT NOT NULL DEFAULT 'fixed_horizon',  -- or 'pocock','obf'
    spec_hash        BYTEA NOT NULL,         -- SHA-256 of the sealed spec
    UNIQUE (tenant_id, name)
);

CREATE INDEX ix_experiments_tenant_state
    ON experiments (tenant_id, state);
CREATE INDEX ix_experiments_layer_state
    ON experiments (layer, state);

-- ============================================================
-- variants — one row per arm per experiment
-- ============================================================
CREATE TABLE variants (
    variant_id       UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    experiment_id    UUID NOT NULL REFERENCES experiments(experiment_id)
                                  ON DELETE CASCADE,
    name             TEXT NOT NULL,        -- 'control', 'treatment_a', ...
    is_control       BOOLEAN NOT NULL DEFAULT false,
    allocation_pct   SMALLINT NOT NULL CHECK (
                        allocation_pct >= 0 AND allocation_pct <= 100),
    payload_jsonb    JSONB NOT NULL,       -- model_id, prompt_template, ...
    UNIQUE (experiment_id, name)
);

CREATE INDEX ix_variants_experiment
    ON variants (experiment_id);

-- Constraint (trigger): sum(allocation_pct) over variants of one experiment
-- must equal 100. See Appendix E.

-- ============================================================
-- experiment_preregistrations — sealed plan + HMAC signature
-- ============================================================
CREATE TABLE experiment_preregistrations (
    prereg_id        UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    experiment_id    UUID NOT NULL REFERENCES experiments(experiment_id)
                                  ON DELETE CASCADE,
    spec_hash        BYTEA NOT NULL,          -- must match experiments.spec_hash
    signing_key_id   TEXT NOT NULL,
    signature        BYTEA NOT NULL,          -- HMAC-SHA256
    plan_json        JSONB NOT NULL,          -- frozen copy
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (experiment_id)                     -- one pre-reg per experiment
);

-- ============================================================
-- exposures — audit trail of every assignment decision
-- Partition by day for retention + query speed.
-- ============================================================
CREATE TABLE exposures (
    exposure_id      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    experiment_id    UUID NOT NULL REFERENCES experiments(experiment_id),
    variant_id       UUID NOT NULL REFERENCES variants(variant_id),
    tenant_id        TEXT NOT NULL,
    user_id          TEXT NOT NULL,
    assigned_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    layer            TEXT NOT NULL,
    forced           BOOLEAN NOT NULL DEFAULT false,
    holdout          BOOLEAN NOT NULL DEFAULT false,
    request_id       TEXT,
    segment_hash     BYTEA,                   -- hashed segment for HTE
    hmac_sig         BYTEA NOT NULL
) PARTITION BY RANGE (assigned_at);

CREATE INDEX ix_exposures_exp_time
    ON exposures (experiment_id, assigned_at);
CREATE INDEX ix_exposures_exp_user
    ON exposures (experiment_id, user_id);

-- ============================================================
-- outcomes — metric observations linked to exposure
-- ============================================================
CREATE TABLE outcomes (
    outcome_id       UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    experiment_id    UUID NOT NULL REFERENCES experiments(experiment_id),
    variant_id       UUID NOT NULL REFERENCES variants(variant_id),
    tenant_id        TEXT NOT NULL,
    user_id          TEXT NOT NULL,
    metric_name      TEXT NOT NULL,
    metric_value     DOUBLE PRECISION NOT NULL,
    observed_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    request_id       TEXT,
    covariate_json   JSONB                   -- CUPED covariate + segment
) PARTITION BY RANGE (observed_at);

CREATE INDEX ix_outcomes_exp_metric
    ON outcomes (experiment_id, metric_name, observed_at);

-- ============================================================
-- analyses — computed results (frequentist + bayesian)
-- ============================================================
CREATE TABLE analyses (
    analysis_id      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    experiment_id    UUID NOT NULL REFERENCES experiments(experiment_id),
    kind             TEXT NOT NULL CHECK (kind IN (
                        'interim','final','post_hoc','segment','cuped')),
    computed_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    n_per_variant    JSONB NOT NULL,
    estimate         JSONB NOT NULL,
    ci_95            JSONB,
    p_value          DOUBLE PRECISION,
    posterior_prob   DOUBLE PRECISION,        -- P(B > A) for bayesian
    decision         TEXT,                     -- recommended: 'ship','rollback','inconclusive'
    methods          JSONB NOT NULL            -- {'cuped': true, 'cluster_robust': true}
);

CREATE INDEX ix_analyses_exp_time
    ON analyses (experiment_id, computed_at);

-- ============================================================
-- bandit_state — stateful Beta/posterior for each arm
-- ============================================================
CREATE TABLE bandit_state (
    experiment_id    UUID NOT NULL REFERENCES experiments(experiment_id),
    variant_id       UUID NOT NULL REFERENCES variants(variant_id),
    alpha            DOUBLE PRECISION NOT NULL DEFAULT 1.0,
    beta             DOUBLE PRECISION NOT NULL DEFAULT 1.0,
    n_pulls          BIGINT NOT NULL DEFAULT 0,
    n_successes      BIGINT NOT NULL DEFAULT 0,
    last_updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (experiment_id, variant_id)
);

-- ============================================================
-- guardrails — per-experiment guardrail specs + last state
-- ============================================================
CREATE TABLE guardrails (
    guardrail_id     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    experiment_id    UUID NOT NULL REFERENCES experiments(experiment_id)
                                  ON DELETE CASCADE,
    metric_name      TEXT NOT NULL,
    direction        TEXT NOT NULL CHECK (direction IN ('up','down')),
    threshold_pct    DOUBLE PRECISION NOT NULL,  -- e.g. +20% vs control
    window_s         INTEGER NOT NULL DEFAULT 300,
    consecutive_breaches INTEGER NOT NULL DEFAULT 3,
    breaches_so_far  INTEGER NOT NULL DEFAULT 0,
    last_check_at    TIMESTAMPTZ,
    last_breach_at   TIMESTAMPTZ
);

-- ============================================================
-- forced_assignments — overrides for QA / founders / debug
-- ============================================================
CREATE TABLE forced_assignments (
    experiment_id    UUID NOT NULL REFERENCES experiments(experiment_id),
    user_id          TEXT NOT NULL,
    variant_id       UUID NOT NULL REFERENCES variants(variant_id),
    reason           TEXT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_by       TEXT NOT NULL,
    PRIMARY KEY (experiment_id, user_id)
);

-- ============================================================
-- layer_allocations — active experiment per (layer, tenant, task)
-- Enforces at most ONE running experiment per slot.
-- ============================================================
CREATE TABLE layer_allocations (
    tenant_id        TEXT NOT NULL,
    task_id          TEXT NOT NULL,
    layer            TEXT NOT NULL,
    experiment_id    UUID NOT NULL REFERENCES experiments(experiment_id),
    started_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    ended_at         TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, task_id, layer, experiment_id)
);

-- Postgres partial unique index enforces ONE active exp per slot:
CREATE UNIQUE INDEX uq_layer_one_active
    ON layer_allocations (tenant_id, task_id, layer)
    WHERE ended_at IS NULL;
```

### 6.2 Schéma Redis

```
# Assignment cache (hot path)
rollout:assign:{tenant}:{experiment}           HASH
  {user_id} -> {variant_id}
  TTL = experiment duration

# Version counter for cache invalidation
rollout:assign:{tenant}:{experiment}:v          STRING

# Bandit hot state (mirror of DB)
rollout:bandit:{experiment}:{variant}           HASH
  alpha, beta, n_pulls, n_successes

# Guardrail counters
rollout:guardrail:{experiment}:{metric}         HASH
  breaches, last_breach_ms

# PubSub channels
channel rollout.assignment.updated
channel rollout.guardrail.breached
channel rollout.experiment.state_changed
```

### 6.3 Partitioning + retention

- `exposures` + `outcomes` partitionnées par jour.
- Retention : 90 j (par défaut), jusqu'à 365 j pour les expés shipped.
- Agrégats rollup dans `analyses` permettent d'analyser long-terme
  même après purge des partitions brutes.

---

## 7. Assignation de variants

### 7.1 Principe : hash stable

Pour chaque `(user_id, experiment_id)`, on calcule un hash
déterministe qui détermine le variant :

```python
def assign(user_id: str, exp: Experiment) -> Variant | None:
    # 1. Forced?
    if (force := forced_assignments.get((exp.id, user_id))):
        return force.variant

    # 2. Holdout global (1%)?
    if murmur3(f"holdout:{user_id}") % 10000 < 100:
        return None    # hold out → no variant

    # 3. Layer allocation
    layer_hash = murmur3(f"layer:{exp.layer}:{user_id}:{exp.tenant_salt}")
    if layer_hash % 10000 >= exp.traffic_allocation_bps:
        return None    # outside this experiment's traffic allocation

    # 4. Variant pick
    variant_hash = murmur3(f"variant:{exp.id}:{user_id}") % 10000
    cumulative = 0
    for variant in exp.variants_sorted_by_name:
        cumulative += variant.allocation_pct * 100
        if variant_hash < cumulative:
            return variant
    return exp.variants_sorted_by_name[-1]    # safety
```

**Propriétés critiques** :

- **I1 stable** : même hash input → même variant output.
- **Orthogonal avec autres expés** via `layer` différent dans le hash.
- **Sticky même après restart** du pod Assigner : rien n'est stocké,
  tout est recomputé.

### 7.2 Choix du hash : MurmurHash3 vs SHA-256

MurmurHash3 :
- Non-cryptographique, très rapide (~200 MB/s).
- Distribution uniforme excellente sur chaines courtes.
- 0 collision problème dans nos gammes (< 10^9 users).

SHA-256 :
- Cryptographique ; plus lent (~100 MB/s).
- Overkill pour assignation ; réservé à `hmac_sig`.

**Décision MVP** : `mmh3.hash64` pour assignations, `hmac-sha256` pour
audit signatures.

### 7.3 Layer salts + rotation

Chaque layer a un salt statique par tenant :

```yaml
tenant_salts:
  acme:
    prompt: "salt-acme-prompt-v1"
    model:  "salt-acme-model-v1"
    postproc: "salt-acme-postproc-v1"
```

Rotation manuelle : changer le salt vide tous les assignments et
démarre une nouvelle "époque". Utile si on suspecte que le hash-based
pattern s'est contaminé.

### 7.4 Orthogonalité : preuve

Pour deux expés sur layers différents avec salts différents,
```
  H1 = mmh3(layer1 || salt1 || user)
  H2 = mmh3(layer2 || salt2 || user)
```

`H1` et `H2` sont indépendants par la propriété avalanche de
MurmurHash3 : un bit différent dans l'input change ~50 % des bits de
l'output. `corr(H1 mod K, H2 mod K) ≤ 1/K` pour K suffisamment grand.

En pratique (K=10000), corrélation mesurée ~0.002, largement sous le
seuil 0.05.

### 7.5 Forced assignment (QA, founders)

Usage : on veut que Alice voie toujours variant B quand elle teste.
Géré via table `forced_assignments` + shortcut au top de `assign()`.

CLI ops :

```bash
efe-rollout forced add \
  --experiment exp-123 --user alice@example.com \
  --variant treatment_b --reason "QA"
```

Les forced assignments apparaissent avec `forced=true` dans la table
`exposures` et sont **exclus des analyses** par défaut.

### 7.6 Holdout global 1 %

Avant tout autre traitement, 1 % des users sont mis en "jamais exposé".
Ils reçoivent toujours le champion (reference model). Sert à :

- **Baseline long-terme** : détecter le drift global (model aging).
- **Contamination detection** : si les stats du groupe holdout
  changent sans qu'on ait pushé, il y a un bug (data leak, bug M06).
- **Causal baseline** : comparer la moyenne population vs holdout
  donne une estimation grossière de "l'effet cumulé de toutes les
  expés en prod".

---

## 8. Collection d'exposures &amp; outcomes

### 8.1 Exposures

Une **exposure** est un record "ce user a vu ce variant à cette
heure pour cette expé". Insérée dès que `assign()` retourne un variant
non-None (ou même None pour holdout — traçabilité).

Format :

```python
@dataclass(frozen=True, slots=True)
class Exposure:
    exposure_id: str
    experiment_id: str
    variant_id: str | None   # None = holdout
    tenant_id: str
    user_id: str
    assigned_at: datetime
    layer: str
    forced: bool
    holdout: bool
    request_id: str | None
    segment_hash: bytes | None
    hmac_sig: bytes          # HMAC over all fields above
```

HMAC signature permet de détecter la falsification en back-filling
d'exposures (cheating post-hoc).

### 8.2 Outcomes

Un **outcome** est une observation de métrique. Émis par M07
(l'inference adapter) ou par tout autre producteur :

```python
@dataclass(frozen=True, slots=True)
class Outcome:
    outcome_id: str
    experiment_id: str
    variant_id: str | None
    tenant_id: str
    user_id: str
    metric_name: str           # 'ctr', 'latency_ms', 'thumbs_up_rate' ...
    metric_value: float
    observed_at: datetime
    request_id: str | None
    covariate: dict[str, float]   # for CUPED + HTE
```

Association exposure ↔ outcome : via `(user_id, experiment_id)` +
proximité temporelle. Pas besoin de FK directe (les outcomes peuvent
arriver delayed — clic 2 h après exposition).

### 8.3 Channel d'ingestion

Deux chemins :

1. **Synchronous HTTP** (M07 → M08) : fire-and-forget POST sur
   `/v1/rollout/outcomes`. Latence < 5 ms p99.
2. **NATS** : publish sur `efe.rollout.outcome` ; M08 subscriber
   persiste. Backpressure géré par NATS JetStream.

Buffer local : chaque pod M07 buffer 100 outcomes max + flush every
1 s. Évite de saturer M08 sous burst.

### 8.4 Intent-to-Treat vs Compliance

**Intent-to-Treat (ITT)** : l'outcome compte même si le user n'a pas
vu le variant (e.g., il a quitté la page). Biais-libre mais dilué.

**Per-Protocol (PP)** : l'outcome compte seulement si le user a été
exposé. Potentiellement biaisé (self-selection) mais plus sensible.

M08 calcule les deux ; l'analyste choisit quelle métrique pré-enregistrer
dans le plan.

---

## 9. Analyse statistique

### 9.1 Frequentist : Welch t-test pour moyennes

```python
def welch_t_test(
    a: Sequence[float], b: Sequence[float], alpha: float = 0.05,
) -> TestResult:
    mean_a, mean_b = mean(a), mean(b)
    var_a, var_b = var(a, ddof=1), var(b, ddof=1)
    n_a, n_b = len(a), len(b)
    se = sqrt(var_a / n_a + var_b / n_b)
    t = (mean_b - mean_a) / se
    # Welch-Satterthwaite dof
    dof = (var_a / n_a + var_b / n_b) ** 2 / (
        (var_a ** 2) / (n_a ** 2 * (n_a - 1)) +
        (var_b ** 2) / (n_b ** 2 * (n_b - 1))
    )
    p_value = 2 * (1 - t_cdf(abs(t), dof))
    ci = (mean_b - mean_a - z_alpha * se,
          mean_b - mean_a + z_alpha * se)
    return TestResult(
        estimate=mean_b - mean_a, p_value=p_value, ci=ci,
    )
```

### 9.2 Frequentist : Wald pour proportions

```python
def wald_prop_test(
    s_a: int, n_a: int, s_b: int, n_b: int, alpha: float = 0.05,
) -> TestResult:
    p_a, p_b = s_a / n_a, s_b / n_b
    pooled = (s_a + s_b) / (n_a + n_b)
    se = sqrt(pooled * (1 - pooled) * (1/n_a + 1/n_b))
    z = (p_b - p_a) / se
    p_value = 2 * (1 - norm_cdf(abs(z)))
    # Newcombe score interval for difference
    ci = newcombe_ci(s_a, n_a, s_b, n_b, alpha)
    return TestResult(
        estimate=p_b - p_a, p_value=p_value, ci=ci,
    )
```

### 9.3 Bayesian : bêta-binomiale

```python
def bayesian_beta(
    s_a: int, n_a: int, s_b: int, n_b: int,
    prior_a: Beta, prior_b: Beta, n_samples: int = 100_000,
) -> BayesianResult:
    posterior_a = Beta(prior_a.alpha + s_a, prior_a.beta + (n_a - s_a))
    posterior_b = Beta(prior_b.alpha + s_b, prior_b.beta + (n_b - s_b))
    samples_a = posterior_a.sample(n_samples)
    samples_b = posterior_b.sample(n_samples)
    p_b_gt_a = mean(samples_b > samples_a)
    expected_lift = mean((samples_b - samples_a) / samples_a)
    credible_interval_95 = percentiles(
        samples_b - samples_a, [2.5, 97.5],
    )
    return BayesianResult(
        p_b_greater_than_a=p_b_gt_a,
        expected_lift=expected_lift,
        credible_interval=credible_interval_95,
    )
```

Decision rule : si `p_b_gt_a > 0.95` → ship ; si `p_b_gt_a < 0.05`
→ rollback ; autrement → continuer ou inconclusive.

### 9.4 CUPED (variance reduction)

Si une covariate `X` (e.g., métrique pre-experiment même-user) est
disponible :

```python
def apply_cuped(
    y: np.ndarray, x: np.ndarray,
) -> np.ndarray:
    theta = np.cov(y, x)[0, 1] / np.var(x)
    y_adj = y - theta * (x - np.mean(x))
    return y_adj
```

Appliquer AVANT le t-test. Variance réduite d'un facteur `(1 - ρ²)` où
`ρ = corr(Y, X)`. Typiquement `ρ ≈ 0.5-0.7` → variance réduite 25-50 %.

### 9.5 Sequential testing : α-spending

Problème : regarder les résultats plusieurs fois pendant l'expé
(peeking) inflate le faux-positif. Solution : **α-spending function**.

**Pocock** : allocation uniforme du budget α sur les K peeks :
```
    α_k = 1 - (1 - α)^(1/K)
```

**O'Brien-Fleming** : plus conservateur en début, plus large en fin :
```
    α_k = 2 × (1 - Φ(z_α/2 × √(K/k)))
```

M08 supporte les deux ; pre-reg spécifie `stopping_rule: 'pocock'` ou
`'obf'`.

### 9.6 Cluster-robust SE

Quand la randomisation est par user mais les outcomes sont per-request,
on doit prendre en compte la corrélation intra-user. Implémentation :

```python
def cluster_robust_se(
    y: np.ndarray, x: np.ndarray, clusters: np.ndarray,
) -> float:
    """Huber-White sandwich estimator."""
    import statsmodels.api as sm
    X = sm.add_constant(x)
    model = sm.OLS(y, X).fit(cov_type="cluster",
                               cov_kwds={"groups": clusters})
    return float(model.bse[1])
```

### 9.7 Corrections multiples (Benjamini-Hochberg)

Si on teste plusieurs métriques en parallèle, le risque de faux
positif gonfle. BH contrôle le **False Discovery Rate** :

```python
def bh_correction(p_values: list[float], alpha: float) -> list[bool]:
    sorted_p = sorted(enumerate(p_values), key=lambda t: t[1])
    m = len(p_values)
    threshold = [(i+1) / m * alpha for i in range(m)]
    rejected = [False] * m
    for i, (idx, p) in enumerate(sorted_p):
        if p <= threshold[i]:
            for j, (k, _) in enumerate(sorted_p[:i+1]):
                rejected[k] = True
    return rejected
```

### 9.8 Pipeline d'analyse

```
    raw outcomes + exposures
        │
        ▼
    apply ITT / PP filter
        │
        ▼
    (optional) apply CUPED
        │
        ▼
    compute test (welch / wald / bayesian)
        │
        ▼
    (if sequential) α-spending correction
        │
        ▼
    (if multiple metrics) BH correction
        │
        ▼
    produce Analysis row in DB + NATS event
```

---

## 10. Multi-armed bandits

### 10.1 Thompson Sampling (bêta-binomial)

Pour outcomes binaires (clic, conversion) :

```python
@dataclass
class BanditArm:
    variant_id: str
    alpha: float = 1.0
    beta: float = 1.0

class ThompsonSampler:
    def __init__(self, arms: list[BanditArm], rng: np.random.Generator):
        self._arms = arms
        self._rng = rng

    def choose(self) -> BanditArm:
        samples = [self._rng.beta(a.alpha, a.beta) for a in self._arms]
        return self._arms[int(np.argmax(samples))]

    def update(self, variant_id: str, reward: float) -> None:
        for a in self._arms:
            if a.variant_id == variant_id:
                if reward > 0:
                    a.alpha += reward
                else:
                    a.beta += 1 - reward
                return
```

### 10.2 UCB1 (Upper Confidence Bound)

Pour outcomes continus bornés :

```python
@dataclass
class UcbArm:
    variant_id: str
    n_pulls: int = 0
    mean_reward: float = 0.0

def ucb1_choose(arms: list[UcbArm], total_t: int) -> UcbArm:
    for a in arms:
        if a.n_pulls == 0:
            return a  # explore untried
    ucb_values = [
        a.mean_reward + sqrt(2 * log(total_t) / a.n_pulls)
        for a in arms
    ]
    return arms[int(np.argmax(ucb_values))]
```

### 10.3 Hybrid : A/B + bandit

Certaines expés démarrent en A/B fixe pour collecter statistiques
fiables (14 j), puis passent en bandit pour optimiser allocation
while continuing to learn. Transition automatique à `planned_end_at`.

### 10.4 Batch update vs online

**Online** (par request) : latence minimale, mais nécessite lock DB à
chaque update. Utilisable jusqu'à ~100 req/s par expé.

**Batch** (chaque 5 min) : daemon lit les outcomes non-processés,
update les compteurs, écrit en une transaction. Scalable à 100k req/s.

MVP : batch par default, online opt-in.

### 10.5 Garde-fous bandit

- **Minimum allocation** : chaque arm ≥ 5 % de trafic, même si domination
  écrasante (prévention du "arm lockout").
- **Exploration boost** : après k pulls, boost l'exploration via un
  epsilon (ε = 0.05) pour éviter la stagnation.
- **Reset on significant regret** : si un arm perd plus de X % de
  pulls en 24 h vs moyenne, reset partial posterior.

---

## 11. Guardrails &amp; auto-abort

### 11.1 Spécification guardrail

```python
@dataclass(frozen=True, slots=True)
class GuardrailSpec:
    metric_name: str
    direction: Literal["up", "down"]   # is increase bad or decrease bad?
    threshold_pct: float               # e.g. +20% means 20% worse than baseline
    window_s: int = 300                 # rolling window
    consecutive_breaches: int = 3
```

Examples canoniques :

- `latency_p95_ms`, direction=up, threshold_pct=50, window=300s.
- `error_5xx_rate`, direction=up, threshold_pct=100, window=300s.
- `cost_usd_per_req`, direction=up, threshold_pct=30.
- `thumbs_down_rate`, direction=up, threshold_pct=25.

### 11.2 Daemon de surveillance

```python
class GuardrailWatcher:
    async def tick(self) -> None:
        running = await self._running_experiments()
        for exp in running:
            for g in exp.guardrails:
                ratio = await self._compute_ratio(exp, g)
                if self._is_breach(ratio, g):
                    g.breaches_so_far += 1
                    if g.breaches_so_far >= g.consecutive_breaches:
                        await self._auto_abort(exp, g)
                else:
                    g.breaches_so_far = 0
```

### 11.3 Auto-abort

```python
async def _auto_abort(self, exp: Experiment, g: Guardrail) -> None:
    # 1. Transition state
    await registry.transition(exp.id, "running", "aborted")
    # 2. Publish NATS event
    await nats.publish("efe.rollout.aborted", {
        "experiment_id": exp.id,
        "reason": f"guardrail {g.metric_name} breached",
        "breaches": g.breaches_so_far,
    })
    # 3. Trigger M06 rollback (routing cut)
    await m06_rollback.execute(
        tenant_id=exp.tenant_id, task_id=exp.task_id,
        actor="rollout-watcher", reason=f"guardrail {g.metric_name}",
    )
    # 4. Audit event in M05 chain
    await audit.record_override(
        stage="rollout_auto_abort",
        metadata={"experiment_id": exp.id},
    )
```

Latence cible : breach detecté → rollback M06 démarré en < 30 s.
Rollback M06 complète en < 60 s (I7 de M06).

### 11.4 Faux positifs : aging

Guardrails ont un **aging** : si 24 h passent sans breach,
`breaches_so_far` reset à 0. Empêche le "effet tampon" qui
accumulerait des breaches isolés sur une longue durée.

---

## 12. Orthogonalité multi-layer

### 12.1 Rationale

On veut souvent tester N × M combinaisons (N prompts, M modèles).
Faire un "full factorial" avec une expé unique à N×M variants est
très coûteux en sample size. Solution : **layers orthogonaux**.

Chaque user est randomisé indépendamment dans chaque layer. Si les
effets sont additifs (ce qu'on suppose vrai), on peut analyser chaque
layer séparément → sample size = N + M au lieu de N×M.

### 12.2 Les 3 layers MVP

- **`prompt`** : variantes de template / system prompt / few-shot.
- **`model`** : variantes de poids (Llama-7B vs Mistral-7B).
- **`postproc`** : variantes de post-processing (tool-use retry,
  output filter, …).

### 12.3 Enforcement : 1 seule expé active par (tenant, task, layer)

Partial unique index (voir §6.1) :

```sql
CREATE UNIQUE INDEX uq_layer_one_active
    ON layer_allocations (tenant_id, task_id, layer)
    WHERE ended_at IS NULL;
```

→ Tenter de démarrer une 2ème expé sur le même slot → IntegrityError
côté application traduit en `LayerSlotBusy`.

### 12.4 Hypothèse d'additivité

On suppose que l'effet du variant `prompt.P1` + du variant `model.M1`
est `f(P1) + f(M1)` (additif). Si l'interaction est forte, la
décomposition layer-wise sous-estime (ou surestime) l'effet conjoint.

**Détection** : M08 calcule périodiquement un test d'interaction (ANOVA
avec terme d'interaction). Si p-value d'interaction < 0.05, marque
l'expé `interaction_detected: true` et recommande analyse conjointe.

### 12.5 Interaction explicite via `interaction_experiment`

Quand on VEUT mesurer une interaction, on crée une seule expé qui
exige les N×M cells sur un layer unique. Le système le supporte via
`layer="custom"`.

---

## 13. Segment analysis &amp; HTE

### 13.1 Heterogeneous Treatment Effects

Le treatment effect peut varier par segment (premium vs free, FR vs US,
desktop vs mobile). Reporter seulement l'ATE (Average Treatment
Effect) peut masquer ces hétérogénéités.

### 13.2 Segments prédéfinis

M08 ingère `covariate_json` sur chaque outcome :

```json
{
  "tenant_tier": "premium",
  "country": "FR",
  "device": "mobile",
  "first_week_user": true
}
```

Les champs sont hashés pour éviter high-cardinality metrics, mais
l'analyseur lit les champs bruts pour splitter.

### 13.3 Subgroup analysis

```python
def subgroup_analysis(
    outcomes: list[Outcome], segment_key: str,
) -> dict[str, TestResult]:
    groups = defaultdict(list)
    for o in outcomes:
        seg = o.covariate.get(segment_key, "__unknown__")
        groups[seg].append(o)
    return {
        seg: welch_t_test(
            *[o.metric_value for o in outs if o.variant == "control"],
            *[o.metric_value for o in outs if o.variant == "treatment"],
        )
        for seg, outs in groups.items()
    }
```

Avec BH correction si > 5 segments.

### 13.4 Heterogeneity test global

Test formel : interaction `variant × segment`. Si p < 0.05, segments
hétérogènes → publier Analysis de type `segment`.

---

## 14. Meta-analysis &amp; knowledge base

### 14.1 Archivage post-expé

Toute expé terminée (SHIPPED / ROLLBACK / INCONCLUSIVE) publie un
résumé dans `experiment_archive` :

```json
{
  "experiment_id": "...",
  "tenant_id": "acme",
  "hypothesis": "...",
  "primary_metric": "ctr",
  "decision": "ship",
  "ate": 0.032,
  "ci_95": [0.018, 0.046],
  "p_value": 0.003,
  "bayesian_p_b_gt_a": 0.998,
  "sample_size": {"control": 15000, "treatment": 15000},
  "duration_days": 14,
  "segments_tested": ["country","device"],
  "segment_hte_significant": ["country"],
  "interaction_detected": false,
  "notes": "CUPED applied, cluster SE"
}
```

### 14.2 Meta-analysis cross-experiment

Plusieurs expés sur le même `(tenant, task, metric)` peuvent être
agrégées via random-effects meta-analysis :

```
    θ_meta = Σ (w_i × θ_i) / Σ w_i,  w_i = 1 / (var_i + τ²)
```

où `τ²` est la variance between-study (DerSimonian-Laird estimator).

Utile pour :
- Découvrir "le prompt P1 améliore CTR cohéremment sur 3 expés".
- Évaluer "combien coûte chaque unité de latency en CTR en moyenne".

### 14.3 Negative results reporting

Le biais de publication (seuls les succès sont rapportés) pollue la
knowledge base. M08 **archive systématiquement** les résultats
négatifs (INCONCLUSIVE, ROLLBACK) avec autant de rigueur que les
SHIPPED. Un humain décide si le learning est intéressant à partager
cross-tenant.

---

## 15. NATS topics et flows

### 15.1 Topics produits par M08

| Topic | Payload | Consommateur |
|---|---|---|
| `efe.rollout.experiment.created` | `{experiment_id, spec_hash}` | M12 telemetry |
| `efe.rollout.experiment.registered` | idem + signature | M10 compliance |
| `efe.rollout.experiment.started` | idem + variants | M06 TenantRouter |
| `efe.rollout.assignment.overrides` | per-user forced list | M07 InferenceAdapter |
| `efe.rollout.guardrail.breached` | `{experiment_id, metric, ratio}` | M08 itself + M12 |
| `efe.rollout.aborted` | `{experiment_id, reason}` | M06 (→ trigger rollback) |
| `efe.rollout.analyzed` | `{experiment_id, decision, estimate, ci, p_value}` | M10, M12 |
| `efe.rollout.shipped` | idem | M06 (→ promote variant) |
| `efe.rollout.rollback_requested` | idem | M06 |

### 15.2 Topics consommés par M08

| Topic | Producteur | Action |
|---|---|---|
| `efe.inference.completed` | M07 | extrait metrics → `outcomes` |
| `efe.inference.safety.flagged` | M07 | consulte si guardrail activé |
| `efe.registry.routing.updated` | M06 | invalide cache assignments |
| `efe.gate.decisions.promote` | M05 | déclenche auto-creation d'une expé monitor post-prom |

### 15.3 Format envelope

Identique à M06/M07 : JSON avec `schema_version`, `event_type`,
`event_id`, `occurred_at`, `tenant_id`, `payload`, `hmac`.

**Domain tag** : `b"efe-rollout/nats/v1\\x00"` préfixé avant HMAC pour
isolation vs M06/M07/M08 audit events.

---

## 16. Interfaces publiques

### 16.1 Python (in-process SDK)

```python
from efe_rollout import RolloutClient

client = RolloutClient.from_env()

# Before making an inference call, get the user's variant:
variant = await client.get_variant(
    user_id="user-123",
    experiment_id="exp-summarize-prompt-v2",
)
if variant is None:
    # holdout — use reference model
    pass
else:
    # use variant.payload to pick model/prompt
    model_id = variant.payload["model_id"]

# Record the exposure (usually done automatically by M07):
await client.record_exposure(
    experiment_id="exp-summarize-prompt-v2",
    user_id="user-123",
    variant_id=variant.variant_id if variant else None,
    request_id="req-abc",
)

# After inference, record the outcome:
await client.record_outcome(
    experiment_id="exp-summarize-prompt-v2",
    user_id="user-123",
    metric_name="thumbs_up",
    metric_value=1.0,
    request_id="req-abc",
)

# Get the analysis on demand:
analysis = await client.get_analysis("exp-summarize-prompt-v2")
print(analysis.decision, analysis.estimate, analysis.p_value)
```

### 16.2 gRPC

```proto
service RolloutService {
    rpc CreateExperiment(ExperimentSpec) returns (ExperimentSummary);
    rpc PreRegister(PreRegisterRequest) returns (PreRegisterReply);
    rpc StartExperiment(ExperimentRef) returns (ExperimentSummary);
    rpc PauseExperiment(ExperimentRef) returns (ExperimentSummary);
    rpc AbortExperiment(AbortRequest) returns (ExperimentSummary);

    rpc Assign(AssignRequest) returns (AssignReply);
    rpc RecordExposure(ExposureRecord) returns (Ack);
    rpc RecordOutcome(OutcomeRecord) returns (Ack);

    rpc RequestAnalysis(AnalysisRequest) returns (Analysis);
    rpc StreamGuardrail(ExperimentRef) returns (stream GuardrailUpdate);
}
```

### 16.3 HTTP REST

| Méthode | Chemin | Usage |
|---|---|---|
| `POST` | `/v1/experiments` | create draft |
| `POST` | `/v1/experiments/{id}/design` | submit power analysis |
| `POST` | `/v1/experiments/{id}/pre-register` | seal the plan |
| `POST` | `/v1/experiments/{id}/start` | transition to RUNNING |
| `POST` | `/v1/experiments/{id}/pause` | |
| `POST` | `/v1/experiments/{id}/abort` | manual abort |
| `GET` | `/v1/experiments/{id}` | summary |
| `GET` | `/v1/experiments/{id}/variants` | |
| `GET` | `/v1/experiments/{id}/analysis` | on-demand computation |
| `GET` | `/v1/experiments/{id}/exposures?since=...` | audit |
| `POST` | `/v1/rollout/assign` | (user_id, experiment_id) → variant |
| `POST` | `/v1/rollout/exposures` | bulk ingest |
| `POST` | `/v1/rollout/outcomes` | bulk ingest |

### 16.4 CLI ops (`efe-rollout`)

```bash
efe-rollout experiments list --state running
efe-rollout experiments show exp-123
efe-rollout experiments power --baseline 0.05 --mde 0.1 --alpha 0.05 --power 0.8
efe-rollout experiments pre-register exp-123 --file plan.yaml --signing-key alias/efe-rollout
efe-rollout experiments start exp-123
efe-rollout experiments abort exp-123 --reason "incident P1"
efe-rollout analysis run exp-123 --kind final
efe-rollout forced add --experiment exp-123 --user alice --variant treatment
efe-rollout bandit inspect exp-456
```

---

## 17. Stratégie de tests

### 17.1 Unit tests (≥ 250)

```
tests_rollout/
  test_assignment_stable.py            I1 — determinism
  test_assignment_holdout_1pct.py      I8
  test_assignment_orthogonal_layers.py I4 corr ≤ 0.05 over 100k users
  test_assignment_forced_override.py
  test_state_machine_whitelist.py      FSM 12 transitions
  test_state_machine_cas.py            optimistic lock
  test_prereg_hmac_sealed.py           I6 hash + signature
  test_power_analysis_proportions.py   known-case agreement
  test_power_analysis_means.py
  test_welch_ttest_vs_scipy.py
  test_wald_prop_vs_statsmodels.py
  test_newcombe_ci_vs_paper_examples.py
  test_bayesian_beta_posterior.py
  test_cuped_variance_reduction.py     ≥ 25% reduction under synthetic data
  test_cluster_robust_se.py
  test_bh_correction.py
  test_pocock_alpha_spending.py
  test_obf_alpha_spending.py
  test_thompson_sampling_regret.py     O(sqrt(KT log T)) bound
  test_ucb1_regret.py
  test_bandit_minimum_allocation.py
  test_guardrail_spec_validation.py
  test_guardrail_rolling_window.py
  test_guardrail_consecutive_breaches.py
  test_guardrail_auto_abort.py
  test_guardrail_aging.py
  test_exposure_hmac_signed.py
  test_exposure_itt_vs_pp.py
  test_outcome_ingest_throughput.py
  test_layer_one_active_constraint.py
  test_layer_orthogonality_empirical.py
  test_segment_analysis_bh.py
  test_hte_interaction_test.py
  test_metaanalysis_dl_random_effects.py
  test_archive_shape.py
```

### 17.2 Integration (≥ 40)

```
tests_rollout_integration/
  test_e2e_ab_experiment_full_cycle.py
  test_e2e_bandit_converges_to_best_arm.py
  test_e2e_guardrail_triggers_m06_rollback.py
  test_e2e_multilayer_no_interference.py
  test_e2e_sequential_stops_at_peak.py
  test_e2e_holdout_never_exposed.py
  test_e2e_prereg_replay_attack_refused.py
  test_e2e_assignment_cache_invalidation.py
```

### 17.3 Chaos

```
tests_rollout_chaos/
  test_chaos_db_primary_kill_during_abort.py
  test_chaos_redis_flush_during_assign.py
  test_chaos_clock_skew_bandit_update.py
  test_chaos_ingester_burst_100k_rps.py
```

### 17.4 Couverture cible

- `assignment/` : 98 % (critique)
- `analysis/` : 95 %
- `guardrails/` : 95 %
- `bandits/` : 90 %
- Global : ≥ 90 %

---

## 18. Observabilité

### 18.1 Metrics Prometheus

```
# Lifecycle
efe_rollout_experiments_total{tenant, state}       (gauge)
efe_rollout_experiments_created_total{tenant}
efe_rollout_experiments_registered_total{tenant}
efe_rollout_experiments_aborted_total{tenant, reason}
efe_rollout_experiments_shipped_total{tenant}

# Assignment
efe_rollout_assign_latency_seconds                  (histogram)
efe_rollout_assign_total{tenant, experiment, outcome}
efe_rollout_holdout_total{tenant}
efe_rollout_forced_assignment_total{tenant}

# Ingestion
efe_rollout_exposure_ingest_total{tenant}
efe_rollout_outcome_ingest_total{tenant, metric}
efe_rollout_ingest_lag_seconds{tenant}              (histogram)

# Analysis
efe_rollout_analysis_latency_seconds                (histogram)
efe_rollout_analysis_decisions_total{decision}

# Bandits
efe_rollout_bandit_arm_pulls_total{experiment, variant}
efe_rollout_bandit_cumulative_reward{experiment, variant}
efe_rollout_bandit_update_latency_seconds

# Guardrails
efe_rollout_guardrail_checks_total{experiment, metric}
efe_rollout_guardrail_breaches_total{experiment, metric}
efe_rollout_guardrail_auto_aborts_total{tenant, reason}

# Orthogonality
efe_rollout_layer_interference_score{layer_pair}   (gauge, daily job)
```

### 18.2 Alerting rules

```yaml
- alert: RolloutGuardrailAutoAbort
  expr: rate(efe_rollout_guardrail_auto_aborts_total[10m]) > 0.01
  for: 0s
  labels: {severity: warning}
  annotations:
    summary: "Auto-aborts > 1% of experiments per hour"

- alert: RolloutAssignLatencyBudget
  expr: histogram_quantile(0.99,
            rate(efe_rollout_assign_latency_seconds_bucket[5m])) > 0.002
  for: 5m
  labels: {severity: warning}
  annotations:
    summary: "Assign p99 > 2ms"

- alert: RolloutIngestLag
  expr: histogram_quantile(0.95,
            rate(efe_rollout_ingest_lag_seconds_bucket[5m])) > 5
  for: 5m
  labels: {severity: warning}

- alert: RolloutOrthogonalityViolation
  expr: max(efe_rollout_layer_interference_score) > 0.1
  for: 1h
  labels: {severity: critical}
  annotations:
    summary: "Layers corr > 0.10 — check salts"
```

### 18.3 Dashboards

**Panels** :
1. Experiments by state (stacked area).
2. Assign p50/p95/p99 latency.
3. Exposure + outcome throughput.
4. Analysis decisions distribution (pie).
5. Guardrail auto-aborts timeline.
6. Bandit arm allocations (time-series per experiment).
7. Power analysis distribution (MDE histogram).
8. Layer interference heatmap.
9. Top shipped / rolled-back experiments (table).
10. Cost per experiment.

### 18.4 Traces OpenTelemetry

Root span : `rollout.assign`. Enfants : `cache.lookup`,
`forced.lookup`, `holdout.check`, `variant.compute`,
`exposure.enqueue`.

Root span : `rollout.analysis`. Enfants : `load.exposures`,
`load.outcomes`, `apply.cuped`, `compute.welch`, `compute.bayes`,
`persist`.

---

## 19. Plan de sprint M08

### 19.1 Découpage en 5 lots (10 jours ouvrés, 2 devs)

| Lot | Durée | Contenu | DRI |
|---|---|---|---|
| **L1 — Core experiment lifecycle** | 2 j | tables DDL, Experiment spec, FSM, pre-reg HMAC, API create/design/register/start | Dev A |
| **L2 — Assignment &amp; ingestion** | 2 j | `HashAssigner`, holdout 1%, forced, `ExposureRecorder`, `OutcomeRecorder`, Redis cache | Dev B |
| **L3 — Analysis** | 2 j | Welch, Wald, Bayesian bêta, CUPED, cluster SE, BH, α-spending | Dev A |
| **L4 — Bandits &amp; guardrails** | 2 j | Thompson, UCB1, ε-greedy, GuardrailWatcher, auto-abort → M06 | Dev B |
| **L5 — Layers &amp; observability** | 2 j | `LayerRegistry`, `LayerAllocation` constraint, metrics, dashboards, CLI | Dev A+B |

### 19.2 Dépendances amont

- M05 stable (audit chain, signing key management).
- M06 stable (`Rollback.execute()` gRPC).
- M07 stable (emits `efe.inference.completed` with tenant_id + user_id).

### 19.3 Definition of Done

- [ ] Tests unitaires verts, couverture ≥ 90 %.
- [ ] Tests d'intégration E2E verts.
- [ ] Assignment latency p99 < 2 ms mesuré.
- [ ] Auto-abort mesuré < 60 s end-to-end (guardrail breach → M06 rollback).
- [ ] Orthogonality test quotidien vert (corr ≤ 0.05).
- [ ] 10 alertes Alertmanager provisionnées.
- [ ] Dashboard Grafana importé.
- [ ] Runbooks : `runbook-guardrail-abort.md`, `runbook-prereg-forensics.md`,
  `runbook-bandit-stuck.md`.
- [ ] Docs opérateur + docs data-scientist publiées.

---

## 20. Critères d'acceptation

### 20.1 Fonctionnel

1. Une expé A/B classique (2 variants, 50/50) s'exécute de CREATE à
   SHIPPED sans intervention manuelle hors la décision finale.
2. Un bandit converge vers le bras optimal avec regret
   O(√(K·T·log T)) mesuré sur benchmark synthétique.
3. Un guardrail breach triple consécutif déclenche auto-abort + M06
   rollback en < 60 s.
4. 2 expés sur 2 layers différents s'exécutent en parallèle avec
   corrélation empirique ≤ 0.05 après 100k exposures.
5. Une tentative de démarrage d'une 3ème expé sur `(tenant, task,
   layer)` déjà occupé est refusée avec 409 Conflict.
6. Un attaquant modifiant `plan_json` dans `experiment_preregistrations`
   invalide la signature ; l'analyzer refuse de produire un résultat.
7. Le holdout 1 % ne reçoit jamais d'assignation variant sur toute la
   durée de vie d'une expé (vérifié par job quotidien).

### 20.2 Non-fonctionnel

1. Assign latency p99 < 2 ms.
2. Ingest throughput ≥ 10 000 exposure/s + 10 000 outcome/s par pod.
3. Analysis on-demand p95 < 10 s pour expé ≤ 1M observations.
4. Disponibilité 99.95 %/mois.

### 20.3 Sécurité &amp; compliance

1. Aucune donnée PII user dans les rows `exposures`/`outcomes` (tenant
   s'assure de hasher les user_ids en amont).
2. Pre-registration HMAC signé + vérifiable indépendamment.
3. Audit trail complet (chaque transition + chaque decision publiée
   dans `efe_gate` chain).
4. Aucune expé démarrée sans feu vert M05 + pre-reg signé.

### 20.4 Qualité d'ingénierie

1. mypy --strict vert.
2. ruff check zéro erreur.
3. OpenAPI 3.1 complet.
4. SDK Python (`efe-rollout`) typé + testé contre API.

---

## 21. Risques et mitigations

| Risque | Impact | Probabilité | Mitigation |
|---|---|---|---|
| **P-hacking** (analyste peek then decide) | faux positifs | Haute | pre-reg HMAC-scellée ; alpha-spending enforce ; audit |
| **Interaction non-détectée** | conclusions erronées | Moyenne | test d'interaction auto + flag ; pousser vers full-factorial si p<0.05 |
| **SRM** (Sample Ratio Mismatch : assignations non uniformes) | biais silencieux | Moyenne | χ² sur allocation observed vs designed ; alerte si p<0.001 |
| **Novelty effect** (users réagissent différemment au début) | ATE biaisé | Moyenne | prolonger l'expé ; analyser semaine 1 vs semaine 2 |
| **Bandit arm starvation** | moins optim que A/B | Moyenne | minimum 5 % allocation par arm |
| **Clock drift pour sequential** | α-spending incorrect | Faible | NTP + check ntpdate daily |
| **Redis flush → loss of cache** | spike latency | Faible | fallback DB + alert ; cache chaud rebuild < 1 min |
| **Cross-experiment contamination** (même user dans 2 expés qui se neutralise) | bias ATE | Moyenne | layer orthogonality enforce ; surveillance corr quotidienne |
| **HMAC key compromise** | forged pre-regs | Très faible | rotation + M05 KMS ; alerte sur changement `signing_key_id` |
| **Guardrail false positive** (legit traffic spike) | inutile abort | Moyenne | seuils conservateurs ; manual override 10-min window |
| **Metadata inflation** (trop de segments) | dashboard grow | Basse | allowlist de 10 segments max par tenant |
| **Schema drift outcomes** | test fail silently | Basse | pydantic strict + CI snapshot |

Runbooks (liste minimale) :

- `runbook-guardrail-auto-abort.md` — diagnose a triggered abort.
- `runbook-prereg-forensics.md` — verify seal integrity.
- `runbook-bandit-stuck.md` — unjam a bandit stuck on one arm.
- `runbook-srm-detection.md` — investigate sample ratio mismatch.
- `runbook-layer-salt-rotation.md`
- `runbook-retention-purge.md`

---

## 22. Annexes

### A. Helm values (extrait)

```yaml
rollout:
  replicaCount: 3
  image:
    repository: efe-registry.io/core/rollout
    tag: "1.0.0"
  resources:
    requests: { cpu: 500m, memory: 1Gi }
    limits:   { cpu: 2,    memory: 4Gi }

  holdout:
    global_pct: 1.0

  layers:
    - prompt
    - model
    - postproc
  layer_salts:
    prompt:   "salt-prompt-v1"
    model:    "salt-model-v1"
    postproc: "salt-postproc-v1"

  guardrails:
    default_window_s: 300
    default_consecutive_breaches: 3
    default_aging_s: 86400

  bandits:
    batch_update_interval_s: 300
    minimum_allocation_pct: 5
    explore_boost_pulls: 1000

  analysis:
    default_alpha: 0.05
    default_power: 0.8
    apply_cuped_if_covariate: true
    apply_cluster_robust_se: true

  ingest:
    exposure_buffer: 100
    outcome_buffer: 100
    flush_interval_s: 1
```

### B. Exemple pre-reg YAML

```yaml
experiment:
  name: "summarize-prompt-v2-test"
  tenant_id: acme
  task_id: summarize
  layer: prompt
  kind: ab_fixed

hypothesis: |
  Replacing the current zero-shot system prompt with a three-shot
  prompt will improve thumbs_up_rate by at least 5% (relative).

primary_metric: thumbs_up_rate
guardrails:
  - metric: latency_p95_ms
    direction: up
    threshold_pct: 30
  - metric: error_5xx_rate
    direction: up
    threshold_pct: 100
  - metric: cost_per_request_usd
    direction: up
    threshold_pct: 50

power_analysis:
  baseline: 0.45
  minimum_detectable_effect: 0.05      # relative
  alpha: 0.05
  power: 0.80
  planned_sample_size: 10000            # per variant

stopping_rule: fixed_horizon
planned_duration_days: 14

variants:
  - name: control
    is_control: true
    allocation_pct: 50
    payload:
      prompt_template: "prompt-zero-shot-v1"
  - name: treatment
    is_control: false
    allocation_pct: 50
    payload:
      prompt_template: "prompt-three-shot-v2"

covariates_for_cuped:
  - thumbs_up_rate_7d_prior
```

### C. Exemple analyse JSON

```json
{
  "experiment_id": "exp-summarize-prompt-v2-test",
  "kind": "final",
  "computed_at": "2026-05-04T10:12:00Z",
  "n_per_variant": {"control": 10283, "treatment": 10301},
  "method": "welch_t_test",
  "cuped_applied": true,
  "cluster_robust": true,
  "estimate": 0.0231,
  "ci_95": [0.0118, 0.0344],
  "p_value": 0.0003,
  "bayesian": {
    "posterior_p_b_gt_a": 0.997,
    "expected_lift_pct": 5.1,
    "credible_interval_95": [2.6, 7.7]
  },
  "decision": "ship",
  "segments": {
    "country=FR": {"estimate": 0.035, "p_value": 0.001},
    "country=US": {"estimate": 0.012, "p_value": 0.21}
  },
  "interaction_p_value": 0.04,
  "notes": "Segment effect detected (country); larger lift in FR."
}
```

### D. Script SQL : detect SRM daily

```sql
-- Run daily for each running experiment
WITH obs AS (
  SELECT variant_id, count(*) AS n
  FROM exposures
  WHERE experiment_id = :experiment_id
    AND assigned_at > now() - interval '24 hours'
    AND NOT forced
    AND NOT holdout
  GROUP BY variant_id
),
planned AS (
  SELECT variant_id, allocation_pct / 100.0 AS p
  FROM variants
  WHERE experiment_id = :experiment_id
),
chi AS (
  SELECT
    sum(
      (obs.n - (totals.total * planned.p)) ^ 2
      / nullif(totals.total * planned.p, 0)
    ) AS chisq
  FROM obs
  JOIN planned USING (variant_id),
  (SELECT sum(n) AS total FROM obs) totals
)
SELECT chisq,
       1 - chi2_cdf(chisq, (SELECT count(*) - 1 FROM obs)) AS p_value
FROM chi;
```

### E. Trigger Postgres : allocation_pct sum = 100

```sql
CREATE OR REPLACE FUNCTION check_variants_sum_100()
RETURNS TRIGGER AS $$
DECLARE
  total INTEGER;
BEGIN
  SELECT COALESCE(SUM(allocation_pct), 0) INTO total
  FROM variants
  WHERE experiment_id = NEW.experiment_id;
  IF total != 100 THEN
    RAISE EXCEPTION
      'variant allocations for experiment % sum to % (expected 100)',
      NEW.experiment_id, total;
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER trg_variants_sum_100
    AFTER INSERT OR UPDATE OR DELETE ON variants
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION check_variants_sum_100();
```

### F. Glossary M08

| Terme | Définition |
|---|---|
| **ATE** (Average Treatment Effect) | E[Y_treat] − E[Y_control] |
| **HTE** (Heterogeneous Treatment Effect) | ATE varying by segment |
| **ITT** (Intent-to-Treat) | analysis by assignment, regardless of compliance |
| **PP** (Per-Protocol) | analysis restricted to compliant users |
| **MDE** (Minimum Detectable Effect) | smallest effect we can detect with given power |
| **SRM** (Sample Ratio Mismatch) | observed allocation differs from designed |
| **CUPED** | Controlled-experiment Using Pre-Experiment Data — variance reduction |
| **α-spending** | sequential testing that controls cumulative α |
| **Pocock** | flat α-spending (uniform per peek) |
| **OBF** (O'Brien-Fleming) | conservative early, liberal late |
| **Regret** | cumulative reward loss vs best-arm baseline |
| **Thompson Sampling** | Bayesian bandit policy — probability matching |
| **UCB1** | frequentist bandit policy — upper confidence bound |
| **Layer** | orthogonal experimentation axis (prompt, model, postproc) |
| **Holdout** | subset of users never exposed to any experiment |
| **Pre-registration** | sealed, signed experimental plan filed before data collection |
| **Guardrail** | secondary metric whose regression triggers abort |

---

**Fin du document M08.**
*Version 1.0 — spec gelée pour implémentation — dernière revue 2026-04-20.*
*Toute évolution de contrat (Experiment spec, Analysis schema,
Guardrail semantics) passe par une ADR référençant ce document.*
