# M09 — Observability & Drift Monitoring (PSI)
## Architecture complète, ultra-détaillée
### (détection statistique de drift + alerting + baselines versionnées + intégration multi-scale)

---

**Document** : spécification d'architecture
**Module Python** : `efe_drift` (paquet) + sous-paquets `ingestion/`, `extraction/`, `aggregation/`, `baseline/`, `detection/`, `alerting/`, `catalog/`, `api/`, `db/`, `nats/`, `observability/`
**Sprint ciblé** : Sprint 9 (semaines 17-18) du plan MVP
**Dépend de** : M06 (Routing : lit le champion pour nommer les baselines par artefact), M07 (Inference : émet `efe.inference.completed` consommé ici), M08 (Rollout : émet `efe.rollout.outcome` ; M09 alimente `efe.drift.detected` → guardrail auto-abort M08)
**Consommé par** : M08 (guardrails peuvent lire un drift signal), M10 (Fairness : reçoit les analyses segmentées par attribut), M11 (API gateway), M12 (dashboards), M06 (rollback si drift critique)
**Statut** : spec gelée pour implémentation — aucune ambiguïté subsistante
**Invariants non-négociables** :
  - PSI calculé déterministiquement à partir d'un binning figé (invariant I2)
  - Baselines immuables une fois promues (I4) — la seule modification possible est la rotation
  - Isolation multi-tenant stricte (I5) — aucun feature d'un tenant ne traverse vers un autre
  - Bucketing temporel basé sur `observed_at` (I7) — jamais `ingested_at`
  - Alertes monotones en sévérité (I3) — plus de drift ⇒ plus grave, jamais l'inverse
  - Rotation baseline avec fenêtre de recouvrement ≥ N jours (I8) — pas de black-out
  - Aucune exposure de PII dans les histogrammes (I6) — binning + hashing systématiques
  - Détection multi-scale indépendante (1 h / 1 j / 7 j évalués séparément)

---

## Table des matières

1. [Mission et positionnement](#1-mission-et-positionnement)
2. [Scope MVP — ce qui est IN et OUT](#2-scope-mvp--ce-qui-est-in-et-out)
3. [Théorie : détection de drift rigoureuse](#3-théorie--détection-de-drift-rigoureuse)
4. [Vue d'ensemble — 8 plans](#4-vue-densemble--8-plans)
5. [Cycle de vie d'un baseline et d'un signal](#5-cycle-de-vie-dun-baseline-et-dun-signal)
6. [Modèle de données](#6-modèle-de-données)
7. [PSI — calcul et binning](#7-psi--calcul-et-binning)
8. [Autres divergences : KL, JS, KS, Wasserstein, χ²](#8-autres-divergences--kl-js-ks-wasserstein-χ²)
9. [Drift d'embeddings (espace vectoriel)](#9-drift-dembeddings-espace-vectoriel)
10. [Drift catégoriel et log-likelihood ratio](#10-drift-catégoriel-et-log-likelihood-ratio)
11. [Drift temporel multi-scale](#11-drift-temporel-multi-scale)
12. [Alerting et routage](#12-alerting-et-routage)
13. [Gestion des baselines](#13-gestion-des-baselines)
14. [Catalogue des features](#14-catalogue-des-features)
15. [NATS topics et flows](#15-nats-topics-et-flows)
16. [Interfaces publiques](#16-interfaces-publiques)
17. [Stratégie de tests](#17-stratégie-de-tests)
18. [Observabilité](#18-observabilité)
19. [Plan de sprint M09](#19-plan-de-sprint-m09)
20. [Critères d'acceptation](#20-critères-dacceptation)
21. [Risques et mitigations](#21-risques-et-mitigations)
22. [Annexes](#22-annexes)

---

## 1. Mission et positionnement

### 1.1 Mission fonctionnelle

M09 est la **plateforme de détection de drift** de l'EFE. Elle
répond à une question que ni M05 (offline gate), ni M06 (canary SLO),
ni M07 (inference SLO), ni M08 (expérimentation contrôlée) ne peuvent
résoudre :

> **« Le monde a-t-il changé sous nos pieds depuis qu'on a promu ce
> modèle ? »**

Autrement dit, M09 surveille continuellement la **distribution** des
inputs, outputs, et métadonnées de production, la compare à une
**baseline gouvernée**, et émet un **signal de drift** quand la
divergence franchit un seuil. Ce signal alimente :

1. Les **dashboards opérateurs** (Grafana) — visibilité temps-réel.
2. Les **alertes** (PagerDuty, Slack, webhook, NATS) — humains dans
   la boucle sous contraindre en moins de 5 s du breach.
3. Les **guardrails M08** — une expé peut consulter un drift signal
   comme métrique de garde-fou complémentaire au SLO.
4. Le **rollback M06** — un drift critique sur le champion déclenche
   un rollback automatique vers le challenger ou un artefact stable
   précédent.

### 1.2 Les trois flavors de drift

M09 modélise explicitement les trois types de drift formalisés dans
la littérature (Gama et al. 2014 ; Lu et al. 2020) :

| Flavor | Formalisation | Exemple EFE |
|---|---|---|
| **Covariate shift** | `P(X)` change, `P(Y‖X)` stable | Les utilisateurs posent désormais plus de prompts multilingues |
| **Label shift** | `P(Y)` change, `P(X‖Y)` stable | La proportion de prompts nécessitant du code monte vs texte |
| **Concept drift** | `P(Y‖X)` change | Une formulation qui menait à une bonne réponse il y a 6 mois échoue aujourd'hui (le LLM a été ré-entraîné ailleurs) |

M09 détecte les trois en combinant :

* Des tests **univariés** sur chaque feature : PSI, KS, χ² ;
* Des tests **multivariés** par classifier-based drift detection
  (entraînement d'un classifieur baseline-vs-courant, AUC comme
  signal) ;
* Des tests **sémantiques** dans l'espace d'embedding pour prompts
  et réponses (MMD à noyau RBF).

### 1.3 Distinction avec modules adjacents

| Module | Objectif | Temporalité | Signal de sortie |
|---|---|---|---|
| **M05 Gate** | feu vert offline (golden set) | pré-prod, one-shot | `decision: promote/reject` |
| **M06 Canary** | bascule progressive | intra-jour | `promotion_status` + auto-rollback sur SLO |
| **M07 Inference SLO** | latence / 5xx per-request | temps-réel | HTTP status + circuit breaker |
| **M08 Rollout** | comparer A vs B | semaine(s) | `estimate` + `decision: ship/rollback` |
| **M09 Drift** | **distribution de X vs baseline** | continu (heures → jours) | `drift_signal` + alerte |
| **M10 Fairness** | distribution par **groupe protégé** | continu | `fairness_report` par segment |

M09 et M10 sont proches — M10 est un cas particulier de M09 focalisé
sur les attributs protégés (âge, genre, pays). Dans la réalité, M10
réutilise 80 % de l'infrastructure M09 et ajoute une couche de
taxonomie spécifique (groupes protégés au lieu de features
arbitraires).

### 1.4 Position dans le pipeline EFE

```
                             ┌─────────────────────────┐
                             │  M11 Gateway (AuthN/Z)  │
                             └───────────┬─────────────┘
                                         │ /v1/chat ...
                                         ▼
                     ┌───────────────────────────────────┐
                     │   M07 Inference Adapter           │
                     └─────┬─────────────────────────┬───┘
                           │                         │
                 emit      │ emit                    │
        efe.inference.completed       efe.rollout.outcome
                           │                         │
                           ▼                         ▼
          ┌────────────────────────────────────────────────────┐
          │                   M09 Drift Service                 │
          │  ┌────────────┐  ┌──────────┐  ┌────────────────┐  │
          │  │ Ingestion  │  │ Extractor │  │ Aggregator     │  │
          │  └──────┬─────┘  └─────┬────┘  └────────┬───────┘  │
          │         │              │                 │          │
          │         ▼              ▼                 ▼          │
          │  ┌────────────────────────────────────────────┐     │
          │  │   Feature Snapshot Store  (TimescaleDB)    │     │
          │  └────────────┬───────────────────────────────┘     │
          │               │                                     │
          │               ▼                                     │
          │  ┌─────────────┐  ┌──────────────┐ ┌─────────────┐  │
          │  │ Baseline   │  │ Detector     │ │ AlertRouter │  │
          │  │ Manager    │  │  (PSI, MMD…) │ │             │  │
          │  └─────┬──────┘  └──────┬───────┘ └──────┬──────┘  │
          │        │                 │                │         │
          └────────┼─────────────────┼────────────────┼─────────┘
                   ▼                 ▼                ▼
         ┌─────────────────┐  ┌──────────────┐ ┌────────────────┐
         │  Postgres       │  │ NATS         │ │ PagerDuty /    │
         │  (metadata)     │  │ efe.drift.*  │ │ Slack / webhook │
         └─────────────────┘  └──────────────┘ └────────────────┘

 Amont : M06 publie `routing.updated` → rebind baseline ownership
 Aval  : M08 lit drift signal pour guardrail
         M06 lit critical-drift pour rollback auto
         M10 Fairness hérite des primitives
         M12 dashboards Grafana
```

### 1.5 Les trois boucles temporelles de M09

**Boucle hot (ingestion continue)**

```
  inference event (NATS) → extract features → write to TimescaleDB
  latency budget : p99 < 10 ms
```

**Boucle warm (aggregation par minute)**

```
  every 60 s : SELECT ... GROUP BY bucket → histograms per feature
  latency budget : p95 < 1 s for 1 M events/min
```

**Boucle cold (détection par heure / jour)**

```
  every 1 h : compute PSI(current_hour, baseline) for each feature
  every 1 j : same at day scale, with larger windows
  every 7 j : recompute baseline rotation eligibility
  latency budget : p95 < 10 s for 10⁴ features
```

Les trois boucles cohabitent sans se bloquer : l'ingestion écrit dans
TimescaleDB, l'aggregation lit et ré-écrit des agrégats, la détection
lit les agrégats et écrit les signaux.

### 1.6 Contraintes non-négociables

| Contrainte | Valeur | Preuve |
|---|---|---|
| PSI déterministe pour des inputs identiques | hash d'entrée → PSI identique bit-à-bit | test unitaire |
| Binning figé par baseline | bins stockés dans `baselines.bins_json` | seal SHA-256 |
| Baseline immuable post-promotion | UPDATE bloquée par trigger | test d'intégration |
| Alerting p99 < 5 s | breach détecté → message envoyé | mesure Prometheus |
| Ingestion p99 < 10 ms | histogramme `efe_drift_ingest_latency_seconds` | SLO |
| Pas de PII dans `feature_snapshots` | job linting quotidien | audit |
| Rotation baseline avec ≥ 3 j de recouvrement | `overlap_days` requis ≥ 3 | validation API |
| Retention max 90 j sur histogrammes bruts | partition-drop scheduler | runbook |

---

## 2. Scope MVP — ce qui est IN et OUT

### 2.1 IN au MVP (sprint 9, 2 semaines)

**Types de drift détectés** :

- **Univarié numérique** : PSI avec binning équifréquence (10 bins par
  défaut) ; KS (one-sample vs baseline CDF) ; Wasserstein-1
  1-dimensionnel.
- **Univarié catégoriel** : χ² goodness-of-fit ; log-likelihood
  ratio ; lift par catégorie.
- **Embedding sémantique** : MMD² à noyau RBF sur 384-dim
  (sentence-transformers MiniLM ou équivalent).
- **Multivarié** : classifier-based drift detection (logistic
  regression baseline vs current, AUC > 0.55 → drift).

**Features surveillées (extractors par défaut)** :

| Feature | Type | Extractor |
|---|---|---|
| `prompt_length_tokens` | numeric | comptage tokens vLLM |
| `response_length_tokens` | numeric | id |
| `latency_ms` | numeric | backend-reported |
| `cost_usd` | numeric | pricing table × tokens |
| `language` | categorical | lang-detect (fasttext) |
| `has_code` | categorical | heuristique regex triple-backtick |
| `prompt_embedding` | vector[384] | sentence-transformers |
| `response_embedding` | vector[384] | idem |
| `temperature` | numeric | request param |
| `top_p` | numeric | request param |

**Observations temporelles** :

- Buckets : 1 min (raw), 1 h (rollup), 1 j (rollup), 7 j (rollup).
- Rolling windows de détection : 1 h, 1 j, 7 j.
- Retention : 90 j brut, 365 j agrégat jour.

**Baseline** :

- Fenêtre de construction : 7 jours de production stable post-promotion.
- Rotation : recommandée tous les 30 jours, recouvrement 7 jours.
- Versioning : `baseline_id` stable avec `spec_hash` scellé.

**Alerting** :

- Seuils PSI : 0.10 (warning) / 0.25 (major) / 0.40 (critical).
- Canaux : webhook générique, Slack (via webhook), PagerDuty (via
  Events v2 API), NATS (`efe.drift.alert`).
- Hysteresis : pour éviter flapping, un signal doit rester au-dessus
  du seuil 3 buckets consécutifs.
- Suppression : pas de nouvelle alerte sur le même
  `(tenant, task, feature, level)` pendant 30 min après la précédente.

**API** :

- REST : CRUD baselines, read drift signals, list alert rules, ack
  incidents.
- SDK Python : `DriftClient.report_sample()`, `.get_drift()`,
  `.promote_baseline()`.

### 2.2 OUT au MVP

| Fonctionnalité | Raison | Jalon |
|---|---|---|
| Root-cause analysis (causal DAG) | recherche avancée, nécessite M10 + M14 | v1.2 |
| Auto-remediation (trigger re-train) | bouclage complet nécessite orchestration cross-cluster | v1.1 |
| Explicabilité par SHAP / LIME | OOS → M10 Fairness tooling | v1.2 |
| Détection distributed (fédéré multi-cluster) | un cluster suffit MVP | v1.1 |
| Streaming real-time anomaly (< 1 s) | pas de use-case MVP | v1.2 |
| Drift sur données non-texte (images, audio) | hors-périmètre LLM platform | — |
| Unsupervised embedding clustering shift | v1.2 | v1.2 |
| Fairness-specific drift (disparate impact) | M10 dédié | — |

### 2.3 Roadmap post-MVP

- **v1.1** : auto-remediation pipeline (trigger re-train via M05 gate),
  drift sur corpus RAG, federated multi-cluster.
- **v1.2** : causal attribution (drift due to which feature?), SHAP
  explainability, images/audio, unsupervised.
- **v2.0** : full observability mesh (unified signals with M08, M10,
  M11, M12), predictive drift (forecast next-week PSI).

---

## 3. Théorie : détection de drift rigoureuse

### 3.1 Le protocole scientifique en 6 étapes

**E1 — Feature catalog** : chaque feature surveillée est déclarée
explicitement avec type (numeric/categorical/embedding), extractor,
binning strategy, retention.

**E2 — Baseline** : construction sur N jours de prod stable
(par défaut N=7). Snapshot scellé SHA-256 + bins figés.

**E3 — Sampling continu** : chaque requête inference produit un ou
plusieurs échantillons de feature → snapshot par minute.

**E4 — Aggregation** : histogrammes par `(tenant, task, feature,
bucket)` — structures réduites, pas de PII.

**E5 — Détection** : PSI (ou divergence choisie) calculé entre
fenêtre courante et baseline. Test de signification pour certaines
métriques (KS, χ²).

**E6 — Alerting & action** : breach → signal publié → routeur →
humain ou M06/M08 automatisé.

### 3.2 Invariants statistiques (I1-I8)

**I1 — Déterminisme du calcul de drift**
> Pour une baseline donnée et un ensemble d'observations donné, le PSI
> (ou toute autre métrique M09) calculé deux fois produit des valeurs
> identiques bit-à-bit. Aucune source d'aléatoire non seedée.

**I2 — Stabilité du binning**
> Les bins d'un feature sont calculés une seule fois, au moment de la
> promotion du baseline, et figés dans `baselines.bins_json`. Toute
> observation future est classée selon ces bins (pas de re-binning
> adaptatif).

**I3 — Monotonie de sévérité**
> Plus la divergence entre baseline et current est grande, plus la
> sévérité de l'alerte est élevée. La fonction `severity(psi)` est
> monotone non-décroissante.

**I4 — Immutabilité des baselines promues**
> Une fois `state = 'active'`, un baseline ne peut plus être modifié.
> Seules les opérations `retire` et `rotate` sont autorisées. Toute
> tentative d'UPDATE sur un baseline actif est rejetée par trigger.

**I5 — Isolation multi-tenant**
> Aucun `feature_snapshot`, `baseline`, ou `drift_evaluation` d'un
> tenant ne peut être lu ou écrit par un autre. Les requêtes sont
> toujours scopées par `tenant_id` vérifié au gateway M11.

**I6 — Zéro-PII dans les snapshots**
> Les `feature_snapshots` ne contiennent que des **histogrammes
> agrégés** ou des **hash-bins**. Aucune valeur brute
> (prompt, réponse utilisateur) n'est persistée au-delà de
> l'embedding vectoriel normalisé. Un lint-job quotidien vérifie
> l'absence de chaînes de caractères "brutes".

**I7 — Bucketing temporel sur `observed_at`**
> Les snapshots agrègent selon l'heure d'observation, pas l'heure
> d'ingestion. Un événement arrivé en retard (e.g. 2 h de latence
> NATS) est bucketé dans l'heure de sa production, pas dans l'heure
> courante. Garantit la cohérence cross-boucles.

**I8 — Rotation avec recouvrement**
> Lors de la rotation d'un baseline `B_old → B_new`, il existe une
> fenêtre `overlap_days ≥ 3` pendant laquelle les deux baselines sont
> interrogeables. Empêche les "black-outs" où aucun baseline n'est
> disponible et toute détection échoue.

### 3.3 PSI — théorie formelle

Population Stability Index (Siddiqi 2005, bancaire) :

```
        K
PSI = ∑ (p_i - q_i) × ln(p_i / q_i)
       i=1
```

où :
- `K` : nombre de bins (10 par défaut)
- `p_i` : proportion des observations **courantes** dans le bin `i`
- `q_i` : proportion des observations **baseline** dans le bin `i`

Seuils industriels (Siddiqi, repris par FICO, Experian) :

| PSI | Interprétation | Action M09 |
|---|---|---|
| < 0.10 | Pas de shift significatif | — |
| 0.10 – 0.25 | Shift modéré | Warning |
| 0.25 – 0.40 | Shift important | Major |
| > 0.40 | Shift sévère | Critical + rollback candidate |

**Propriétés** :

- PSI ≥ 0 (toujours positif).
- PSI = 0 ssi `p_i = q_i` pour tout `i`.
- Symétrique : PSI(P, Q) = PSI(Q, P) (contrairement à KL).
- Sensible aux bins vides — nécessite smoothing ε (voir §7.3).

### 3.4 KS, Wasserstein, χ² — quand utiliser quoi

| Métrique | Input | Hypothèse nulle | Sortie | Test de signification |
|---|---|---|---|---|
| **PSI** | binned numeric / cat | P = Q | score ∈ [0, ∞) | non (seuils empiriques) |
| **KS** | continuous numeric | F_P = F_Q | D ∈ [0, 1] | oui (Kolmogorov distribution) |
| **Wasserstein-1** | continuous numeric | — | ≥ 0 (unités de X) | non (bootstrap) |
| **χ² goodness** | categorical | P = Q | χ² ∈ [0, ∞) | oui (χ² distribution) |
| **JS divergence** | any (binned) | P = Q | ∈ [0, ln 2] | non |
| **MMD² (RBF)** | vector | P = Q | ≥ 0 | oui (permutation bootstrap) |

**Règle de choix MVP** :

- Numeric univarié : PSI primaire, KS secondaire pour signification.
- Catégoriel : χ² primaire.
- Embedding : MMD² primaire.
- Multivariée générale : classifier-based (logistic reg).

### 3.5 MMD² et noyau RBF

Maximum Mean Discrepancy (Gretton et al. 2012) compare deux
distributions dans un espace de Hilbert à noyau reproduisant (RKHS) :

```
MMD²(P, Q) = E_{x,x'∼P}[k(x,x')] + E_{y,y'∼Q}[k(y,y')] − 2 E_{x∼P,y∼Q}[k(x,y)]
```

Avec noyau RBF `k(x, y) = exp(-‖x-y‖² / 2σ²)`, σ choisi par heuristique
médiane (median heuristic — σ² = median{‖x_i - x_j‖² : i ≠ j}).

Estimateur unbiased sur `n` samples de chaque distribution :

```
  MMD²_u = 1/(n(n-1)) ∑_{i≠j} k(x_i, x_j)
         + 1/(n(n-1)) ∑_{i≠j} k(y_i, y_j)
         − 2/n² ∑_{i,j} k(x_i, y_j)
```

Signification via bootstrap de permutation : on mélange `X ∪ Y`, on
redécoupe en deux moitiés, on recalcule MMD² sur les partitions
permutées. La p-value est la proportion de permutations dont le MMD²
dépasse l'observé.

### 3.6 Classifier-based drift detection (CDD)

Principe (Rabanser et al. 2019) : si on peut entraîner un classifier
qui distingue significativement les samples baseline des samples
current, alors les distributions diffèrent.

Protocole :

1. Échantillonner N points du baseline + N points de la fenêtre courante.
2. Label = 0 pour baseline, 1 pour current.
3. Split train/test 80/20.
4. Entraîner logistic regression.
5. Calculer AUC sur test.
6. AUC > 0.55 → drift significatif (après correction pour N).

**Avantages** : multivarié, sans hypothèse sur la forme des distributions.
**Inconvénients** : coût CPU non-trivial (quelques secondes sur 10 k
samples), nécessite suffisamment de données.

### 3.7 Budget temporel des opérations

| Opération | Budget | Notes |
|---|---|---|
| `ingest.process_event` | p99 < 10 ms | featurisation + write |
| `aggregate.minute_rollup` | p95 < 1 s | 1 min fenêtre, ~1 M events |
| `detect.psi_hour` | p95 < 10 s | ~10⁴ (tenant, task, feature) |
| `detect.mmd_embedding` | p95 < 30 s | bootstrap 100 permutations |
| `alert.dispatch` | p99 < 5 s | NATS + webhook + PagerDuty |
| `baseline.promote` | p95 < 1 min | validation + hash + seal |
| `baseline.rotate` | p95 < 5 min | recouvrement + diff + seal |

---

## 4. Vue d'ensemble — 8 plans

### 4.1 Ingestion Plane

- `NatsConsumer` : consomme `efe.inference.completed` et
  `efe.rollout.outcome`, désérialise l'envelope HMAC, pousse vers
  extracteur.
- `HttpSink` : endpoint REST `/v1/drift/samples` pour appels directs.
- `EventBuffer` : buffer asyncio avec backpressure, flush par
  batches de 1 000 ou 1 s.

### 4.2 Extraction Plane

- `FeatureExtractor` : `Protocol` pluggable par type.
- `NumericExtractor`, `CategoricalExtractor`, `EmbeddingExtractor`.
- `ExtractorRegistry` : mapping feature_name → extractor via
  `feature_catalog`.
- `EmbeddingModel` : sentence-transformers lazy-loaded (MiniLM-L6-v2).

### 4.3 Aggregation Plane

- `HistogramBuilder` : binning numérique avec bins pré-calculés.
- `CategoricalCounter` : dict de comptes par valeur.
- `EmbeddingReservoir` : reservoir sampling de 10 k vecteurs par
  fenêtre + centroid + covariance.
- `MinuteRollup` : job scheduler 60 s — materialise
  `feature_snapshots` rows.

### 4.4 Baseline Plane

- `BaselineCandidate` : accumulation post-promotion modèle (N jours).
- `BaselineValidator` : sanity checks (min sample size, coverage,
  absence d'outliers extrêmes).
- `BaselineStore` : persistence + seal.
- `RotationPolicy` : détection éligibilité rotation (âge, drift
  interne).

### 4.5 Detection Plane

- `DriftDetector` : façade avec méthode pluggable.
- `PsiDetector`, `KsDetector`, `WassersteinDetector`, `ChiSquaredDetector`,
  `MmdDetector`, `ClassifierDriftDetector`.
- `DetectionScheduler` : jobs APScheduler 1h / 1j / 7j.
- `DetectionCache` : mémoise résultats par `(baseline_hash,
  window_hash, method)`.

### 4.6 Alerting Plane

- `AlertEvaluator` : applique `AlertRule` sur signaux nouvellement
  produits.
- `AlertRouter` : multi-channel dispatcher.
- `Channels` : `SlackChannel`, `PagerDutyChannel`,
  `WebhookChannel`, `NatsChannel`.
- `AlertStateMachine` : firing → ack → resolved, avec hysteresis +
  suppression.

### 4.7 Catalog Plane

- `FeatureCatalog` : registry immuable versionné par
  `(feature_name, version)`.
- `SchemaValidator` : vérifie que chaque feature a un extractor
  déclaré avant ingestion.
- `PiiLinter` : job quotidien qui scanne les histogrammes pour
  détecter des strings "brutes" (qui auraient échappé au binning).

### 4.8 Observability Plane

- Metrics Prometheus (ingest latency, detection latency, alert count,
  baseline coverage).
- Logs structurés (audit des promotions / rotations).
- Dashboards Grafana dédiés.
- Traces OTel sur les jobs de détection (span parent par baseline
  evaluation).

---

## 5. Cycle de vie d'un baseline et d'un signal

### 5.1 FSM du baseline (6 états, 8 transitions)

```
      ┌─────────┐  start_accumulating     ┌──────────────┐
      │ PLANNED ├─────────────────────────▶ ACCUMULATING │
      └─────────┘                         └──────┬───────┘
                                                 │ enough_data
                                                 ▼
                                         ┌───────────────┐
                                         │   CANDIDATE   │
                                         └──────┬────────┘
                          reject                │ promote
                         ◀───────────────┐     ▼
                   ┌──────────┐    ┌─────────────┐  rotate_out
                   │ REJECTED │    │   ACTIVE    ├─────────────▶  ┌──────────┐
                   └──────────┘    └─────┬───────┘                │ RETIRING │
                                         │ emergency_retire       └────┬─────┘
                                         ▼                             │ overlap_expired
                                   ┌───────────┐                       ▼
                                   │  RETIRED  │◀──────────────────────┘
                                   └───────────┘
```

**Transitions autorisées** :

| Depuis | Vers | Garde | Acteur |
|---|---|---|---|
| PLANNED | ACCUMULATING | promotion modèle détectée | auto |
| ACCUMULATING | CANDIDATE | coverage ≥ N jours & min sample met | auto |
| CANDIDATE | ACTIVE | sanity OK + signature scellée | owner/admin |
| CANDIDATE | REJECTED | sanity fail ou manuel | owner/admin |
| ACTIVE | RETIRING | nouvelle baseline promue | auto via rotation |
| ACTIVE | RETIRED | emergency (admin, drift critique propre) | admin |
| RETIRING | RETIRED | overlap_days écoulés | auto |
| PLANNED | REJECTED | cancel manuel | admin |

Terminaux : `REJECTED`, `RETIRED`.

### 5.2 FSM du signal de drift (4 états, 5 transitions)

```
  ┌───────────┐ threshold_crossed ┌───────────┐ window_passed ┌─────────┐
  │ OBSERVING ├────────────────────▶  FIRING  ├───────────────▶ RESOLVED │
  └───────────┘                    └─────┬────┘               └─────────┘
       ▲                                 │ acked
       │                                 ▼
       │                           ┌──────────┐   no_more_breach
       └───────────────────────────┤   ACKED  ├───────────────▶ RESOLVED
                                   └──────────┘
```

- `OBSERVING` : état par défaut, signal sous seuil.
- `FIRING` : seuil franchi en montée, alerte émise.
- `ACKED` : opérateur a acquitté via API/UI, l'alerte ne re-dispatch
  pas tant que FIRING dure, mais on continue à tracker.
- `RESOLVED` : la fenêtre de 1 h suivant la dernière breach est passée
  sans dépasser le seuil.

### 5.3 Effets de bord par transition

- `CANDIDATE → ACTIVE` :
  - `spec_hash` SHA-256 calculé et scellé.
  - Bins figés dans `baselines.bins_json`.
  - NATS `efe.drift.baseline_promoted` publié.
  - Ancienne baseline (si elle existait) passe `ACTIVE → RETIRING`.

- `ACTIVE → RETIRING` :
  - Nouvelle baseline `ACTIVE` coexiste `overlap_days`.
  - Détection continue de calculer des signaux **vs les deux**
    baselines pendant l'overlap.
  - Au terme de l'overlap, `RETIRING → RETIRED`.

- `OBSERVING → FIRING` :
  - `alert_incidents` row créée.
  - NATS `efe.drift.alert` publié.
  - Dispatch vers canaux selon `alert_rules.routing_json`.

### 5.4 Idempotence

Toutes les transitions baseline utilisent CAS (`state_version`)
identique aux patterns M06/M08. Les signaux de drift sont
idempotents par `(baseline_id, window_end, feature_name, method)`.

---

## 6. Modèle de données

### 6.1 DDL Postgres (DDL complet)

```sql
-- ============================================================
-- feature_catalog — registry de features surveillées
-- ============================================================
CREATE TABLE feature_catalog (
    feature_id       UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id        TEXT NOT NULL,
    task_id          TEXT NOT NULL,
    name             TEXT NOT NULL,
    version          INTEGER NOT NULL DEFAULT 1,
    kind             TEXT NOT NULL CHECK (kind IN (
                       'numeric','categorical','embedding','binary')),
    extractor        TEXT NOT NULL,      -- e.g. 'token_length', 'lang_detect'
    binning_strategy TEXT NOT NULL CHECK (binning_strategy IN (
                       'equifrequency','equiwidth','custom','none')),
    expected_cardinality INTEGER,        -- NULL for numeric/embedding
    retention_days   INTEGER NOT NULL DEFAULT 90,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_by       TEXT NOT NULL,
    notes            TEXT,
    UNIQUE (tenant_id, task_id, name, version)
);
CREATE INDEX ix_feature_catalog_tenant_task ON feature_catalog (tenant_id, task_id);

-- ============================================================
-- baselines — snapshots promus servant de référence
-- ============================================================
CREATE TABLE baselines (
    baseline_id        UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id          TEXT NOT NULL,
    task_id            TEXT NOT NULL,
    feature_id         UUID NOT NULL REFERENCES feature_catalog(feature_id),
    model_artifact_id  TEXT,                    -- binds to M06 artifact
    state              TEXT NOT NULL CHECK (state IN (
                         'planned','accumulating','candidate',
                         'active','retiring','retired','rejected')),
    state_version      BIGINT NOT NULL DEFAULT 0,
    window_start       TIMESTAMPTZ,
    window_end         TIMESTAMPTZ,
    sample_count       BIGINT,
    bins_json          JSONB,             -- frozen bin edges
    centroid_json      JSONB,             -- for embeddings
    covariance_json    JSONB,             -- for embeddings (diag only MVP)
    stats_json         JSONB,             -- mean/std/quantiles
    spec_hash          BYTEA,             -- SHA-256 of the sealed spec
    signing_key_id     TEXT,
    signature          BYTEA,
    planned_end_at     TIMESTAMPTZ,       -- expiration / rotation trigger
    overlap_days       INTEGER NOT NULL DEFAULT 3,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    promoted_at        TIMESTAMPTZ,
    retired_at         TIMESTAMPTZ,
    UNIQUE (tenant_id, task_id, feature_id, window_start, window_end)
);
CREATE INDEX ix_baselines_state_tenant ON baselines (state, tenant_id);
CREATE INDEX ix_baselines_feature_active
    ON baselines (feature_id)
    WHERE state IN ('active','retiring');

-- ============================================================
-- feature_snapshots — histogrammes agrégés par bucket temporel
-- Partitionné par jour en prod.
-- ============================================================
CREATE TABLE feature_snapshots (
    snapshot_id      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id        TEXT NOT NULL,
    task_id          TEXT NOT NULL,
    feature_id       UUID NOT NULL REFERENCES feature_catalog(feature_id),
    bucket_kind      TEXT NOT NULL CHECK (bucket_kind IN (
                       'minute','hour','day','week')),
    bucket_start     TIMESTAMPTZ NOT NULL,
    bucket_end       TIMESTAMPTZ NOT NULL,
    sample_count     BIGINT NOT NULL,
    histogram_json   JSONB NOT NULL,       -- [count_per_bin]
    stats_json       JSONB,                -- mean, std, quantiles
    embedding_ref    TEXT,                 -- blob URL if kind=embedding
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now()
) PARTITION BY RANGE (bucket_start);

CREATE INDEX ix_snapshots_feat_bucket
    ON feature_snapshots (feature_id, bucket_kind, bucket_start);

-- ============================================================
-- drift_evaluations — output des détecteurs
-- ============================================================
CREATE TABLE drift_evaluations (
    evaluation_id    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id        TEXT NOT NULL,
    task_id          TEXT NOT NULL,
    feature_id       UUID NOT NULL REFERENCES feature_catalog(feature_id),
    baseline_id      UUID NOT NULL REFERENCES baselines(baseline_id),
    window_start     TIMESTAMPTZ NOT NULL,
    window_end       TIMESTAMPTZ NOT NULL,
    method           TEXT NOT NULL CHECK (method IN (
                       'psi','ks','wasserstein','chi_squared',
                       'js_divergence','mmd','classifier_based')),
    score            DOUBLE PRECISION NOT NULL,
    p_value          DOUBLE PRECISION,      -- null for PSI/Wasserstein
    severity         TEXT NOT NULL CHECK (severity IN (
                       'none','warning','major','critical')),
    details_json     JSONB,                 -- bin-by-bin deltas, etc.
    computed_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (baseline_id, window_start, window_end, method)
);
CREATE INDEX ix_evals_tenant_time
    ON drift_evaluations (tenant_id, computed_at);
CREATE INDEX ix_evals_feature_time
    ON drift_evaluations (feature_id, computed_at);

-- ============================================================
-- alert_rules — configuration utilisateur des seuils et canaux
-- ============================================================
CREATE TABLE alert_rules (
    rule_id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id        TEXT NOT NULL,
    task_id          TEXT,
    feature_id       UUID REFERENCES feature_catalog(feature_id),
    method           TEXT NOT NULL,
    threshold_warning DOUBLE PRECISION,
    threshold_major  DOUBLE PRECISION,
    threshold_critical DOUBLE PRECISION,
    window_kind      TEXT NOT NULL DEFAULT 'hour',
    consecutive_breaches INTEGER NOT NULL DEFAULT 3,
    suppression_s    INTEGER NOT NULL DEFAULT 1800,
    routing_json     JSONB NOT NULL,       -- {"slack":{...}, "pagerduty":{...}}
    enabled          BOOLEAN NOT NULL DEFAULT true,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ix_alert_rules_feature ON alert_rules (feature_id) WHERE enabled;

-- ============================================================
-- alert_incidents — ledger des alertes tirées
-- ============================================================
CREATE TABLE alert_incidents (
    incident_id      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    rule_id          UUID NOT NULL REFERENCES alert_rules(rule_id),
    baseline_id      UUID NOT NULL REFERENCES baselines(baseline_id),
    feature_id       UUID NOT NULL REFERENCES feature_catalog(feature_id),
    state            TEXT NOT NULL CHECK (state IN (
                       'firing','acked','resolved')),
    severity         TEXT NOT NULL,
    score_at_fire    DOUBLE PRECISION NOT NULL,
    fired_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    acked_at         TIMESTAMPTZ,
    acked_by         TEXT,
    resolved_at      TIMESTAMPTZ,
    dispatch_results JSONB,               -- per-channel dispatch audit
    UNIQUE (rule_id, baseline_id, fired_at)
);
CREATE INDEX ix_incidents_state ON alert_incidents (state, fired_at);

-- ============================================================
-- baseline_state_transitions — audit journal
-- ============================================================
CREATE TABLE baseline_state_transitions (
    transition_id    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    baseline_id      UUID NOT NULL REFERENCES baselines(baseline_id),
    from_state       TEXT NOT NULL,
    to_state         TEXT NOT NULL,
    actor            TEXT NOT NULL,
    reason           TEXT,
    occurred_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ix_baseline_transitions
    ON baseline_state_transitions (baseline_id, occurred_at);

-- ============================================================
-- Trigger : interdire modification d'une baseline ACTIVE (I4)
-- ============================================================
CREATE OR REPLACE FUNCTION enforce_baseline_immutable()
RETURNS TRIGGER AS $$
BEGIN
  IF OLD.state = 'active' AND NEW.state = 'active' THEN
    -- allow only state column changes triggered by rotation workflow
    IF OLD.spec_hash IS DISTINCT FROM NEW.spec_hash
       OR OLD.bins_json::text IS DISTINCT FROM NEW.bins_json::text
       OR OLD.centroid_json::text IS DISTINCT FROM NEW.centroid_json::text THEN
      RAISE EXCEPTION 'baseline % is ACTIVE and immutable', OLD.baseline_id;
    END IF;
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_baseline_immutable
BEFORE UPDATE ON baselines
FOR EACH ROW EXECUTE FUNCTION enforce_baseline_immutable();
```

### 6.2 Schéma TimescaleDB (continuous aggregates)

```sql
-- Le rollup minute est maintenu par un continuous aggregate
-- TimescaleDB pour la performance (query < 100 ms sur 30 j).

CREATE TABLE raw_events (
    event_id    UUID NOT NULL,
    tenant_id   TEXT NOT NULL,
    task_id     TEXT NOT NULL,
    feature_id  UUID NOT NULL,
    observed_at TIMESTAMPTZ NOT NULL,
    value_num   DOUBLE PRECISION,
    value_cat   TEXT,
    value_emb   BYTEA            -- serialized vector
);

SELECT create_hypertable('raw_events', 'observed_at',
    chunk_time_interval => INTERVAL '1 hour');

CREATE MATERIALIZED VIEW snapshots_1min
WITH (timescaledb.continuous) AS
SELECT
  tenant_id, task_id, feature_id,
  time_bucket('1 minute', observed_at) AS bucket_start,
  count(*) AS sample_count,
  avg(value_num) AS mean,
  stddev_pop(value_num) AS std,
  percentile_cont(0.5) WITHIN GROUP (ORDER BY value_num) AS p50,
  percentile_cont(0.95) WITHIN GROUP (ORDER BY value_num) AS p95
FROM raw_events
GROUP BY tenant_id, task_id, feature_id, bucket_start;

SELECT add_continuous_aggregate_policy('snapshots_1min',
    start_offset => INTERVAL '7 days',
    end_offset   => INTERVAL '1 minute',
    schedule_interval => INTERVAL '30 seconds');
```

### 6.3 Schéma Redis (hot cache)

```
# Drift score cache (hot path for dashboards)
drift:score:{tenant}:{task}:{feature}:{method}:{window}       STRING  (JSON)
drift:score:{tenant}:{task}:{feature}:{method}:{window}:ts    STRING  (epoch)
  TTL = 5 min

# Active baseline pointer
drift:baseline:{tenant}:{task}:{feature}                       STRING  (baseline_id)

# Alert suppression lockouts
drift:alert:suppress:{tenant}:{feature}:{level}                STRING
  TTL = rule.suppression_s

# Consecutive-breach counter (anti-flapping)
drift:breach:{tenant}:{feature}:{rule_id}                      HASH
  {count, last_breach_ms}

# PubSub channels (dashboard live refresh)
channel drift.score.updated
channel drift.alert.firing
channel drift.baseline.promoted
```

### 6.4 Partitioning + retention

- `raw_events` : hypertable, chunks horaires, retention 7 jours
  (agrégés dans `snapshots_1min` avant purge).
- `feature_snapshots` : partitionnée par jour, retention configurable
  par `feature_catalog.retention_days` (default 90).
- `drift_evaluations` : pas de partition, retention 365 j.
- `alert_incidents` : pas de partition, retention 365 j.

Job nightly `drift-retention-cleanup` purge les partitions expirées.

---

## 7. PSI — calcul et binning

### 7.1 Algorithme de référence

```python
def compute_psi(
    current_counts: list[int],
    baseline_counts: list[int],
    eps: float = 1e-6,
) -> float:
    """Population Stability Index entre deux histogrammes alignés.

    * ``current_counts`` et ``baseline_counts`` ont la même longueur
      (= nombre de bins).
    * ``eps`` est l'epsilon de smoothing, ajouté à chaque proportion
      pour éviter ``log(0)`` / divergence infinie sur bins vides.
    """
    if len(current_counts) != len(baseline_counts):
        raise ValueError("bin count mismatch")
    n_cur = sum(current_counts)
    n_base = sum(baseline_counts)
    if n_cur == 0 or n_base == 0:
        return float("nan")
    psi = 0.0
    for c, b in zip(current_counts, baseline_counts, strict=True):
        p = max(eps, c / n_cur)
        q = max(eps, b / n_base)
        psi += (p - q) * math.log(p / q)
    return psi
```

**Propriétés** :

- Le smoothing `eps = 1e-6` équivaut à ajouter ~1 observation fictive
  par bin — évite l'explosion quand un bin est vide mais reste
  négligeable sur des histogrammes de ≥ 10 k observations.
- Avec `K = 10` bins, PSI max théorique ≈ `2 × ln(1/eps)` quand toute
  la distribution a migré vers un bin vide dans la baseline.
- PSI = 0 quand `p_i = q_i` pour tout `i` (à `eps` près).

### 7.2 Stratégies de binning

M09 supporte **trois** stratégies, choisies à la promotion du baseline :

**A. Equifrequency (quantile-based) — DEFAULT**

Les bins sont définis par les quantiles du baseline. Avec `K = 10`,
chaque bin contient ~10 % des observations baseline.

```
bins = [q_0, q_10, q_20, ..., q_100]
```

Avantages : stable numériquement, robuste aux outliers.
Inconvénient : coût CPU à la promotion (tri complet).

**B. Equiwidth**

Les bins ont largeur identique sur `[min_base, max_base]`. Plus rapide
mais sensible à la queue.

**C. Custom**

L'utilisateur fournit les bin edges explicitement via l'API
`POST /v1/drift/baselines/{id}/bins`.

**Invariant** : une fois la baseline `ACTIVE`, les bins sont figés
dans `baselines.bins_json`. Toute observation future est classée
selon ces bin edges, **sans re-binning adaptatif** (I2).

### 7.3 Smoothing et edge cases

| Edge case | Traitement |
|---|---|
| `current_count == 0` sur un bin | `p = eps` |
| `baseline_count == 0` sur un bin | `q = eps` (≈ Laplace add-one scaled) |
| Nouvelle valeur hors `[min_base, max_base]` | bucketée dans bin_0 ou bin_K-1 |
| `n_cur < min_sample_size` | PSI renvoie NaN, détecteur skip |
| Tous les comptes identiques (dist uniforme) | PSI ≈ 0 |

### 7.4 Signification statistique du PSI

PSI n'a pas de distribution théorique exploitable — les seuils sont
**empiriques**. Pour obtenir une p-value approximative, M09 propose
un bootstrap optionnel :

```python
def psi_bootstrap_p_value(
    current_values: list[float], baseline_counts: list[int],
    bins: list[float], n_iter: int = 1000, seed: int = 20260420,
) -> tuple[float, float]:
    """Retourne (psi_observé, p_value_approx).

    Ré-échantillonne ``current_values`` avec remplacement et calcule
    la proportion de PSI simulés sous H0 qui dépasse l'observé.
    H0 étant construite en tirant depuis baseline_counts.
    """
    import random
    rng = random.Random(seed)
    observed = compute_psi(
        counts_from(current_values, bins), baseline_counts,
    )
    exceedances = 0
    for _ in range(n_iter):
        # Sample n_cur points from baseline distribution
        ...
    p_value = exceedances / n_iter
    return observed, p_value
```

Par défaut MVP : le bootstrap est désactivé (coût CPU), les seuils
empiriques suffisent.

### 7.5 Exemple numérique complet

Baseline (10 bins, 10 000 observations) :

```
counts_base = [1000, 1000, 1000, 1000, 1000, 1000, 1000, 1000, 1000, 1000]
```

Current (distribution shift vers les grands valeurs) :

```
counts_cur  = [ 200,  400,  800, 1000, 1000, 1200, 1500, 1800, 1100, 1000]
              n_cur = 10000
```

Calcul :

| bin | p_i | q_i | (p-q) ln(p/q) |
|---|---|---|---|
| 0 | 0.02 | 0.10 | -0.08 × (-1.609) = 0.129 |
| 1 | 0.04 | 0.10 | -0.06 × (-0.916) = 0.055 |
| 2 | 0.08 | 0.10 | -0.02 × (-0.223) = 0.004 |
| 3 | 0.10 | 0.10 | 0 |
| 4 | 0.10 | 0.10 | 0 |
| 5 | 0.12 | 0.10 | 0.02 × 0.182 = 0.004 |
| 6 | 0.15 | 0.10 | 0.05 × 0.405 = 0.020 |
| 7 | 0.18 | 0.10 | 0.08 × 0.588 = 0.047 |
| 8 | 0.11 | 0.10 | 0.01 × 0.095 = 0.001 |
| 9 | 0.10 | 0.10 | 0 |

**PSI = 0.260 → major shift** (franchit 0.25, warning sur 0.10).

---

## 8. Autres divergences : KL, JS, KS, Wasserstein, χ²

### 8.1 KL divergence (asymétrique)

```
KL(P ‖ Q) = ∑_i p_i × ln(p_i / q_i)
```

Lien avec PSI : `PSI(P, Q) = KL(P ‖ Q) + KL(Q ‖ P)` — c'est la KL
**symétrisée non-normalisée**.

M09 expose KL comme métrique auxiliaire (utile pour distinguer quel
coté dévie) mais n'alerte pas dessus.

### 8.2 Jensen-Shannon divergence (symétrique, bornée)

```
M = (P + Q) / 2
JS(P, Q) = 0.5 × KL(P ‖ M) + 0.5 × KL(Q ‖ M)
```

Propriétés :

- Symétrique : `JS(P, Q) = JS(Q, P)`.
- Bornée : `JS ∈ [0, ln 2]`.
- `sqrt(JS)` est une distance métrique.

M09 expose JS comme alternative à PSI pour les distributions très
asymétriques, avec seuils :

| JS | Sévérité |
|---|---|
| < 0.01 | none |
| 0.01 – 0.05 | warning |
| 0.05 – 0.15 | major |
| > 0.15 | critical |

### 8.3 Test de Kolmogorov-Smirnov (KS)

Pour distributions continues :

```
D = sup_x |F_current(x) - F_baseline(x)|
```

Sous H₀, `sqrt(n_cur × n_base / (n_cur + n_base)) × D` suit la
distribution de Kolmogorov.

Avantages :
- Pas de binning (distribution libre).
- Signification exacte.

Inconvénients :
- 1D uniquement.
- Peu sensible aux queues de distribution.

### 8.4 Wasserstein-1 (Earth-Mover's distance)

En 1D :

```
W_1(P, Q) = ∫_0^1 |F_P^-1(u) - F_Q^-1(u)| du
         = ∫_ℝ |F_P(x) - F_Q(x)| dx
```

Interprétation : le "coût" minimum pour transformer P en Q en
déplaçant de la "masse". Unité = unité de X (e.g. ms pour latence).

Complémentaire au PSI : Wasserstein mesure l'**ampleur géométrique**
du shift, PSI mesure la **difficulté probabiliste**. Une distribution
déplacée de +100 ms uniformément donne un Wasserstein non-nul mais
un PSI potentiellement faible (si les deux distributions ont formes
similaires).

### 8.5 χ² goodness-of-fit (catégoriel)

```
χ² = ∑_{k=1}^K (O_k - E_k)² / E_k
```

Où `O_k` est le comptage observé et `E_k = n_cur × q_k` le comptage
attendu sous baseline.

Sous H₀, χ² suit χ² à (K-1) degrés de liberté.

Seuils M09 (α = 0.05) :

- K = 2 : χ² > 3.84 → warning
- K = 5 : χ² > 9.49 → warning
- K = 10 : χ² > 16.92 → warning

Sévérité basée sur p-value :

| p-value | Sévérité |
|---|---|
| > 0.05 | none |
| 0.01 – 0.05 | warning |
| 0.001 – 0.01 | major |
| < 0.001 | critical |

### 8.6 Log-likelihood ratio (G-test)

Alternative à χ² plus robuste aux petits échantillons :

```
G = 2 × ∑_k O_k × ln(O_k / E_k)
```

Converge vers χ² en loi pour grands échantillons, mais G est
asymptotiquement optimal au sens de Neyman-Pearson.

---

## 9. Drift d'embeddings (espace vectoriel)

### 9.1 Pourquoi l'embedding drift est spécial

Les prompts et réponses sont des **textes de longueur variable**. Pour
les comparer distributionnellement, on passe par un espace d'embedding
de dimension fixe (384 ou 768). Deux approches MVP :

- **Centroid + covariance** (fast, scalable) : on maintient le
  centroïde `μ` et la diagonale de la matrice de covariance `Σ` du
  baseline, et on calcule la distance de Mahalanobis-like du centroïde
  courant.
- **MMD² à noyau RBF** (rigorous, CPU-bound) : test statistique à
  deux échantillons dans le RKHS.

### 9.2 Centroid drift (fast path)

```python
def centroid_drift(
    baseline_centroid: np.ndarray,
    baseline_std: np.ndarray,
    current_samples: np.ndarray,
) -> CentroidDriftResult:
    current_centroid = current_samples.mean(axis=0)
    # Normalized distance per dimension, then L2
    normalized = (current_centroid - baseline_centroid) / (baseline_std + 1e-6)
    distance = float(np.linalg.norm(normalized))
    return CentroidDriftResult(
        distance=distance,
        centroid_shift=current_centroid - baseline_centroid,
    )
```

Seuils (empiriques, à tuner par tenant) :

| `distance` | Sévérité |
|---|---|
| < 0.5 | none |
| 0.5 – 1.0 | warning |
| 1.0 – 2.0 | major |
| > 2.0 | critical |

### 9.3 MMD² avec noyau RBF

```python
def mmd_squared_rbf(
    x: np.ndarray, y: np.ndarray, sigma: float | None = None,
) -> float:
    n, m = len(x), len(y)
    if sigma is None:
        # Median heuristic
        all_pts = np.vstack([x, y])
        pair_dists = pdist(all_pts)
        sigma = np.sqrt(np.median(pair_dists ** 2) / 2)
    gamma = 1.0 / (2 * sigma ** 2)
    Kxx = rbf_kernel(x, x, gamma)
    Kyy = rbf_kernel(y, y, gamma)
    Kxy = rbf_kernel(x, y, gamma)
    # Unbiased estimator (exclude diagonals)
    np.fill_diagonal(Kxx, 0)
    np.fill_diagonal(Kyy, 0)
    mmd_sq = (
        Kxx.sum() / (n * (n - 1)) +
        Kyy.sum() / (m * (m - 1)) -
        2 * Kxy.mean()
    )
    return float(mmd_sq)
```

### 9.4 Permutation test pour MMD²

```python
def mmd_permutation_p_value(
    x: np.ndarray, y: np.ndarray,
    n_permutations: int = 200, seed: int = 20260420,
) -> tuple[float, float]:
    rng = np.random.default_rng(seed)
    observed = mmd_squared_rbf(x, y)
    pooled = np.vstack([x, y])
    n = len(x)
    count_exceed = 0
    for _ in range(n_permutations):
        perm = rng.permutation(len(pooled))
        x_p = pooled[perm[:n]]
        y_p = pooled[perm[n:]]
        if mmd_squared_rbf(x_p, y_p) >= observed:
            count_exceed += 1
    p_value = (count_exceed + 1) / (n_permutations + 1)
    return observed, p_value
```

### 9.5 Reservoir sampling pour bases sous contrainte mémoire

Maintenir 10 k vecteurs baseline et 10 k vecteurs current dans un
reservoir (Vitter 1985) pour éviter OOM sur des workloads à 100 k
req/s :

```python
class EmbeddingReservoir:
    def __init__(self, size: int = 10_000, seed: int = 0):
        self._size = size
        self._rng = random.Random(seed)
        self._items: list[np.ndarray] = []
        self._count = 0

    def add(self, vec: np.ndarray) -> None:
        self._count += 1
        if len(self._items) < self._size:
            self._items.append(vec)
        else:
            j = self._rng.randrange(self._count)
            if j < self._size:
                self._items[j] = vec

    def sample(self) -> np.ndarray:
        return np.stack(self._items) if self._items else np.empty((0, 0))
```

### 9.6 Dimensionality reduction (pré-MMD)

Pour `dim = 768`, MMD² sur 10 k × 10 k = 100 M kernel evaluations ≈ 30 s.
Si latence budget tight, on peut réduire via :

- **Random projection** (Johnson-Lindenstrauss) vers dim=64 : MMD²
  reste valide à ε près pour `dim_proj ≥ 4 ln n / ε²`.
- **PCA** baseline-fit puis projection : préserve la variance maximum.

MVP : pas de réduction, on assume 384-dim, coût acceptable.

### 9.7 Classifier-based drift detection (CDD)

Alternative multivariée :

```python
def classifier_drift_score(
    baseline_samples: np.ndarray,
    current_samples: np.ndarray,
    seed: int = 20260420,
) -> ClassifierDriftResult:
    from sklearn.linear_model import LogisticRegression
    from sklearn.model_selection import train_test_split
    from sklearn.metrics import roc_auc_score
    X = np.vstack([baseline_samples, current_samples])
    y = np.array([0] * len(baseline_samples) + [1] * len(current_samples))
    X_train, X_test, y_train, y_test = train_test_split(
        X, y, test_size=0.2, random_state=seed, stratify=y,
    )
    clf = LogisticRegression(max_iter=1000, random_state=seed)
    clf.fit(X_train, y_train)
    auc = roc_auc_score(y_test, clf.predict_proba(X_test)[:, 1])
    # Drift score: distance from 0.5 (no discrimination)
    drift_score = 2 * abs(auc - 0.5)
    return ClassifierDriftResult(
        auc=auc,
        drift_score=drift_score,
        p_value=None,  # could use bootstrap
    )
```

Seuils :

| AUC | Sévérité |
|---|---|
| < 0.55 | none |
| 0.55 – 0.65 | warning |
| 0.65 – 0.75 | major |
| > 0.75 | critical |

---

## 10. Drift catégoriel et log-likelihood ratio

### 10.1 Cas d'usage

- `language` : fr, en, es, de, zh...
- `has_code` : 0 / 1.
- `model_variant` : llama-7b, mistral-7b, gpt-4o-mini...
- `country_hash` : hashed-bucketized country.

### 10.2 χ² goodness-of-fit détaillé

Exemple : baseline `{fr: 60, en: 30, other: 10}` vs current
`{fr: 40, en: 40, other: 20}` sur 100 observations chaque.

Expected = baseline proportions × n_current :
- `E_fr = 60`, `E_en = 30`, `E_other = 10`

Observed = `{fr: 40, en: 40, other: 20}`

```
χ² = (40 - 60)²/60 + (40 - 30)²/30 + (20 - 10)²/10
   = 400/60 + 100/30 + 100/10
   = 6.67 + 3.33 + 10 = 20.0
```

Avec `df = 2`, `p_value(χ² = 20) ≈ 4.5e-5` → critical.

### 10.3 Cardinalité élevée : regroupement d'autres

Pour des variables à haute cardinalité (e.g. `country` avec 200+
valeurs possibles, dont beaucoup rares), on regroupe en `top-K + other` :

```python
def bucket_categorical(
    counts: dict[str, int], top_k: int = 10,
) -> dict[str, int]:
    sorted_items = sorted(counts.items(), key=lambda t: -t[1])
    top = dict(sorted_items[:top_k])
    other = sum(c for _, c in sorted_items[top_k:])
    if other > 0:
        top["__other__"] = other
    return top
```

La stratégie `top_k` est figée à la promotion baseline (invariant I2)
pour garantir la stabilité cross-fenêtre.

### 10.4 Nouvelles catégories en production

Si une valeur apparaît en current qu'on n'avait pas en baseline, on la
route vers `__other__`. Un signal auxiliaire
`new_categories_count` est exposé pour détecter l'apparition de
valeurs novelle — utile pour repérer proactivement les nouvelles
langues ou marchés.

### 10.5 Log-likelihood ratio (G-test)

Alternatif à χ², plus précis sur petits échantillons :

```
G = 2 × ∑_k O_k × ln(O_k / E_k)
```

avec convention `0 × ln(0/x) = 0`.

Converge asymptotiquement vers χ²(K-1). Pour `n ≥ 5 × K`, χ² et G-test
donnent des p-values indistinguables à 1e-3 près.

---

## 11. Drift temporel multi-scale

### 11.1 Rolling windows

M09 maintient trois fenêtres glissantes par (tenant, task, feature) :

- **W_1h** : last 1 hour (60 rollups minute).
- **W_1d** : last 24 hours (1440 rollups minute).
- **W_7d** : last 7 days.

Chacune est comparée indépendamment à la baseline → trois
`drift_evaluation` rows par tick.

### 11.2 Why multi-scale

Un drift peut apparaître à différents horizons :

- **1 h** : incident ponctuel (déploiement d'un client, burst de
  traffic atypique).
- **1 j** : dérive diurne, shift marketing campagne.
- **7 j** : drift graduel (saisonnalité, contamination long-terme).

Un système qui alerte uniquement sur 1 h génère beaucoup de faux
positifs (bruit court-terme). Un système qui alerte uniquement sur
7 j rate les incidents aigus.

### 11.3 Exponential smoothing

Pour lisser le PSI horaire, M09 expose une version exponentielle :

```
PSI_smoothed[t] = α × PSI_raw[t] + (1 - α) × PSI_smoothed[t-1]
```

avec `α = 0.3` par défaut. Réduit le bruit d'un facteur ~3 sans
retarder les détections critiques (confirmé par simulation).

### 11.4 Indépendance multi-scale (invariant I6)

Les trois fenêtres sont calculées **indépendamment**. Pas de
propagation de seuil entre scales. Un PSI > 0.25 sur 1 h ne force pas
un alerte sur 1 j. Les trois signaux sont publiés séparément avec
`method: psi`, `window: 1h / 1d / 7d`.

---

## 12. Alerting et routage

### 12.1 Structure d'une AlertRule

```python
@dataclass(frozen=True, slots=True)
class AlertRule:
    rule_id: str
    tenant_id: str
    task_id: str | None             # None = all tasks
    feature_id: str | None          # None = all features
    method: str                     # 'psi','ks','mmd',...
    threshold_warning: float | None
    threshold_major: float | None
    threshold_critical: float | None
    window_kind: Literal["hour", "day", "week"]
    consecutive_breaches: int = 3
    suppression_s: int = 1800
    routing: list[ChannelConfig]
    enabled: bool = True
```

### 12.2 Hysteresis (anti-flapping)

Pour éviter qu'un signal oscillant autour d'un seuil ne tire des
alertes en cascade, on exige **N breaches consécutifs** avant de
passer à `FIRING` :

```python
def _should_fire(
    history: list[float], threshold: float, n: int = 3,
) -> bool:
    return len(history) >= n and all(v > threshold for v in history[-n:])
```

Même logique pour repasser à `RESOLVED` : N values sous le seuil.

### 12.3 Suppression

Après une alerte firing, le même `(rule_id, baseline_id, severity)`
est suppressé pendant `suppression_s` (default 1 800 s = 30 min). Le
compteur `breaches_so_far` continue de tourner mais aucune nouvelle
notification n'est dispatchée.

### 12.4 Channels et format

**SlackChannel** :

```json
{
  "channel_webhook": "https://hooks.slack.com/services/...",
  "mention": "@drift-on-call"
}
```

Message :

```
:rotating_light: *Drift Alert — CRITICAL*
Tenant: acme / Task: summarize / Feature: prompt_length_tokens
PSI: 0.42 (threshold: 0.40, window: 1h)
Baseline: baseline-abc123 (age: 45 days)
Dashboard: https://grafana.efe/dashboards/drift?tenant=acme&feature=...
```

**PagerDutyChannel** (Events API v2) :

```json
{
  "routing_key": "...",
  "event_action": "trigger",
  "dedup_key": "drift:acme:summarize:prompt_length_tokens:1h",
  "payload": {
    "summary": "EFE drift — critical",
    "severity": "critical",
    "source": "efe-drift",
    "custom_details": { ... }
  }
}
```

**WebhookChannel** : POST JSON générique, auth via HMAC-SHA256
header.

**NatsChannel** : publie sur `efe.drift.alert` — M08 guardrail peut
souscrire.

### 12.5 Dispatch résultat et audit

Chaque dispatch logge son résultat dans
`alert_incidents.dispatch_results` :

```json
{
  "slack": { "status": "ok", "delivered_at": "2026-05-01T10:00:00Z" },
  "pagerduty": { "status": "ok", "dedup_key": "..." },
  "webhook": { "status": "failed", "error": "timeout", "retries": 3 }
}
```

Les échecs transient sont retryés avec backoff exponentiel (1 s, 2 s,
4 s, 8 s, abandon).

### 12.6 Acquittement

Endpoint `POST /v1/drift/incidents/{id}/ack` avec :

```json
{
  "acked_by": "alice@example.com",
  "comment": "investigating"
}
```

Transitionne `firing → acked`. Empêche re-dispatch tant que l'état
reste `firing`. Si l'incident passe `resolved` puis re-fires, un
nouveau incident_id est créé (pas de résurrection).

---

## 13. Gestion des baselines

### 13.1 Création et promotion

Workflow standard :

1. **Sprint N, jour J** : M06 promeut un nouvel artefact (champion).
   NATS `efe.registry.routing.updated` publié.
2. **Sprint N, jour J+0** : M09 crée un `baseline` en état `PLANNED`.
   Liaison via `model_artifact_id`.
3. **Jours J+1 à J+7** : `PLANNED → ACCUMULATING`. Les histogrammes
   sont agrégés quotidiennement.
4. **Jour J+7** : validateur évalue coverage + sanity →
   `ACCUMULATING → CANDIDATE`.
5. **Jour J+7 à J+10** : opérateur humain review (UI). Optionnel —
   auto-promotion si `auto_promote: true` dans la policy.
6. **Promote** : `CANDIDATE → ACTIVE`. Bins figés, hash scellé, NATS
   publié.

### 13.2 Sanity checks à la promotion

```python
def validate_baseline_candidate(
    baseline: BaselineCandidate,
) -> list[ValidationIssue]:
    issues = []
    # Coverage
    if baseline.sample_count < 10_000:
        issues.append(Issue("insufficient_samples", severity="blocker"))
    # Uniformity
    if _any_bin_pct(baseline.histogram) > 0.95:
        issues.append(Issue("near_degenerate_distribution", severity="warning"))
    # Outlier detection
    if baseline.stats.get("std", 0) == 0:
        issues.append(Issue("zero_variance", severity="blocker"))
    # Time coverage
    days_covered = (baseline.window_end - baseline.window_start).days
    if days_covered < 7:
        issues.append(Issue("short_window", severity="warning"))
    return issues
```

Un `blocker` empêche la promotion ; un `warning` est loggé mais ne
bloque pas.

### 13.3 Rotation avec recouvrement

Quand un nouveau baseline est promu pour un `(tenant, task, feature)` :

1. Ancienne baseline `A` en `ACTIVE`, nouvelle `B` en `CANDIDATE`.
2. À la promotion de `B` :
   - `B.state = ACTIVE`, `B.promoted_at = now`.
   - `A.state = RETIRING`, `A.planned_end_at = now + overlap_days`.
3. Pendant `overlap_days` : les deux baselines sont actives pour la
   détection. Chaque évaluation produit deux
   `drift_evaluation` rows (une pour A, une pour B). Les dashboards
   affichent les deux pour comparaison.
4. `overlap_days` écoulés : job auto-scheduler fait
   `A.state = RETIRED`, `A.retired_at = now`. Les détections
   n'utilisent plus A.

Default `overlap_days = 3`. Min autorisé = 1, max = 14.

### 13.4 Emergency retire

Un opérateur peut forcer `ACTIVE → RETIRED` immédiatement via
`POST /v1/drift/baselines/{id}/retire?emergency=true` avec un reason
obligatoire. Dans ce cas :

- Pas d'overlap.
- Si aucune baseline successeur `ACTIVE`, la détection bascule en
  "insufficient_baseline" pour les features concernés.
- NATS `efe.drift.baseline_emergency_retired` publié (M06 peut écouter
  pour un rollback préventif).

### 13.5 Signature et intégrité

Chaque baseline scellée a :

- `spec_hash` : SHA-256 du payload canonique (bins + stats + centroid +
  metadata).
- `signature` : HMAC-SHA256 du `spec_hash` avec clé tenant-specific
  et domain-tag `b"efe-drift/baseline/v1\x00"`.
- `signing_key_id` : référence KMS.

Un verify-job quotidien recompute le hash et compare la signature —
any mismatch déclenche alerte P0.

---

## 14. Catalogue des features

### 14.1 Schéma d'enregistrement

Chaque feature surveillée est explicitement déclarée avant la première
ingestion :

```yaml
# features.yaml  (loaded at boot + via admin API)
features:
  - name: prompt_length_tokens
    tenant: acme
    task: summarize
    kind: numeric
    extractor: token_length
    binning: equifrequency
    retention_days: 90

  - name: language
    tenant: acme
    task: summarize
    kind: categorical
    extractor: lang_detect
    expected_cardinality: 20
    retention_days: 90

  - name: prompt_embedding
    tenant: acme
    task: summarize
    kind: embedding
    extractor: sbert_minilm
    retention_days: 30   # embeddings more expensive to store
```

### 14.2 Extractor registry

```python
class FeatureExtractor(Protocol):
    def extract(self, event: InferenceEvent) -> FeatureValue: ...


_EXTRACTOR_REGISTRY: dict[str, FeatureExtractor] = {}


def register_extractor(name: str, extractor: FeatureExtractor) -> None:
    if name in _EXTRACTOR_REGISTRY:
        raise ValueError(f"extractor {name!r} already registered")
    _EXTRACTOR_REGISTRY[name] = extractor


def get_extractor(name: str) -> FeatureExtractor:
    if name not in _EXTRACTOR_REGISTRY:
        raise KeyError(f"unknown extractor: {name!r}")
    return _EXTRACTOR_REGISTRY[name]
```

Extractors MVP :

- `token_length` : renvoie `len(tokens)` fourni par M07 dans l'event.
- `lang_detect` : fasttext 176-language model (lazy-loaded).
- `cost_pricing` : applique la pricing table par model_id.
- `sbert_minilm` : encodage via sentence-transformers
  `all-MiniLM-L6-v2` (384 dim).
- `has_code_heuristic` : regex ```` ``` ```` ou bloc indenté.

### 14.3 PII linter

Invariant I6 impose zéro PII dans les snapshots. Un job quotidien
`drift-pii-audit` :

- Échantillonne 1000 snapshots.
- Pour chaque, scanne `histogram_json` et `stats_json` pour détecter :
  - Emails (regex).
  - Numéros téléphone (regex).
  - IBANs (regex).
  - Identifiants numériques longs (≥ 10 chars).
- Si détection → alerte P0 + identifie l'extractor fautif.

### 14.4 Évolution du catalogue

Ajouter un nouveau feature :

1. `POST /v1/drift/features` avec la spec.
2. Nouvelle row `feature_catalog` version 1.
3. À partir de ce point, ingestion commence.
4. Après N jours → baseline candidate auto-créé.

Modifier un feature existant :

- **NE PAS mettre à jour** la row existante (baselines liées).
- Créer une nouvelle version : `(name=X, version=2)`.
- Les baselines liées à v1 restent valides.
- Les nouvelles ingestions vont sur v2 quand cutover ops.

Suppression : `DELETE /v1/drift/features/{id}` passe `enabled=false`,
les baselines ACTIVE restent queryables mais pas de nouvelles ingestions.

---

## 15. NATS topics et flows

### 15.1 Topics produits par M09

| Topic | Payload | Consommateur |
|---|---|---|
| `efe.drift.baseline_promoted` | `{baseline_id, feature, spec_hash}` | M12 dashboards, M06 |
| `efe.drift.baseline_retired` | `{baseline_id, reason}` | M12 |
| `efe.drift.baseline_emergency_retired` | idem + severity | M06 (rollback trigger) |
| `efe.drift.detected` | `{feature, method, score, severity, window}` | M08, M10 |
| `efe.drift.alert.firing` | `{incident_id, rule_id, score, severity}` | M11 gateway |
| `efe.drift.alert.resolved` | idem | M11 |
| `efe.drift.feature_registered` | `{feature_id, name, kind}` | M12 |
| `efe.drift.pii_detected` | `{feature_id, extractor, sample_evidence}` | Security team |

### 15.2 Topics consommés par M09

| Topic | Producteur | Action |
|---|---|---|
| `efe.inference.completed` | M07 | extrait features → raw_events |
| `efe.rollout.outcome` | M08 | extrait métriques → raw_events |
| `efe.registry.routing.updated` | M06 | crée baseline `PLANNED` pour le nouvel artefact |
| `efe.registry.rollback.executed` | M06 | marque une baseline `emergency_retired` si drift était la cause |

### 15.3 Format envelope

Identique M06/M07/M08 : JSON avec `schema_version`, `event_type`,
`event_id`, `occurred_at`, `tenant_id`, `payload`, `hmac`.

**Domain tag** : `b"efe-drift/nats/v1\x00"` préfixé avant HMAC.

### 15.4 Backpressure

Le consommateur NATS M09 utilise JetStream avec :

- `ack_policy: explicit`
- `max_ack_pending: 10 000`
- `max_deliver: 3` (retries avant dead letter)
- `filter_subject: efe.inference.completed, efe.rollout.outcome`

Si la file interne dépasse 50 000 events, le pod M09 émet
`efe_drift_backpressure_s` metric et NATS throttle l'upstream.

---

## 16. Interfaces publiques

### 16.1 Python SDK (in-process)

```python
from efe_drift import DriftClient, FeatureSpec

client = DriftClient.from_env()

# Option 1 : report individual samples (sync)
client.report_sample(
    tenant_id="acme",
    task_id="summarize",
    feature_name="prompt_length_tokens",
    value=427,
    observed_at=datetime.now(timezone.utc),
)

# Option 2 : report in bulk
client.report_samples([
    SampleRecord(
        tenant_id="acme", task_id="summarize",
        feature_name="prompt_length_tokens", value=427,
    ),
    SampleRecord(
        tenant_id="acme", task_id="summarize",
        feature_name="language", value="fr",
    ),
])

# Query drift
evaluations = client.get_drift_evaluations(
    tenant_id="acme", task_id="summarize",
    feature_name="prompt_length_tokens",
    since=datetime.now(timezone.utc) - timedelta(days=1),
)
for ev in evaluations:
    print(f"{ev.method}: score={ev.score}, severity={ev.severity}")

# Baseline ops
baselines = client.list_baselines(
    tenant_id="acme", task_id="summarize", state="active",
)
client.promote_baseline(baseline_id=baselines[0].id)
```

### 16.2 gRPC

```proto
service DriftService {
    // Ingestion
    rpc ReportSample(SampleRecord) returns (Ack);
    rpc ReportSamples(stream SampleRecord) returns (BatchAck);

    // Query
    rpc GetDriftEvaluations(DriftQuery) returns (stream Evaluation);
    rpc GetLatestScore(FeatureRef) returns (DriftScore);

    // Baseline management
    rpc ListBaselines(BaselineQuery) returns (stream Baseline);
    rpc PromoteBaseline(BaselineRef) returns (Baseline);
    rpc RetireBaseline(RetireRequest) returns (Baseline);

    // Alert management
    rpc ListIncidents(IncidentQuery) returns (stream Incident);
    rpc AckIncident(AckRequest) returns (Incident);

    // Feature catalog
    rpc RegisterFeature(FeatureSpec) returns (Feature);
    rpc ListFeatures(FeatureQuery) returns (stream Feature);
}
```

### 16.3 HTTP REST

| Méthode | Chemin | Usage |
|---|---|---|
| `POST` | `/v1/drift/samples` | ingérer un échantillon |
| `POST` | `/v1/drift/samples/bulk` | ingérer en batch |
| `GET` | `/v1/drift/evaluations` | liste les signals filtré |
| `GET` | `/v1/drift/features` | liste catalog |
| `POST` | `/v1/drift/features` | register new feature |
| `GET` | `/v1/drift/baselines` | liste baselines |
| `POST` | `/v1/drift/baselines/{id}/promote` | `candidate → active` |
| `POST` | `/v1/drift/baselines/{id}/retire` | `active → retired` |
| `GET` | `/v1/drift/incidents` | liste alertes |
| `POST` | `/v1/drift/incidents/{id}/ack` | acquitter |
| `GET` | `/healthz` | liveness |
| `GET` | `/metrics` | Prometheus endpoint |

### 16.4 CLI ops (`efe-drift`)

```bash
efe-drift features list --tenant acme --task summarize
efe-drift features register --file features.yaml
efe-drift baselines list --state active
efe-drift baselines promote --id <uuid>
efe-drift baselines rotate --feature prompt_length_tokens --tenant acme
efe-drift evaluations show --feature prompt_length_tokens --window 1h
efe-drift incidents list --state firing
efe-drift incidents ack <uuid> --by alice
efe-drift init-db
efe-drift health
```

---

## 17. Stratégie de tests

### 17.1 Unit tests (≥ 250)

```
tests_drift/
  test_psi_computation.py            I1 + valeurs canoniques (worked example §7.5)
  test_psi_smoothing.py              edge cases (bins vides, eps)
  test_binning_equifrequency.py
  test_binning_equiwidth.py
  test_binning_custom.py
  test_binning_stability.py          I2 — bins figés
  test_ks_vs_scipy.py                parity avec scipy.stats.ks_2samp
  test_wasserstein_vs_scipy.py
  test_chi_squared_vs_scipy.py
  test_js_divergence.py
  test_mmd_rbf.py
  test_mmd_permutation_p_value.py
  test_classifier_drift.py
  test_centroid_drift.py
  test_embedding_reservoir.py        Vitter parity
  test_random_projection_mmd.py
  test_baseline_state_machine.py     FSM 8 transitions
  test_baseline_state_cas.py         optimistic lock
  test_baseline_immutable_trigger.py I4 — Postgres trigger
  test_baseline_rotation_overlap.py  I8 — recouvrement
  test_baseline_emergency_retire.py
  test_baseline_signature_seal.py    SHA-256 + HMAC
  test_feature_catalog_versioning.py
  test_feature_extractor_registry.py
  test_pii_linter.py
  test_aggregator_minute_rollup.py
  test_aggregator_ordering.py        I7 — observed_at based
  test_multi_scale_windows.py        I6 — indépendance
  test_exponential_smoothing.py
  test_alert_hysteresis.py
  test_alert_suppression.py
  test_alert_router_slack.py
  test_alert_router_pagerduty.py
  test_alert_router_webhook_hmac.py
  test_alert_incident_fsm.py
  test_nats_envelope_domain_tag.py   domain separation
  test_nats_consumer_backpressure.py
  test_http_api_crud.py
  test_sdk_client_happy_path.py
  test_cli_init_db.py
  test_concurrent_baseline_promotion.py
  test_timezone_boundaries.py        buckets UTC strict
  test_high_cardinality_rollup.py    10_000 features
  test_retention_purge.py
  test_detection_cache_invalidation.py
  test_degenerate_baseline_detection.py  zero-variance refusé
  test_new_category_tracking.py
  test_cross_tenant_isolation.py     I5
```

### 17.2 Integration (≥ 40)

```
tests_drift_integration/
  test_e2e_ingest_detect_alert.py
  test_e2e_baseline_rotation_with_overlap.py
  test_e2e_emergency_retire_triggers_m06_rollback.py
  test_e2e_multi_scale_detection.py
  test_e2e_classifier_drift_pipeline.py
  test_e2e_mmd_with_real_embeddings.py
  test_e2e_pagerduty_sandbox.py
  test_e2e_slack_sandbox.py
  test_e2e_m07_consumption.py
  test_e2e_m08_guardrail_integration.py
  test_e2e_pii_linter_blocks_bad_feature.py
```

### 17.3 Chaos

```
tests_drift_chaos/
  test_chaos_db_primary_kill_during_promote.py
  test_chaos_nats_down_during_ingest.py
  test_chaos_clock_skew_affects_buckets.py
  test_chaos_burst_1M_events_in_60s.py
  test_chaos_partial_baseline_accumulation.py
```

### 17.4 Property-based tests (Hypothesis)

```
tests_drift_property/
  test_psi_symmetry_property.py       PSI(P,Q) == PSI(Q,P)
  test_psi_non_negative_property.py   PSI ≥ 0 always
  test_js_divergence_bounded.py       JS ∈ [0, ln 2]
  test_bucketing_preserves_count.py   sum(histogram) == count
  test_chi_squared_non_negative.py
  test_wasserstein_triangle_inequality.py
  test_mmd_zero_iff_identical_distributions.py
```

### 17.5 Couverture cible

- `detection/` : 98 % (critique)
- `baseline/` : 95 %
- `aggregation/` : 95 %
- `alerting/` : 90 %
- Global : ≥ 90 %

---

## 18. Observabilité

### 18.1 Metrics Prometheus

```
# Ingestion
efe_drift_ingest_total{tenant, source}           (counter)
efe_drift_ingest_latency_seconds                 (histogram)
efe_drift_ingest_rejected_total{reason}          (counter)
efe_drift_backpressure_s                          (gauge)

# Extraction
efe_drift_extractor_invocations_total{extractor} (counter)
efe_drift_extractor_latency_seconds{extractor}   (histogram)
efe_drift_extractor_failures_total{extractor, kind}

# Aggregation
efe_drift_rollup_latency_seconds{bucket_kind}    (histogram)
efe_drift_rollup_row_count                       (gauge)

# Baseline lifecycle
efe_drift_baselines_total{tenant, state}         (gauge)
efe_drift_baselines_promoted_total{tenant}
efe_drift_baselines_rotated_total{tenant}
efe_drift_baselines_emergency_retired_total{tenant, reason}

# Detection
efe_drift_detections_total{method, severity}     (counter)
efe_drift_detection_latency_seconds{method}      (histogram)
efe_drift_score{tenant, task, feature, method, window}  (gauge — last observed)

# Alerts
efe_drift_alerts_firing_total{severity}
efe_drift_alert_dispatch_latency_seconds{channel} (histogram)
efe_drift_alert_dispatch_failures_total{channel, error}

# PII
efe_drift_pii_detected_total{feature, pattern}   (counter)

# Cache hit rate
efe_drift_cache_hits_total{layer}
efe_drift_cache_misses_total{layer}
```

### 18.2 Alerting rules

```yaml
- alert: DriftCriticalScore
  expr: max by (tenant, feature) (efe_drift_score{severity="critical"}) > 0.40
  for: 10m
  labels: {severity: critical}

- alert: DriftIngestLag
  expr: efe_drift_backpressure_s > 30
  for: 5m
  labels: {severity: warning}

- alert: DriftAlertDispatchFailure
  expr: rate(efe_drift_alert_dispatch_failures_total[5m]) > 0.01
  for: 5m
  labels: {severity: major}

- alert: DriftBaselineCoverageGap
  expr: |
    count by (tenant, task) (efe_drift_baselines_total{state="active"}) == 0
  for: 15m
  labels: {severity: major}

- alert: DriftPiiDetected
  expr: increase(efe_drift_pii_detected_total[1h]) > 0
  for: 0s
  labels: {severity: p0}
  annotations: { runbook: "runbook-drift-pii-leak.md" }

- alert: DriftDetectionLatencyBudget
  expr: histogram_quantile(0.95, rate(efe_drift_detection_latency_seconds_bucket[5m])) > 10
  for: 10m
  labels: {severity: warning}
```

### 18.3 Dashboards

**Panel list** :

1. Timeline multi-scale (PSI 1h/1d/7d) par feature.
2. Heatmap features × time — sévérité colorée.
3. Baseline health (état, âge, rotation imminente).
4. Alert incidents timeline.
5. Ingest throughput + backpressure.
6. Detection latency par méthode.
7. Embedding drift — MMD² + centroid distance.
8. Classifier drift AUC.
9. Top features by drift score (last 24 h).
10. Tenant isolation check (cross-tenant hits).

### 18.4 Traces OpenTelemetry

Root span : `drift.evaluate`. Enfants :

- `db.load_snapshot` (fetch current + baseline histograms)
- `compute.psi` (or `compute.mmd`, `compute.ks`)
- `db.write_evaluation`
- `alert.evaluate`
- `alert.dispatch` (un child par channel)

---

## 19. Plan de sprint M09

### 19.1 Découpage en 5 lots (10 jours ouvrés, 2 devs)

| Lot | Durée | Contenu | DRI |
|---|---|---|---|
| **L1 — Catalog + ingestion** | 2 j | feature_catalog, NatsConsumer, HttpSink, extractors de base, PII linter | Dev A |
| **L2 — Aggregation + TimescaleDB** | 2 j | raw_events hypertable, continuous aggregates, HistogramBuilder, MinuteRollup | Dev B |
| **L3 — Baselines + FSM** | 2 j | BaselineCandidate, BaselineManager, rotation + overlap, signature seal | Dev A |
| **L4 — Détecteurs** | 2 j | PSI, KS, χ², Wasserstein, MMD, classifier-drift, multi-scale windows | Dev B |
| **L5 — Alerting + API + obs** | 2 j | AlertRouter (Slack/PD/webhook/NATS), hysteresis, suppression, REST/SDK/CLI, dashboards | Dev A+B |

### 19.2 Dépendances amont

- M06 stable (envoie `routing.updated`).
- M07 stable (envoie `efe.inference.completed` avec tokens+embedding si applicable).
- M08 stable (envoie `efe.rollout.outcome`).

### 19.3 Definition of Done

- [ ] Tests unitaires verts, couverture ≥ 90 %.
- [ ] E2E : ingest → detect → alert en < 5 s.
- [ ] Baseline rotation avec overlap testée sur 2 features.
- [ ] Latence ingest p99 < 10 ms mesurée.
- [ ] 10 alertes Alertmanager provisionnées.
- [ ] Dashboard Grafana importé.
- [ ] Runbooks : `runbook-drift-critical.md`, `runbook-drift-pii-leak.md`,
  `runbook-drift-baseline-rotation.md`, `runbook-drift-alert-flapping.md`.
- [ ] PII linter job configuré et testé contre faux-positifs connus.

---

## 20. Critères d'acceptation

### 20.1 Fonctionnel

1. Un événement inference arrive, ses features sont extraites, et un
   signal de drift apparaît au prochain rollup (< 2 min).
2. Un baseline candidat avec 7 j de données et 100 k samples est
   promu avec succès.
3. Une rotation de baseline avec `overlap_days=3` préserve
   l'interrogeabilité des deux pendant 3 j.
4. Un PSI > 0.40 sur 3 fenêtres consécutives déclenche une alerte
   `critical` avec dispatch Slack + PagerDuty + NATS.
5. Une tentative d'UPDATE sur un baseline `ACTIVE` est rejetée par le
   trigger Postgres.
6. Un feature déclaré avec un extractor inconnu est rejeté au register.
7. Un embedding drift avec MMD² significatif produit une
   `drift_evaluation` avec `method='mmd'` et p-value bootstrap.
8. Un incident acquitté ne re-dispatch pas pendant qu'il reste
   `firing`.

### 20.2 Non-fonctionnel

1. Ingestion p99 < 10 ms sous 10 k req/s.
2. Detection p95 < 10 s sur 10⁴ features.
3. Disponibilité 99.95 %/mois.
4. Rétention 90 j automatique (no manual cleanup).

### 20.3 Sécurité & compliance

1. Aucune PII brute dans les histogrammes (I6, vérifié linter).
2. Baselines signées HMAC vérifiables indépendamment.
3. Audit trail complet des promotions/rotations dans
   `baseline_state_transitions`.
4. Cross-tenant isolation testée (I5).

### 20.4 Qualité d'ingénierie

1. `mypy --strict` vert sur tout `efe_drift`.
2. `ruff check` zéro erreur.
3. OpenAPI 3.1 complet généré.
4. SDK Python typé, couvert par tests d'intégration.

---

## 21. Risques et mitigations

| Risque | Impact | Probabilité | Mitigation |
|---|---|---|---|
| **PSI bruit sur petits échantillons** | faux alertes | Haute | min_sample_size par feature ; skip si pas assez de data |
| **Binning drift subtil** (quantiles hardcoded) | faux négatifs | Moyenne | quantile bins figés à promotion + rotation régulière |
| **Clock skew bucketing** | double-compte ou miss | Faible | NTP strict + check daily ; I7 sur observed_at |
| **MMD² coût CPU 30 s** | jobs en retard | Moyenne | random projection + budget de 1 min/feature max |
| **Alert storm** (flapping autour seuil) | pager fatigue | Haute | hysteresis 3 breaches + suppression 30 min |
| **PII leak via histogramme high-cardinality** | compliance breach | Faible | PII linter quotidien + top_K bucketing obligatoire |
| **Baseline too narrow** (7j pas assez) | détection biaisée | Moyenne | sanity checks + warning sur `days_covered < 14` |
| **Rotation gap** | aucune baseline active | Faible | contrôle "≥ 1 ACTIVE par feature" post-rotation |
| **Retention lost useful data** | audit post-hoc impossible | Faible | aggrégés 365 j préservés même si raw purgés |
| **Sentence embedder drift propre** | drift fantôme | Moyenne | version pin + check hash du modèle au boot |
| **Consumer NATS lag massif** | detection retardée | Moyenne | backpressure exposé + alert on `backpressure_s > 30` |
| **Schema evolution of inference events** | extractors cassés | Haute | schema version check + fallback null feature value |

Runbooks (liste minimale) :

- `runbook-drift-critical.md` — response à un PSI critical.
- `runbook-drift-pii-leak.md` — PII détectée, rotation immédiate du feature.
- `runbook-drift-baseline-rotation.md` — rotation planifiée.
- `runbook-drift-alert-flapping.md` — ajuster hysteresis/suppression.
- `runbook-drift-timescale-down.md` — perte TimescaleDB primary.
- `runbook-drift-retention-adjustment.md` — changement policy retention.

---

## 22. Annexes

### A. Helm values (extrait)

```yaml
drift:
  replicaCount: 3
  image:
    repository: efe-registry.io/core/drift
    tag: "1.0.0"
  resources:
    requests: { cpu: 1,    memory: 2Gi }
    limits:   { cpu: 4,    memory: 8Gi }

  ingestion:
    nats_subject_filter:
      - "efe.inference.completed"
      - "efe.rollout.outcome"
    buffer_size: 10000
    flush_interval_s: 1

  extractors:
    sbert_model_name: "all-MiniLM-L6-v2"
    sbert_device: "cpu"         # GPU pool optional for 100k req/s
    lang_detect_min_chars: 8

  baselines:
    min_accumulation_days: 7
    default_overlap_days: 3
    min_sample_count: 10000
    auto_promote: false

  detection:
    window_1h_enabled: true
    window_1d_enabled: true
    window_7d_enabled: true
    psi_min_samples: 1000
    mmd_reservoir_size: 10000
    mmd_n_permutations: 200
    classifier_train_size: 8000
    classifier_test_size: 2000

  alerts:
    default_thresholds:
      psi:  { warning: 0.10, major: 0.25, critical: 0.40 }
      mmd:  { warning: 0.05, major: 0.10, critical: 0.20 }
      ks:   { warning: 0.10, major: 0.20, critical: 0.30 }
    hysteresis_consecutive: 3
    suppression_s: 1800

  retention:
    raw_events_days: 7
    snapshots_days: 90
    evaluations_days: 365
    incidents_days: 365

  pii_linter:
    enabled: true
    sample_size: 1000
    schedule_cron: "0 2 * * *"   # nightly at 02:00 UTC
```

### B. Worked example PSI sur latence

Baseline (30 jours, 3 M observations, 10 bins équifréquence) :

```
bins      = [0, 80, 120, 150, 180, 210, 250, 300, 400, 600, +inf]
counts    = [300000] * 10    (equifreq => uniform by construction)
```

Current (1 h, 40 k observations) :

```
counts_current = [1200, 2400, 3600, 4800, 5200, 6400, 6800, 5600, 3200, 800]
sum = 40000
```

Proportions :

```
q = 0.10 each
p = [0.030, 0.060, 0.090, 0.120, 0.130, 0.160, 0.170, 0.140, 0.080, 0.020]
```

PSI :

```
bin 0: (0.03-0.10) * ln(0.03/0.10) = -0.07 * -1.204 = 0.084
bin 1: (0.06-0.10) * ln(0.06/0.10) = -0.04 * -0.511 = 0.020
bin 2: (0.09-0.10) * ln(0.09/0.10) = -0.01 * -0.105 = 0.001
bin 3: (0.12-0.10) * ln(0.12/0.10) =  0.02 * 0.182 = 0.004
bin 4: (0.13-0.10) * ln(0.13/0.10) =  0.03 * 0.262 = 0.008
bin 5: (0.16-0.10) * ln(0.16/0.10) =  0.06 * 0.470 = 0.028
bin 6: (0.17-0.10) * ln(0.17/0.10) =  0.07 * 0.531 = 0.037
bin 7: (0.14-0.10) * ln(0.14/0.10) =  0.04 * 0.336 = 0.013
bin 8: (0.08-0.10) * ln(0.08/0.10) = -0.02 * -0.223 = 0.004
bin 9: (0.02-0.10) * ln(0.02/0.10) = -0.08 * -1.609 = 0.129

PSI = 0.328  →  major shift (latency distribution drifted — more fast
responses + a shift away from the slowest tail ; worth investigating).
```

### C. Feature YAML exemple complet

```yaml
# /etc/efe-drift/features.yaml
version: 1
features:
  - name: prompt_length_tokens
    tenant: acme
    task: summarize
    kind: numeric
    extractor: token_length
    binning:
      strategy: equifrequency
      n_bins: 10
    retention_days: 90

  - name: response_length_tokens
    tenant: acme
    task: summarize
    kind: numeric
    extractor: token_length
    binning:
      strategy: equifrequency
      n_bins: 10
    retention_days: 90

  - name: latency_ms
    tenant: acme
    task: summarize
    kind: numeric
    extractor: backend_latency
    binning:
      strategy: custom
      edges: [0, 50, 100, 150, 200, 300, 500, 1000, 2000, 5000, 30000]
    retention_days: 90

  - name: language
    tenant: acme
    task: summarize
    kind: categorical
    extractor: lang_detect
    expected_cardinality: 20
    top_k_bucket: 10
    retention_days: 90

  - name: has_code
    tenant: acme
    task: summarize
    kind: binary
    extractor: has_code_heuristic
    retention_days: 180

  - name: prompt_embedding
    tenant: acme
    task: summarize
    kind: embedding
    extractor: sbert_minilm
    dim: 384
    retention_days: 30      # embeddings are expensive
```

### D. SQL daily rollup script

```sql
-- Run daily at 03:00 UTC by the scheduler
BEGIN;

-- 1. Rollup hourly from 1-min continuous aggregate
INSERT INTO feature_snapshots (
    tenant_id, task_id, feature_id, bucket_kind,
    bucket_start, bucket_end, sample_count, histogram_json, stats_json
)
SELECT
    tenant_id, task_id, feature_id, 'hour',
    time_bucket('1 hour', bucket_start) AS h_start,
    time_bucket('1 hour', bucket_start) + interval '1 hour' AS h_end,
    sum(sample_count),
    aggregate_histogram(histogram_json),   -- custom function
    jsonb_build_object(
      'mean', avg(mean),
      'p50',  percentile_cont(0.5) WITHIN GROUP (ORDER BY mean),
      'p95',  percentile_cont(0.95) WITHIN GROUP (ORDER BY mean)
    )
FROM feature_snapshots
WHERE bucket_kind = 'minute'
  AND bucket_start >= now() - interval '25 hours'
  AND bucket_start <  now() - interval '1 hour'
GROUP BY tenant_id, task_id, feature_id, h_start
ON CONFLICT DO NOTHING;

-- 2. Similar rollup for 'day'
INSERT INTO feature_snapshots (
    tenant_id, task_id, feature_id, bucket_kind,
    bucket_start, bucket_end, sample_count, histogram_json, stats_json
)
SELECT
    tenant_id, task_id, feature_id, 'day',
    date_trunc('day', bucket_start),
    date_trunc('day', bucket_start) + interval '1 day',
    sum(sample_count),
    aggregate_histogram(histogram_json),
    jsonb_build_object(
      'mean', avg((stats_json->>'mean')::float),
      'p50',  percentile_cont(0.5) WITHIN GROUP (ORDER BY (stats_json->>'mean')::float)
    )
FROM feature_snapshots
WHERE bucket_kind = 'hour'
  AND bucket_start >= now() - interval '2 days'
  AND bucket_start <  date_trunc('day', now())
GROUP BY tenant_id, task_id, feature_id, 1
ON CONFLICT DO NOTHING;

COMMIT;
```

### E. Alert routing config exemple

```yaml
# /etc/efe-drift/alert_rules.yaml
rules:
  - name: prod-critical-drift
    tenant: acme
    task: null                # all tasks
    feature: null             # all features
    method: psi
    thresholds:
      warning: 0.10
      major:   0.25
      critical: 0.40
    window_kind: hour
    consecutive_breaches: 3
    suppression_s: 1800
    routing:
      - channel: slack
        webhook_url_env: SLACK_WEBHOOK_DRIFT
        mention: "<!subteam^drift-on-call>"
        min_severity: major
      - channel: pagerduty
        routing_key_env: PAGERDUTY_ROUTING_KEY_DRIFT
        min_severity: critical
      - channel: nats
        subject: efe.drift.alert
        min_severity: warning
      - channel: webhook
        url: https://siem.internal/drift-webhook
        hmac_key_env: SIEM_DRIFT_HMAC_KEY
        min_severity: warning

  - name: pii-detected
    tenant: "*"
    method: pii_lint
    thresholds:
      critical: 1             # any occurrence
    routing:
      - channel: pagerduty
        routing_key_env: PAGERDUTY_SECURITY
        min_severity: critical
      - channel: slack
        webhook_url_env: SLACK_WEBHOOK_SECURITY
        min_severity: critical
```

### F. Grafana dashboard JSON (squelette)

```json
{
  "title": "EFE Drift — Overview",
  "tags": ["efe", "drift"],
  "panels": [
    {
      "title": "PSI by feature (1h rolling)",
      "type": "timeseries",
      "targets": [
        { "expr": "efe_drift_score{window=\"1h\",method=\"psi\"}" }
      ]
    },
    {
      "title": "Alerts firing",
      "type": "stat",
      "targets": [
        { "expr": "sum(efe_drift_alerts_firing_total{})" }
      ]
    },
    {
      "title": "Ingest throughput",
      "type": "timeseries",
      "targets": [
        { "expr": "rate(efe_drift_ingest_total[1m])" }
      ]
    },
    {
      "title": "Detection latency (p95)",
      "type": "timeseries",
      "targets": [
        {
          "expr": "histogram_quantile(0.95, rate(efe_drift_detection_latency_seconds_bucket[5m]))"
        }
      ]
    },
    {
      "title": "Baseline coverage",
      "type": "table",
      "targets": [
        { "expr": "efe_drift_baselines_total{state=\"active\"}" }
      ]
    }
  ]
}
```

### G. Benchmarks visés

| Op | Input | Budget | Mesure cible |
|---|---|---|---|
| PSI computation | 10 bins, 100 k obs | — | < 1 ms |
| KS test | 10 k + 10 k continuous | — | < 50 ms |
| MMD² RBF | 10 k + 10 k × 384-dim | — | < 30 s |
| MMD² RBF + RP dim=64 | 10 k + 10 k | — | < 5 s |
| Classifier-based drift | 10 k + 10 k × 384-dim | — | < 10 s |
| Histogram build | 1 M obs | — | < 500 ms |
| Baseline promote | 1 M obs, 10 features | — | < 30 s |
| Alert dispatch (Slack) | 1 message | p99 | < 2 s |
| Alert dispatch (PagerDuty) | 1 event | p99 | < 3 s |
| Ingest (NATS event → raw_events) | 1 event | p99 | < 10 ms |
| Rollup minute | 1 min × 10⁴ features | p95 | < 1 s |
| Detection 1h tick | 10⁴ features | p95 | < 10 s |

### H. Glossary M09

| Terme | Définition |
|---|---|
| **PSI** (Population Stability Index) | Σ (p-q) ln(p/q) — mesure symétrique de shift distributionnel |
| **KL** (Kullback-Leibler) | Σ p ln(p/q) — divergence asymétrique |
| **JS** (Jensen-Shannon) | KL symétrisée et bornée |
| **KS** (Kolmogorov-Smirnov) | sup\|F_P - F_Q\| — test distribution-free 1D |
| **MMD** (Maximum Mean Discrepancy) | distance entre P et Q dans un RKHS — multivariée |
| **RBF kernel** | k(x,y) = exp(-‖x-y‖²/2σ²) — noyau gaussien |
| **CDD** (Classifier-based Drift Detection) | AUC d'un classifier baseline-vs-current |
| **Baseline** | snapshot scellé d'une distribution de référence |
| **Bin** | intervalle discret d'un histogramme |
| **Rollup** | agrégation d'un bucket temporel en un agrégat plus large |
| **Continuous aggregate** | materialized view TimescaleDB mise à jour incrémentalement |
| **Hysteresis** | règle "N breaches consécutifs" avant firing |
| **Suppression** | verrou anti re-dispatch pendant X minutes |
| **Overlap days** | jours pendant lesquels deux baselines coexistent lors rotation |
| **Random projection** | technique Johnson-Lindenstrauss réduction dim vectorielle |
| **Reservoir sampling** | échantillonnage Vitter sous contrainte mémoire |

### I. Invariants mathématiques vérifiés par tests property-based

| Invariant | Test Hypothesis |
|---|---|
| PSI ≥ 0 | `∀ P, Q: psi(P, Q) ≥ 0` |
| PSI = 0 ⇔ P = Q | `∀ P: psi(P, P) ≈ 0` and `P ≠ Q ⟹ psi(P, Q) > 0` |
| PSI symétrique | `∀ P, Q: psi(P, Q) == psi(Q, P)` |
| JS bornée | `∀ P, Q: 0 ≤ JS(P, Q) ≤ ln 2` |
| KS bornée | `∀ P, Q: 0 ≤ KS(P, Q) ≤ 1` |
| χ² ≥ 0 | `∀ O, E: chi2(O, E) ≥ 0` |
| MMD ≥ 0 | `∀ P, Q: MMD²(P, Q) ≥ 0` (après correction unbiased peut être négatif en pratique mais asymptotiquement ≥ 0) |
| Wasserstein triangle | `W(P, R) ≤ W(P, Q) + W(Q, R)` |
| Histogram conservation | `sum(histogram(X, bins)) == len(X)` |
| Binning stability | `bins(baseline) == bins(baseline)` bit-à-bit cross-call |

### J. Threat model & abuse surface

| Menace | Vecteur | Mitigation |
|---|---|---|
| Baseline tampering | attaque sur DB ou clé signing | HMAC signé + verify quotidien + KMS |
| Fausse alerte injection via NATS | attaquant sign event | domain tag `efe-drift/nats/v1` + skew window 300 s |
| Exfiltration PII via features | regex/hash contourné | PII linter quotidien + allowlist extractors |
| Cross-tenant data leak | query sans `WHERE tenant_id` | row-level security Postgres + audit |
| DoS par ingest burst | flood `/v1/drift/samples` | rate limit tenant-level M11 + backpressure NATS |
| Rotation skip malicieux | désactiver rotation via admin | audit `baseline_state_transitions` + alerte sur absence rotation > 60j |
| Sink failure cascade | Slack down → PagerDuty aussi | indépendance des canaux + NATS fallback obligatoire |

---

**Fin du document M09.**
*Version 1.0 — spec gelée pour implémentation — dernière revue 2026-04-20.*
*Toute évolution de contrat (Feature spec, Evaluation schema, Alert rule
semantics) passe par une ADR référençant ce document.*
