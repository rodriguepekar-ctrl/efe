# Sprint D6 — Architecture : Fix `0-round consensus` + Accuracy SLO infrastructure

> **Statut** : architecture détaillée, prête à implémenter.
> **Phase** : ultra-thinking. Précision chirurgicale suprême.
> **Position dans la séquence** : suit D5 (orchestration unifiée du bench).
> D6 est **le sprint qui débloque la valeur business** : sans lui,
> Mode 1 produit 50 % d'accuracy sur la classification — ce qu'aucun
> SLO bench actuel ne détecte.
> **Pré-requis** : avoir lu
> [`d1_real_strategies_architecture.md`](d1_real_strategies_architecture.md),
> et avoir consulté le rapport du test pratique réel
> (cf. fil conversationnel, run du `2026-04-25T18:58Z` avec
> 9 cas multilingues de `support_triage`).

---

## Table des matières

1. [Contexte & mission](#1-contexte--mission)
2. [Diagnostic du bug `0-round consensus`](#2-diagnostic-du-bug-0-round-consensus)
3. [Trois hypothèses de cause-racine, classées par évidence](#3-trois-hypothèses-de-cause-racine-classées-par-évidence)
4. [Architecture du fix : approche en 3 couches défensives](#4-architecture-du-fix--approche-en-3-couches-défensives)
5. [L'infrastructure eval déjà présente dans le repo](#5-linfrastructure-eval-déjà-présente-dans-le-repo)
6. [Câblage de l'eval dans la pipeline bench (D5)](#6-câblage-de-leval-dans-la-pipeline-bench-d5)
7. [12 décisions de conception clés](#7-12-décisions-de-conception-clés)
8. [10 risques identifiés et mitigations](#8-10-risques-identifiés-et-mitigations)
9. [Plan jour-par-jour (2-3 j wallclock)](#9-plan-jour-par-jour-2-3-j-wallclock)
10. [Calibration & nouveaux SLO `D6-*`](#10-calibration--nouveaux-slo-d6)
11. [Anti-régression & realism check](#11-anti-régression--realism-check)
12. [Intégration CI + parser + baseline v0.1.6](#12-intégration-ci--parser--baseline-v016)
13. [Critères d'acceptation (DoD)](#13-critères-dacceptation-dod)
14. [Out of scope & roadmap follow-ups](#14-out-of-scope--roadmap-follow-ups)

---

## 1. Contexte & mission

### 1.1 Pourquoi D6 est P0 absolu

Le **test pratique réel** du 2026-04-25 (9 cas labélés multilingues
sur le catalog `support_triage`) a révélé que le système EFE
*compile, déploie, sert des requêtes gRPC en ~370 ms, expose
correctement les SLO de latence et de mémoire* — **et produit 50 %
d'accuracy en classification**.

Pour un classificateur 4-classes équiprobables, le baseline aléatoire
est 25 %. **EFE bat le hasard de 2× — un classificateur bien conçu
devrait être au-dessus de 80 %**, et l'embedder seul (cosine nearest
sur les anchors, sans swarm) atteint déjà ~78 % sur les mêmes inputs.

**L'essaim n'apporte aucune valeur — il en retire**.

Et — point critique — **aucun SLO bench actuel ne le détecte** :

| SLO existant | Mesure | Status post-test pratique |
|--------------|--------|:-------------------------:|
| `SLO-RD1-1..3` | latence p95 | ✅ PASS (180 / 370 / 370 ms) |
| `SLO-D2-*` | pool/arena | ✅ PASS (10/10) |
| `SLO-D3-*` | compute pipeline | ✅ PASS (3/3 naïve) |
| `SLO-D4-*` | gRPC stack | ✅ PASS (4/4) |
| `SLO-XXX-accuracy` | accuracy | **N'EXISTE PAS** |

Le système est **opérationnellement vert** sur tous les axes mesurés
**et fonctionnellement rouge** sur l'axe métier qui compte. C'est
l'angle mort que D6 ferme.

### 1.2 Mission D6 (énoncé chirurgical)

> 1. **Diagnostiquer la cause-racine** du `0-round consensus` /
>    `0.575 confidence flat` observé : remonter du symptôme
>    (4/8 wrong) au code Rust spécifique qui fait que l'essaim
>    n'évolue pas le centroïde.
>
> 2. **Implémenter un fix défensif en 3 couches** : (a) `min_rounds`
>    dans `ControllerConfig`, (b) durcir les seuils
>    `ConsensusConfig` Mode 1, (c) valider via le golden set.
>
> 3. **Câbler l'infrastructure eval déjà présente**
>    (`eval/scripts/run_eval.py`, 250 LOC, fonctionnel) **dans la
>    pipeline bench D5** — ajouter `SLO-D6-ACC-*` au parser, lancer
>    l'eval depuis `bench_runner.sh`, exposer le tableau accuracy
>    dans le rapport consolidé, gater la PR sur régression.
>
> 4. **Recalibrer la baseline `v0.1.6`** post-fix : mesurer la
>    nouvelle accuracy + la nouvelle latence (les fusion rounds
>    réels coûtent ~50 ms × N).

Hors scope : recalibrer le catalog (D7), ajouter d'autres modes
(M-extension), backend CUDA (Phase 2). D6 se concentre sur l'unique
sprint qui transforme l'EFE de « bench-vert / produit-rouge » à
« bench-vert / produit-mesurable-vert ».

### 1.3 Localisation

Le fix touche **trois fichiers Rust** + **deux fichiers Python**
+ **trois fichiers tooling** :

| Fichier | Modification | Rôle dans D6 |
|---------|--------------|--------------|
| `crates/efe-core/src/swarm/controller.rs` | + champ `min_rounds: u32` à `ControllerConfig` ; gating dans le `for round in 1..=max_rounds` loop | couche 1 du fix |
| `crates/efe-core/src/swarm/controller.rs` (test mod) | tests unitaires `min_rounds` | anti-régression |
| `crates/efe-server/src/mode1.rs` | `Mode1Config::default()` → `min_rounds=4`, `confidence_threshold=0.65` (au lieu de 0.50) | couche 2 du fix |
| `eval/scripts/run_eval.py` (déjà présent) | + flag `--output-bench-format` qui émet le JSON consommé par `parse_criterion_report.py` | wiring eval ↔ bench |
| `clients/python/efe_tool/client.py` | (rien — déjà fonctionnel) | utilisé tel quel |
| `scripts/parse_criterion_report.py` | + 4 SLO `D6-ACC-*` ; + nouvelle famille de keys eval | tooling |
| `scripts/bench_runner.sh` | + step optionnel `bench_accuracy` lancé via `RUN_ACCURACY=1` | tooling |
| `.github/workflows/bench.yml` | + nouveau job `bench-accuracy` parallèle au `bench-server` | CI |
| `bench_baselines/v0.1.6/baseline.json` | placeholder + (post-merge) baseline gelée | release |

---

## 2. Diagnostic du bug `0-round consensus`

### 2.1 Évidence empirique du test pratique

Sur les 9 cas du `demo_classify_isolated.py` :

| Cas | Texte | Cosine raw nearest | Server prédit | Confidence server |
|-----|-------|--------------------|:-------------:|:----------------:|
| EN-fraud | "Someone charged my card..." | **fraud (0.66)** | billing | 0.575 |
| EN-bug | "The app crashes..." | bug (0.65) | bug | 0.575 |
| EN-question | "How do I export my data..." | question (0.39) | question | 0.575 |
| EN-billing | "I want to cancel..." | billing (0.61) | billing | 0.575 |
| FR-fraud | "J'ai été débité..." | **fraud (0.67)** | billing | 0.575 |
| FR-bug | "L'application affiche..." | bug | billing | 0.575 |
| FR-question | "Comment modifier..." | question | billing | 0.575 |
| FR-billing | "Pouvez-vous m'envoyer..." | billing | billing | 0.575 |
| ADV-weather | "It's raining hard today..." | (toutes < 0.15) | (rejected via alignment breach) | n/a |

**Trois symptômes pathognomoniques** :

1. **`confidence` strictement constant à `0.5750…`** sur les 8 cas
   non-adversariaux, à 4 décimales près. Aucun bruit, aucune
   variation. Or l'embedder est déterministe et le serveur n'est pas
   stub — chaque input devrait produire un signal différent.

2. **4/8 wrong cases collapsent vers `billing`** (la même classe).
   Pas une distribution aléatoire d'erreurs.

3. **Les cas correctement classifiés ont une marge cosine > 0.30
   sur le 2e nearest** ; les wrong cases ont une marge < 0.25.
   Comportement compatible avec un **anchor blending** vers le
   centre du convex hull.

### 2.2 Analyse statique du flow `SwarmController::execute`

Lecture annotée de `crates/efe-core/src/swarm/controller.rs:220-271` :

```text
phase=fusion (logged)
let max_rounds = req.spec.max_rounds.get();    // = 16 in Mode 1
let mut detector = factory();                  // fresh per-request
let mut final_stats = None;
let mut last_variance = NaN;

for round in 1..=max_rounds {                  // 1..=16
    // 1. fusion always runs FIRST
    let stats = fusion.run_round(&mut els, &batch, round).await?;
    last_variance = stats.variance;
    if stats.has_numerical_issues() { return Err(...); }

    // 2. THEN consensus is consulted
    match detector.evaluate(round, &stats) {
        Verdict::Converged { confidence } => {
            final_stats = Some(stats.with_confidence(confidence));
            break;                              // ← exit loop
        }
        Verdict::Continue => continue,
        Verdict::Divergence { reason } => return Err(...),
        Verdict::ModeCollapse => { recover ; continue },
    }
}
```

**Lecture du detector `MultiCriteriaDetector::ingest`**
(`consensus/multi_criteria.rs:315`) :

```text
match (would-converge-this-round) {
    Yes => {
        self.consecutive_high += 1;
        if self.consecutive_high >= self.config.hysteresis_rounds {
            return Converged { confidence: ... };
        }
        // else: Continue
    }
    No => {
        self.consecutive_high = 0;
        // Continue
    }
}
```

Avec `hysteresis_rounds=2`, le détecteur émet `Converged` à la
2ᵉ frame consécutive où le score composite > `confidence_threshold`.

**Conclusion** : si `would-converge-this-round` retourne `Yes` dès
le round 1 (post 1ʳᵉ fusion) **et** dès le round 2 (post 2ᵉ fusion),
alors `Converged` fire au round 2. **La fusion DOES run** —
au moins 2 fois. **Le bug `rounds=0`** dans les logs est probablement
**un artefact du champ `FusionStats::round` initialisé à 0** dans
certaines branches stub/mock (à valider en D6.1).

Le vrai bug n'est donc **pas** « 0 rounds tournent ». C'est :

> **Avec les seuils Mode 1 actuels, l'essaim converge en 2 rounds
> (le minimum hystérésis), avant que le centroïde ait eu le temps
> d'évoluer significativement par rapport à l'état post-genesis.**

Et **post-genesis**, les 1024 agents sont initialisés via les 3 templates
(NaSem/NaLog/NaCrit) avec un état pondéré par le `MetaAnalyzer` qui
ne voit que l'embedding une fois (à la phase meta) et passe ensuite
à la cardinalité. Le centroïde post-genesis dépend faiblement de
l'input — il est dominé par les distributions templates.

### 2.3 Confirmation par les coûts mesurés

Le test pratique a mesuré ~600 ms par requête. Décomposition cohérente
avec D1-D3 :

| Phase | Coût attendu | Observé |
|-------|-------------:|--------:|
| genesis (1024 agents, 384 dim) | ~50-150 ms | (incluse) |
| fusion × 2 rounds @ 200 ms/round | ~400 ms | (incluse) |
| dissolution + alignment + decode | ~30-50 ms | (incluse) |
| **Total** | ~480-600 ms | **608 ms** ✓ |

**2 rounds = exactement la valeur d'hystérésis**. Confirmation
chiffrée.

---

## 3. Trois hypothèses de cause-racine, classées par évidence

### H1 — Hystérésis trop court (le plus probable, P=70 %)

`hysteresis_rounds = 2` est **insuffisant** pour Mode 1 sur 1024
agents en 384 dimensions. Les 6 signaux du détecteur (variance,
gradient_norm, attention_entropy, centroid_drift, outlier_ratio,
attention_entropy_evolution) montent rapidement parce que les
templates spawnés convergent vers leur propre clusters internes,
pas vers une représentation discriminative de l'input.

**Test diagnostique** : forcer `hysteresis_rounds=8` et observer
si le centroïde devient discriminant.

### H2 — `confidence_threshold` trop bas (P=60 %)

`confidence_threshold = 0.50` est **trop laxiste**. Le commentaire
mode1.rs admet l'avoir baissé pour éviter le `ConvergenceTimeout`
sur des essaims hyperconvergents. **Mais l'effet inverse s'est
produit** : la convergence trop précoce, avant évolution
significative.

**Test diagnostique** : monter à 0.70 + observer.

### H3 — Genesis-state biaisé vers une classe (P=40 %)

Le `HeuristicMetaAnalyzer` produit un DAG de tâches dont la
distribution est dominée par 1-2 classes, pas par l'input. Si la
distribution est systématiquement biaisée vers `billing` (ou un
proxy), tous les centroïdes post-genesis le sont aussi.

**Test diagnostique** : remplacer Mode 1 default consensus par un
détecteur stub qui force 16 rounds → mesurer si le centroïde
discrimine.

### Stratégie D6 : adresser H1 + H2 ensemble (le fix combiné)

Les hypothèses H1 et H2 sont **multiplicatives**. Augmenter
hystérésis seul, mais en gardant un seuil bas, ne change rien (le
détecteur fire toujours rapidement). Augmenter le seuil seul, mais
en gardant hystérésis=2, déplace juste la convergence d'un round
ou deux. **Le fix doit attaquer les deux**.

H3 est secondaire — si H1+H2 sont fixés et l'accuracy reste basse,
H3 entre en scène. Reportée au follow-up D7.

---

## 4. Architecture du fix : approche en 3 couches défensives

### 4.1 Couche 1 — `min_rounds` dans `ControllerConfig` (sécurité absolue)

**Pourquoi cette couche** : empêcher tout détecteur futur (ou tout
réglage par tenant) de re-causer le même bug. C'est la barre
mécanique : *la fusion DOIT tourner au moins N fois*.

**Modification** :

```text
// crates/efe-core/src/swarm/controller.rs

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ControllerConfig {
    pub mode_collapse_recovery: CollapseRecoveryPolicy,
    /// NEW (D6) : minimum number of fusion rounds before any
    /// `Converged` verdict from the detector is honoured. Below
    /// this floor, `Converged` is downgraded to `Continue`.
    /// Default 4. Must be >= 1 and <= max_rounds_observed.
    pub min_rounds: u32,
}

impl Default for ControllerConfig {
    fn default() -> Self {
        Self {
            mode_collapse_recovery: CollapseRecoveryPolicy::default(),
            min_rounds: 4,
        }
    }
}
```

**Modification du loop** :

```text
for round in 1..=max_rounds {
    let stats = fusion.run_round(...).await?;
    match detector.evaluate(round, &stats) {
        Verdict::Converged { confidence } if round < self.config.min_rounds => {
            // floor — keep going even though detector is satisfied
            tracing::debug!(round, min_rounds = %self.config.min_rounds,
                "consensus would converge but min_rounds not yet reached");
            // semantically: treat as Continue
            continue;
        }
        Verdict::Converged { confidence } => {
            final_stats = Some(stats.with_confidence(confidence));
            break;
        }
        Verdict::Continue => continue,
        // ... Divergence / ModeCollapse unchanged
    }
}
```

**Default value (post-implémentation) : library default = `0`,
Mode 1 default = `4`**.

> **Note d'implémentation** — l'architecture initiale proposait
> `min_rounds = 4` comme library default, mais ~12 tests (audit /
> failure-injection / properties) utilisent un `ScriptedDetector` à
> 1 frame + `MockFusion` à 1 frame qui auraient cassé silencieusement.
> Le fix réel : library default = `0` (legacy compat), Mode 1
> override à `4` via `Mode1Config::default().controller.min_rounds = 4`.
> Cette séparation localise le fix au profil concerné sans casser le
> harnais de test partagé. Cf. `crates/efe-core/src/swarm/controller.rs`
> docstring.

Justification du `4` :

* Mode 1 typique = 1024 agents, dim=384, n_anchors=4..16.
* Une fusion round complète balaye tous les agents, applique
  attention + softmax + aggregate + β-mix + project + clip.
* 4 rounds = 4 sweeps complets ⇒ chaque dimension du centroïde
  voit ~4 mises à jour ⇒ assez pour amortir le bruit d'init
  genesis sans gaspiller le budget latentiel.
* À ~50-70 ms/round (D3), 4 rounds = 200-280 ms — encore sous
  les SLO `RD1-1` (400 ms small) et `RD1-2` (700 ms medium).

**Cap supérieur** : pas de cap dur ; le `max_rounds` existant
(spec field) reste l'autorité haute. On documente que
`min_rounds <= max_rounds` doit être vrai (validation à la
construction du SwarmSpec ou du ControllerConfig).

### 4.2 Couche 2 — Durcir `Mode1Config::default()` (correction de politique)

**Pourquoi cette couche** : la cause directe du bug observé n'est
pas l'absence de `min_rounds` (qui est une protection), c'est le
**seuil de convergence trop bas**. Couche 2 ajuste les knobs Mode 1.

**Modifications dans `crates/efe-server/src/mode1.rs::Mode1Config::default()`** :

| Champ | Valeur actuelle | Valeur D6 | Rationale |
|-------|---------------|-----------|-----------|
| `consensus.confidence_threshold` | `0.50` | **`0.65`** | Force des fusions où le score composite des 6 signaux dépasse vraiment la moitié du chemin vers 1.0 — pas juste « légèrement au-dessus de la médiane ». |
| `consensus.hysteresis_rounds` | `2` | **`3`** | 2 frames consécutives au-dessus du seuil ne suffisent pas à amortir le bruit ; 3 fait le job sans exploser le budget latentiel. |
| `consensus.variance_threshold` | `0.05` | **`0.02`** | La variance post-genesis est typiquement 0.04-0.06 ⇒ avec 0.05 le seuil est trivialement satisfait dès le 1ᵉʳ round. À 0.02, il faut une réelle convergence. |
| `consensus.entropy_fraction_max` | `0.98` | `0.95` | Force que l'entropie d'attention ait baissé sensiblement (5 % au lieu de 2 %) avant convergence. |
| `consensus.centroid_drift_max` | `0.20` | `0.10` | Le centroïde doit s'être stabilisé à 10 % près entre 2 rounds ⇒ les agents ont atteint un état stable. |
| `consensus.outlier_ratio_max` | `0.30` | `0.20` | Au plus 20 % d'agents en outlier (vs 30 %), gage que la majorité est vraiment alignée. |

**Aucun changement aux autres champs** (`fusion`, `prealloc_arenas`,
`max_arenas`, `n_streams`).

**Effet attendu** : la convergence détectée se déplace du round
2-3 (état actuel) vers le round 5-8 (état D6). Latence p95 small
passe de 180 ms à ~300-400 ms, **toujours dans le budget
`SLO-RD1-1=400 ms`** (calibré post-D5 audit).

### 4.3 Couche 3 — Validation par le golden set (ceinture mesurée)

**Pourquoi cette couche** : couches 1+2 sont des hypothèses guidées
par l'évidence. Mais on doit **prouver** qu'elles fixent l'accuracy
end-to-end, pas juste qu'elles font tourner plus de rounds.

**Méthode** : après chaque modif du fix, lancer
`eval/scripts/run_eval.py --catalog catalogs/support_triage.yaml
--golden eval/golden/support_triage.jsonl --endpoint 127.0.0.1:50051`
et exiger :

* `accuracy_top1 >= 0.75` (gate du script existant)
* `macro_f1 >= 0.70`
* `false_reject_rate <= 0.25`
* `mean_conf_correct >= 0.65`

Si une de ces métriques régresse, **revenir à la couche concernée**
et durcir/relâcher.

### 4.4 Diagram de décision

```text
                  apply Layer 1 + Layer 2
                            │
                            ▼
                run eval/scripts/run_eval.py
                            │
                ┌───────────┴────────────┐
                ▼                        ▼
        accuracy >= 0.75        accuracy < 0.75
                │                        │
                ▼                        ▼
          freeze v0.1.6            Activate H3 path :
                                   - check genesis bias
                                   - tune meta-analyzer
                                   - re-run
```

---

## 5. L'infrastructure eval déjà présente dans le repo

### 5.1 Fichiers existants

| Fichier | LOC | Rôle |
|---------|----:|------|
| `eval/scripts/run_eval.py` | 250 | Runner end-to-end : embedder → catalog → gRPC → métriques |
| `eval/golden/support_triage.jsonl` | 40 | Dataset labellé : 40 exemples × {id, catalog, text, expected, language} |
| `clients/python/efe_tool/client.py` | (~300) | `EfeClient.classify()` consommé par run_eval |
| `catalogs/support_triage.yaml` | (~100) | Catalog avec 4 classes, anchors compilés |
| `catalogs/support_triage.compiled.json` | (~500) | Anchors numpy sérialisés (cached) |

### 5.2 Surface API du runner existant

`run_eval.py` expose :

```text
GATES = {
    "accuracy_top1_min": 0.75,
    "macro_f1_min": 0.70,
    "false_reject_rate_max": 0.25,
    "mean_conf_correct_min": 0.65,
}

def run(catalog, golden, endpoint, output=None, verbose=False) -> int
def compute_metrics(...) -> dict[str, Any]
def _print_report(report, labels) -> None
def _main() -> int      # entry point (--catalog --golden --endpoint --output)
```

Le runner :

1. Charge le catalog YAML (lazy-compiles les anchors si absent).
2. Charge le golden JSONL (40 lignes).
3. Pour chaque ligne, appelle `EfeClient.classify(text, catalog)`.
4. Compute precision/recall/F1 par classe + macro F1 + matrice de
   confusion + calibration.
5. Si `--output X.json`, dump le rapport.
6. Si une gate est violée, exit 1.

### 5.3 Format JSON produit

(à inspecter en D6.1 pour confirmer compatibilité avec le parser
post-extension — voir §6.2)

```text
{
  "catalog": "support_triage@1.0.0",
  "n_examples": 40,
  "accuracy_top1": 0.825,
  "macro_f1": 0.781,
  "per_class": {
    "fraud":    { "precision": ..., "recall": ..., "f1": ..., "support": 10 },
    ...
  },
  "confusion_matrix": [[...]],
  "false_reject_rate": 0.05,
  "mean_conf_correct": 0.74,
  "mean_conf_wrong": 0.52,
  "latency_ms": { "mean": ..., "p50": ..., "p95": ..., "p99": ... },
  "duration_s": 32.1
}
```

### 5.4 Modifications nécessaires (D6.3)

* **Ajouter une option `--output-bench-format`** qui émet en plus
  d'un format compatible `parse_criterion_report.py` (clés sous
  forme `<group>::<id>` → `{ "median_ns" / "approx_p95_ns" / ... }`).

  Mapping :

  | Eval metric | Bench key | Unit |
  |-------------|-----------|------|
  | `accuracy_top1` × 1e9 | `eval_accuracy::top1` | (clé numérique)* |
  | `macro_f1` × 1e9 | `eval_accuracy::macro_f1` | (clé numérique)* |
  | `mean_conf_correct` × 1e9 | `eval_accuracy::mean_conf_correct` | (clé numérique) |
  | `false_reject_rate` × 1e9 | `eval_accuracy::false_reject_rate` | (clé numérique) |
  | `latency_ms.p95` × 1e6 | `eval_latency::p95_ns` | ns |

  *Les valeurs accuracy ∈ [0, 1] sont scalées par 1e9 pour utiliser
  l'infrastructure ns-based du parser sans modif. Le SLO compare
  `measured_ns >= target_ns` après ce scaling — convention inverse
  pour le sens (plus haut = mieux). Voir §10.

---

## 6. Câblage de l'eval dans la pipeline bench (D5)

### 6.1 Architecture cible : 1 nouveau job CI + 1 nouveau type de SLO

```text
┌─────────────────────────────────────┐
│  bench-server (existant — D4)       │  ← ne change pas
├─────────────────────────────────────┤
│  bench-accuracy (NOUVEAU D6)        │
│  1. Build efe-server release        │
│  2. Lance le serveur en bg          │
│  3. python eval/scripts/run_eval.py │
│     --output-bench-format eval.json │
│  4. Upload eval.json + html report  │
└─────────────────────────────────────┘
        │
        ▼
┌─────────────────────────────────────┐
│  bench-aggregate (existant)          │
│  + scripts/aggregate_bench_reports.py│
│  + parse_criterion_report.py         │
│  ← MODIFIÉ : merge eval.json en plus │
└─────────────────────────────────────┘
        │
        ▼
   Rapport unifié + SLO compliance
   (latency + memory + compute + RPC + ACCURACY)
```

### 6.2 Modifications `parse_criterion_report.py`

```text
# Nouvelle famille SLO (cf. §10) :

WANTED_PREFIXES = (
    # ... existing ...
    "eval_accuracy",      # accuracy / macro_f1 / etc.
    "eval_latency",        # client-side observed latency
)

SLO_DEFINITIONS += [
    {
        "id": "SLO-D6-ACC-1",
        "name": "Top-1 accuracy on golden set",
        "bench_key": "eval_accuracy::top1",
        "target_ns": 750_000_000,    # 0.75 × 1e9 (scaling)
        "comparator": "gte",          # NEW : measured >= target
    },
    {
        "id": "SLO-D6-ACC-2",
        "name": "Macro-F1 on golden set",
        "bench_key": "eval_accuracy::macro_f1",
        "target_ns": 700_000_000,    # 0.70 × 1e9
        "comparator": "gte",
    },
    {
        "id": "SLO-D6-ACC-3",
        "name": "Mean confidence on correct predictions",
        "bench_key": "eval_accuracy::mean_conf_correct",
        "target_ns": 650_000_000,    # 0.65 × 1e9
        "comparator": "gte",
    },
    {
        "id": "SLO-D6-ACC-4",
        "name": "False reject rate (lower is better)",
        "bench_key": "eval_accuracy::false_reject_rate",
        "target_ns": 250_000_000,    # 0.25 × 1e9
        "comparator": "lte",          # measured <= target (default)
    },
]
```

**Petite extension du parser** : ajouter le champ `comparator` aux
SLO_DEFINITIONS, avec les 2 valeurs `lte` (par défaut, latence) et
`gte` (nouveau, accuracy). `evaluate_slos()` route sur le bon
comparateur.

### 6.3 Modification `bench_runner.sh`

Nouveau flag `RUN_ACCURACY=1` :

```text
if [ "${RUN_ACCURACY:-}" = "1" ]; then
  echo "==> Running accuracy eval (D6)"
  python3 eval/scripts/run_eval.py \
      --catalog catalogs/support_triage.yaml \
      --golden eval/golden/support_triage.jsonl \
      --endpoint 127.0.0.1:50051 \
      --output target/eval-accuracy.json \
      --output-bench-format target/eval-accuracy-bench.json \
      || echo "WARN: eval gate failed (continuing — gate enforced in CI)"
fi
```

### 6.4 Modification `bench.yml`

Nouveau job `bench-accuracy` :

```yaml
bench-accuracy:
  name: Accuracy eval on golden set
  needs: bench-build
  runs-on: ubuntu-latest
  timeout-minutes: 15
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
    - uses: Swatinem/rust-cache@v2
      with: { shared-key: bench-build-d5 }
    - uses: actions/setup-python@v5
      with: { python-version: '3.12' }
    - run: |
        pip install -e clients/python
    - name: Start efe-server in background
      run: |
        cargo run -p efe-server --release &
        echo $! > server.pid
        sleep 10  # wait for bind
    - name: Run eval
      run: |
        python3 eval/scripts/run_eval.py \
            --catalog catalogs/support_triage.yaml \
            --golden  eval/golden/support_triage.jsonl \
            --endpoint 127.0.0.1:50051 \
            --output target/eval-accuracy.json \
            --output-bench-format target/eval-accuracy-bench.json
    - name: Stop server
      if: always()
      run: kill $(cat server.pid) || true
    - name: Upload eval artefact
      if: always()
      uses: actions/upload-artifact@v4
      with:
        name: bench-partial-accuracy-${{ github.sha }}
        path: |
          target/eval-accuracy.json
          target/eval-accuracy-bench.json
```

Ajouter `bench-accuracy` à `needs:` du job `bench-aggregate` pour
que les SLO accuracy apparaissent dans le rapport unifié + le
PR comment.

---

## 7. 12 décisions de conception clés

| # | Décision | Alternative rejetée | Rationale |
|---|----------|---------------------|-----------|
| D1 | Fix en 3 couches (controller + config + eval-validation) | Fix uniquement en config (couche 2) | Couche 1 (`min_rounds`) protège contre tout futur tuning erroné ; couche 3 (eval) prouve numériquement le fix |
| D2 | `min_rounds` dans `ControllerConfig` (vs `ConsensusConfig`) | détecteur retourne `Continue` si round < min_rounds | Séparation des responsabilités : detector évalue les stats, controller orchestre. Le min_rounds est une politique d'orchestration |
| D3 | Default `min_rounds = 4` (vs 2 ou 8) | 2 (minimum) ou 8 (paranoïaque) | 4 sweeps suffisent pour amortir le bruit d'init genesis sur 1024 agents en 384D ; reste sous SLO latence calibré |
| D4 | `confidence_threshold` Mode 1 = 0.65 (vs 0.50 ou 0.80) | 0.50 (actuel) ou 0.80 (paranoïaque) | 0.65 force une convergence non-triviale ; 0.80 risque de timeout sur essaims convergeants lentement |
| D5 | Réutiliser `eval/scripts/run_eval.py` existant (vs réécrire en Rust) | rebuild en Rust | 250 LOC Python fonctionnel, déjà testé. La duplication Rust ajouterait du code mort |
| D6 | Scaling `accuracy × 1e9` pour clés numériques | type SLO séparé pour ratios | Réutilise tout le pipeline ns-based du parser sans changer son schéma. Convention documentée. |
| D7 | Comparateur `gte` ajouté au parser (vs prefix-based) | hardcoder par bench_key | Plus extensible : tout futur SLO « plus haut = mieux » réutilise le mécanisme |
| D8 | Eval lance le SERVEUR end-to-end (vs mock backend) | mock backend en Python | L'objectif est de prouver que **le système réel** classifie bien, pas que le code Python est correct |
| D9 | 1 job CI dédié `bench-accuracy` (vs intégrer dans `bench-server`) | bench-server étendu | Indépendance : un fail eval n'invalide pas les SLO latence ; le timeout (15 min) est dimensionné pour eval (server boot + 40 RPCs) |
| D10 | Pas de TLS, pas d'auth pour eval CI (vs prod-fidèle) | TLS + JWT signing dans le job | TLS/auth ajoutent du flake CI sans valeur pour la mesure d'accuracy. D4.5 / D4.6 couvrent ces axes |
| D11 | `min_rounds` validé `<= max_rounds` au build du `SwarmSpec` (vs au runtime) | validation runtime | Fail-fast : refus à la construction = erreur claire, pas un panic au round N |
| D12 | Default `min_rounds=4` partagé Mode 1 ET Mode 2/3 futurs | per-mode override | Politique de plancher commune ; les modes spécifiques peuvent override via leur Config si justifié |

---

## 8. 10 risques identifiés et mitigations

### R1 — Le fix `min_rounds=4` casse les SLO latence existants

**Impact** : passer de 2 rounds à 4 rounds = ×2 sur le coût fusion.
Latence small passe de ~180 ms à ~300-400 ms. SLO `RD1-1=400 ms`
risque le FAIL.

**Mitigation** :
* Calibration v0.1.5 (post-D5 audit) déjà à 400 ms. ×2 latence
  reste proche de la limite mais sous-cap.
* En cas de FAIL CI, recalibrer SLO `RD1-1` à 500 ms (toujours dans
  les budgets prod) après confirmation accuracy.

### R2 — `confidence_threshold=0.65` cause des `ConvergenceTimeout`

**Impact** : si l'essaim ne franchit jamais 0.65 sur 16 rounds
(max_rounds), le controller émet `ConvergenceTimeout`. Status gRPC
= `RESOURCE_EXHAUSTED`. Le client voit l'échec.

**Mitigation** :
* Bumper `max_rounds` du SwarmSpec Mode 1 à 32 (depuis 16) si
  observé.
* Test eval sur la golden set vérifie que < 5 % des cas sortent
  en timeout.

### R3 — Eval requires Python deps non disponibles sur runner CI

**Impact** : `sentence-transformers` + `torch` = ~2 GB de deps.
Cold install peut prendre ~3 min sur runner CI sans cache.

**Mitigation** :
* Cache pip via `actions/setup-python@v5` avec key `pip-st-2.7`.
* Si timeout, basculer eval sur runner self-hosted Mac M3 Pro
  (plus rapide, déjà a les wheels).

### R4 — Le serveur en background du job CI ne boot pas en 10s

**Impact** : `sleep 10` insuffisant ⇒ premier RPC échoue avec
`ECONNREFUSED`.

**Mitigation** :
* Polling avec `nc -z 127.0.0.1 50051` jusqu'à OK ou 60s timeout.
* Documenté dans le step yml.

### R5 — Le bench `min_rounds=4` change les chiffres de tous les D1-D5

**Impact** : `real_strategies_e2e_naive::small` mesurait ~180 ms,
passera à ~350 ms. La baseline `v0.1.4` devient obsolète, le
regression check dérivera.

**Mitigation** :
* Freeze nouvelle baseline `v0.1.6` post-fix.
* `check_regression.py` détectera l'écart au premier run, mais le
  `_note` du `v0.1.4` documentera l'invalidation.

### R6 — H3 (genesis bias) est la vraie cause, H1+H2 ne suffisent pas

**Impact** : après fix D6, accuracy reste sous 75 %. Le genesis
biaise toujours.

**Mitigation** :
* Plan B documenté : modifier le `MetaAnalyzer` pour mieux
  pondérer l'embedding input (D7 follow-up).
* Critère DoD : si accuracy < 70 % après D6.4, escalader en D7.

### R7 — `--output-bench-format` casse `run_eval.py` existant

**Impact** : ajouter un flag risque de régresser le mode classique
(stdout report).

**Mitigation** :
* Le flag est OPT-IN ; absence = comportement inchangé.
* Test `test_run_eval_basic` (déjà présent ?) tourne dans CI.

### R8 — Le scaling `accuracy × 1e9` produit des nombres fragiles

**Impact** : 0.75 × 1e9 = 750_000_000. Un drift de 0.01 = 10 M ns —
parser le voit comme une régression de 1.3 %.

**Mitigation** :
* La `noise_threshold` du parser sur ce groupe est fixée à 0.5 %
  (au lieu de 5 %) — accuracy bouge peu d'un run à l'autre si le
  modèle ne change pas.

### R9 — Le golden set est trop petit (40 exemples)

**Impact** : variance statistique élevée. Une baseline change si
1-2 cas basculent par hasard.

**Mitigation** :
* À long terme : étendre à 200+ exemples (D7).
* À court terme : 40 sur 4 classes ⇒ ~10/classe ⇒ macro-F1
  fiable à ±0.05.

### R10 — Le test pratique a été fait sur l'embedder L12-multilingue mais Mode1Config par défaut sait juste dim=384 — y a-t-il un mismatch d'embedder dans CI ?

**Impact** : si CI utilise `all-MiniLM-L6-v2` (English-only) sur le
golden multilingue, les French cases échoueront pour la mauvaise
raison.

**Mitigation** :
* Le job `bench-accuracy` exporte
  `EFE_EMBEDDER_MODEL=sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2`.
* Le `--output-bench-format` inclut le `embedder_fingerprint` pour
  audit.

---

## 9. Plan jour-par-jour (2-3 j wallclock)

### D6.1 (½ j) — Diagnostic confirmé

* **D6.1.1** : Re-run le test pratique avec `RUST_LOG=efe_core::swarm=trace`
  → captures ligne par ligne ce qui se passe round par round.
* **D6.1.2** : Vérifier que `FusionStats::round` est bien renseigné
  par MvpCpuFusion (pas un faux 0).
* **D6.1.3** : Confirmer la valeur de `consecutive_high` à chaque
  round du `MultiCriteriaDetector`.
* **D6.1.4** : Documenter le diagnostic final dans
  `docs/audits/d6_accuracy_validation.md` (le diagnostic D6.1
  et la validation accuracy D6.4 ont été fusionnés en un seul
  rapport ; voir la section « Diagnostic » de ce fichier).

**DoD** :
- [ ] Cause-racine confirmée (H1 vs H2 vs H3).
- [ ] Audit doc créé.

### D6.2 (½ j) — Couche 1 : `min_rounds` dans ControllerConfig

* **D6.2.1** : Ajouter `min_rounds: u32` à `ControllerConfig`.
* **D6.2.2** : Modifier le loop `controller.rs:230-271` pour
  downgrade `Converged` en `Continue` quand round < min_rounds.
* **D6.2.3** : Ajouter validation : si `min_rounds > max_rounds`
  (depuis le SwarmSpec), erreur de config.
* **D6.2.4** : Tests unitaires (3 cas) :
  - `controller_respects_min_rounds_floor`
  - `controller_validates_min_rounds_le_max_rounds`
  - `controller_min_rounds_zero_is_legacy_behavior`

**DoD** :
- [ ] Code compile clean.
- [ ] 3 tests pass.
- [ ] `cargo test -p efe-core controller` vert.

### D6.3 (½ j) — Couche 2 : `Mode1Config` durci

* **D6.3.1** : Modifier `Mode1Config::default()` avec les 6 nouvelles
  valeurs (cf. §4.2 tableau).
* **D6.3.2** : Updater le commentaire en tête de la fonction pour
  expliquer le rationale du durcissement post-D6.
* **D6.3.3** : Lancer le test pratique `demo_classify_isolated.py`
  → vérifier que `rounds_executed >= 4` et que `confidence` varie
  par input (plus de constante 0.575).

**DoD** :
- [ ] `Mode1Config::default()` updated.
- [ ] Test pratique : confidence varie + rounds >= 4.

### D6.4 (½ j) — Validation accuracy

* **D6.4.1** : Lancer `eval/scripts/run_eval.py` sur le golden set.
* **D6.4.2** : Vérifier `accuracy >= 0.75`, `macro_f1 >= 0.70`.
* **D6.4.3** : Si accuracy < 0.75 : escalader en H3 / D7.
  Sinon : poursuivre.
* **D6.4.4** : Capturer la matrice de confusion + calibration ⇒
  documenter dans `docs/audits/d6_accuracy_validation.md`.

**DoD** :
- [ ] accuracy >= 0.75 confirmé.
- [ ] Audit doc avec matrice confusion + commentaires.

### D6.5 (½ j) — Câblage tooling + CI

* **D6.5.1** : Ajouter `--output-bench-format` à
  `eval/scripts/run_eval.py`.
* **D6.5.2** : Étendre `WANTED_PREFIXES` + `SLO_DEFINITIONS` du
  parser.
* **D6.5.3** : Ajouter `comparator: gte/lte` dans
  `evaluate_slos()`.
* **D6.5.4** : Étendre `bench_runner.sh` avec `RUN_ACCURACY=1`.
* **D6.5.5** : Ajouter le job `bench-accuracy` dans `bench.yml`.
* **D6.5.6** : Mettre à jour `BENCHMARKS.md` avec § 8 « Accuracy
  SLO (Sprint D6) ».

**DoD** :
- [ ] Parser reconnaît les nouveaux SLO.
- [ ] CI workflow lance le job accuracy.
- [ ] BENCHMARKS.md updated.

### D6.6 (¼ j) — Baseline v0.1.6 + freeze

* **D6.6.1** : Re-run `bench_runner.sh` complet (latency + memory +
  compute + RPC + accuracy).
* **D6.6.2** : Copier le rapport consolidé vers
  `bench_baselines/v0.1.6/baseline.json`.
* **D6.6.3** : Mettre à jour `_note` avec le contexte D6.

**DoD** :
- [ ] Baseline v0.1.6 freezée.
- [ ] Tous les SLO PASS (incluant `D6-ACC-*`).

---

## 10. Calibration & nouveaux SLO `D6-*`

Référence host : Apple M3 Pro, no-BLAS, embedder L12-multilingue,
catalog `support_triage`.

### 10.1 Cibles bench accuracy (gate « plus haut = mieux »)

| ID | Nom | Bench key | Cible (accuracy ratio) | comparator |
|----|-----|-----------|------------------------:|:----------:|
| `SLO-D6-ACC-1` | Top-1 accuracy on golden set | `eval_accuracy::top1` | **0.75** (× 1e9 = 750 M ns) | gte |
| `SLO-D6-ACC-2` | Macro-F1 on golden set | `eval_accuracy::macro_f1` | **0.70** | gte |
| `SLO-D6-ACC-3` | Mean confidence on correct | `eval_accuracy::mean_conf_correct` | **0.65** | gte |
| `SLO-D6-ACC-4` | False reject rate (lower better) | `eval_accuracy::false_reject_rate` | **0.25** | lte |

### 10.2 Cibles bench latence client-side

Le runner mesure aussi la latence vue côté client (différente de la
latence serveur D1, car incluant transport + post-processing nearest
anchor).

| ID | Nom | Bench key | Cible | comparator |
|----|-----|-----------|------:|:----------:|
| `SLO-D6-LAT-1` | Eval p95 client latency | `eval_latency::p95_ns` | ≤ **1.0 s** | lte |

### 10.3 Coefficient CI

`bench.yml` applique automatiquement ×1.5 sur les cibles latence.
**Pas de coefficient sur les cibles accuracy** — l'accuracy ne
dépend pas de la vitesse du runner.

---

## 11. Anti-régression & realism check

**Fichier nouveau** :
`crates/efe-core/tests/controller_min_rounds_realism.rs`

### Test 1 — `min_rounds_floor_is_respected`

Construit un controller avec `min_rounds=4` + un `MockDetector`
qui retourne `Converged` au round 1. Vérifie que la fusion tourne
4 fois quand-même.

### Test 2 — `min_rounds_zero_is_legacy_behavior`

`min_rounds=0` ⇒ fusion break au round 1 (comme avant D6).
Backward-compat préservée.

### Test 3 — `min_rounds_validated_against_max_rounds`

`min_rounds=10, max_rounds=8` ⇒ erreur de config au build.

### Test 4 — `mode1_config_default_min_rounds_is_4`

`Mode1Config::default().min_rounds == 4`.

### Test 5 — `mode1_config_thresholds_match_d6_calibration`

Test de pin chiffré : `confidence_threshold == 0.65`,
`hysteresis_rounds == 3`, `variance_threshold == 0.02`, etc.
Si quelqu'un modifie ces valeurs sans mettre à jour D6, le test
trippe.

---

## 12. Intégration CI + parser + baseline v0.1.6

### 12.1 `parse_criterion_report.py` — extension

* `WANTED_PREFIXES` += `("eval_accuracy", "eval_latency")`
* `SLO_DEFINITIONS` += 4 SLO `D6-ACC-*` + 1 SLO `D6-LAT-1`
* `evaluate_slos()` : nouveau champ `comparator` (default `lte`,
  alternative `gte`)
* `load_criterion_estimates()` : reconnaît un nouveau format
  optionnel `--source eval` (clés sans `/new/estimates.json` mais
  un fichier `eval-accuracy-bench.json` direct)

### 12.2 `bench_runner.sh` — `RUN_ACCURACY` flag

```text
if [ "${RUN_ACCURACY:-}" = "1" ]; then
  echo "==> Running accuracy eval (D6)"
  # ... server start, eval, server stop ...
fi
```

### 12.3 `bench.yml` — nouveau job

`bench-accuracy` (cf. §6.4) — 15 min timeout, parallel à
`bench-server`, `needs: bench-build`.

### 12.4 `bench_baselines/v0.1.6/baseline.json`

Placeholder identique au pattern v0.1.{1..4}, avec :
* `_note` documentant le fix D6.
* `slo_compliance` listant les nouveaux SLO `D6-*` en `skipped: true`
  jusqu'au premier post-merge run.

### 12.5 `BENCHMARKS.md` — nouvelle § 8

Après § 7 (Reproductibilité D5), insérer :

```markdown
## 8. Accuracy SLO (Sprint D6, v0.1.6)

Mesures end-to-end de l'accuracy de classification sur le golden
set `eval/golden/support_triage.jsonl` (40 exemples multilingues
EN+FR sur 4 classes).

### 8.1 Bench accuracy

| Régime | Bench key | mesuré (M3 Pro) | SLO |
|--------|-----------|----------------:|----:|
| Top-1 accuracy | `eval_accuracy::top1` | **TBD post-fix** | ≥ 0.75 (`D6-ACC-1`) |
| Macro-F1 | `eval_accuracy::macro_f1` | **TBD** | ≥ 0.70 (`D6-ACC-2`) |
| Mean conf correct | `eval_accuracy::mean_conf_correct` | **TBD** | ≥ 0.65 (`D6-ACC-3`) |
| False reject rate | `eval_accuracy::false_reject_rate` | **TBD** | ≤ 0.25 (`D6-ACC-4`) |
| Eval p95 latency client-side | `eval_latency::p95_ns` | **TBD** | ≤ 1.0 s (`D6-LAT-1`) |

### 8.2 Découverte du sprint

Le sprint D6 a fixé un bug critique exposé par le test pratique
multilingue : `0-round consensus` (le `MultiCriteriaDetector`
`Converged` après seulement 2 rounds de fusion, avant que le
centroïde n'ait évolué significativement). Fix en 3 couches :

1. `min_rounds=4` dans `ControllerConfig` (sécurité absolue).
2. `Mode1Config::default()` durci (6 seuils consensus
   recalibrés).
3. Validation par golden set (`accuracy >= 0.75` requis).

Avant D6 : 4/8 cases multilingues prédisaient `billing` (50 %).
Après D6 : **TBD post-fix mesuré**.
```

---

## 13. Critères d'acceptation (DoD)

### 13.1 Code & build

- [x] `crates/efe-core/src/swarm/controller.rs` : `ControllerConfig`
      a un champ `min_rounds: u32`, **library default 0** (deviation
      from architecture §4.1 — see implementation note), validation
      `min_rounds <= max_rounds`.
- [ ] `crates/efe-core/src/swarm/controller.rs` : le loop fusion
      respecte `min_rounds` (downgrade de Converged → Continue
      quand round < min_rounds).
- [ ] `crates/efe-server/src/mode1.rs` : `Mode1Config::default()` a
      les 6 seuils D6 + le commentaire mis à jour.
- [ ] `cargo build --workspace --release` clean.
- [ ] `cargo clippy --workspace --benches --tests -D warnings` clean.

### 13.2 Tests

- [ ] `crates/efe-core/tests/controller_min_rounds_realism.rs` créé,
      5 tests, tous PASS.
- [ ] `cargo test --workspace` (suite complète, 1 683+ tests),
      tous PASS.

### 13.3 Eval

- [ ] `eval/scripts/run_eval.py` accepte `--output-bench-format X.json`.
- [ ] Exécution sur golden support_triage : `accuracy >= 0.75`,
      `macro_f1 >= 0.70`.
- [ ] Confidence varie par input (plus de constante 0.575).
- [ ] `rounds_executed >= 4` dans les server logs.

### 13.4 Tooling

- [ ] `scripts/parse_criterion_report.py` reconnaît
      `eval_accuracy::*` et `eval_latency::*`.
- [ ] `evaluate_slos()` supporte `comparator: gte/lte`.
- [ ] `scripts/bench_runner.sh` lance l'eval avec `RUN_ACCURACY=1`.
- [ ] `.github/workflows/bench.yml` a un job `bench-accuracy`
      qui parallèle `bench-server`.

### 13.5 Documentation

- [ ] `docs/bench/d6_consensus_fix_accuracy_slo_architecture.md`
      (ce document) référencé dans `BENCHMARKS.md`.
- [ ] `BENCHMARKS.md` § 8 « Accuracy SLO (D6) » ajoutée.
- [x] `docs/audits/d6_accuracy_validation.md` couvre à la fois
      le diagnostic D6.1.4 (§ Diagnostic) et la matrice de
      confusion D6.4.4 — les deux livrables planifiés ont été
      fusionnés en un seul rapport.

### 13.6 Baseline & validation

- [ ] `bench_baselines/v0.1.6/baseline.json` placeholder créé.
- [ ] (Post-merge) baseline v0.1.6 freezée avec valeurs réelles.
- [ ] Le test pratique `demo_classify_isolated.py` montre les 8
      cas correctement classifiés (vs 4/8 pré-D6).

---

## 14. Out of scope & roadmap follow-ups

### 14.1 Hors-scope D6

* **H3 (genesis bias) fix** — si D6.4 montre que H1+H2 ne suffisent
  pas, escalader en D7.
* **Étendre golden set à 200 exemples** — D8 follow-up.
* **Multi-catalog accuracy** (autres scenarios que support_triage)
  — D9.
* **Auth ON dans eval** — D6.5 a tranché : pas de TLS/JWT en eval CI.
  Coverage à activer en D4.5/D4.6.
* **CUDA backend benchmark** — Phase 2.
* **Mode 2 / Mode 3 evals** — quand ces modes existent.

### 14.2 Roadmap après D6

| Sprint | Cible | Justification |
|--------|-------|---------------|
| **D7** | Genesis bias remediation (H3) | Si l'accuracy plafonne malgré D6 |
| **D8** | Étendre golden set à 200+ exemples | Réduire variance statistique |
| **D9** | Multi-catalog eval (fraud_alert, billing_escalation) | Couverture des 3 scenarios builtin |
| **D10** | Bench LLM bridges (Claude/OpenAI) | Letter C complétion |
| **D11** | Production deployment validation | Helm + k8s + observability + load |

### 14.3 Métrique de succès post-D6

À 30 jours après merge D6 :

> *« Toute régression > 5 % sur l'accuracy top-1 du golden set
> support_triage produit un fail SLO bench-niveau dans les 24 h après
> le merge, avec une attribution exacte au composant responsable
> (controller, consensus config, genesis, fusion, dissolution,
> alignment). Les SLO `D6-ACC-1..4` sont gated en CI au même titre
> que les SLO latence. La PR comment expose le delta accuracy
> directement à côté des deltas latence. »*

Si cette affirmation tient, D6 a rempli sa mission.

---

## Annexe A — Schéma du flow controller post-D6

```text
  request
     │
     ▼
  ┌────────────────────────────┐
  │ Phase 1 : Genesis           │
  │ spawn 1024 agents           │
  └─────────────┬───────────────┘
                ▼
  ┌────────────────────────────┐
  │ Phase 2 : Fusion loop       │
  │ for round in 1..=max_rounds │
  │ {                            │
  │   stats = fusion.run_round() │
  │   match detector.evaluate() {│
  │     Converged if round < min │
  │       → Continue             │  ← NEW D6
  │     Converged                │
  │       → break                │
  │     Continue → continue      │
  │     Divergence → Err         │
  │     ModeCollapse → recover   │
  │   }                          │
  │ }                            │
  └─────────────┬───────────────┘
                ▼
  ┌────────────────────────────┐
  │ Phase 3 : Dissolution       │
  └─────────────┬───────────────┘
                ▼
  ┌────────────────────────────┐
  │ Phase 4 : Alignment         │
  │ check delta < cone radius   │
  └─────────────┬───────────────┘
                ▼
  ┌────────────────────────────┐
  │ Phase 5 : Decode            │
  └─────────────┬───────────────┘
                ▼
  response
```

## Annexe B — Eval pipeline data flow

```text
  catalogs/support_triage.yaml ──┐
  + support_triage.compiled.json─┤
                                 ▼
                     LocalSTEmbedder (L12 multilingual)
                                 │
                                 ▼
                         AnchorCatalog (4 classes)
                                 │
                                 ▼
  eval/golden/support_triage.jsonl
  (40 examples × {id, text, expected, language})
                                 │
                                 ▼
                         EfeClient.classify()
                                 │
                       ┌─────────┴───────────┐
                       ▼                     ▼
              gRPC SwarmService         (latency timer)
                       │                     │
                       ▼                     │
              compute_metrics()              │
                       │                     │
                       ▼                     ▼
              eval-accuracy.json   eval-accuracy-bench.json
              (human report)        (parser-compatible)
                                          │
                                          ▼
                          parse_criterion_report.py
                                          │
                                          ▼
                            bench-report-aggregated.json
                                          │
                                          ▼
                    show_bench_report.py / PR comment
```

## Annexe C — Glossary

| Terme | Définition |
|-------|------------|
| `0-round consensus` | bug où le `MultiCriteriaDetector` émet `Converged` après 2 rounds (le minimum hystérésis) avant que le centroïde n'ait évolué significativement |
| `min_rounds` | nouveau champ de `ControllerConfig` (D6) ; downgrade `Converged → Continue` tant que `round < min_rounds` |
| `anchor blending` | propriété de la projection cone : tout centroïde hors-cone est tiré vers le convex hull des anchors → perte de discrimination |
| `golden set` | dataset labellé canonique utilisé pour mesurer l'accuracy sans fluctuation aléatoire |
| `macro-F1` | F1 moyenné par classe (vs micro-F1 qui pondère par nombre d'exemples) ; meilleur pour datasets déséquilibrés |
| `comparator` | nouveau champ optionnel dans SLO_DEFINITIONS ; `lte` pour SLO « plus bas = mieux » (latence), `gte` pour « plus haut = mieux » (accuracy) |
| `eval-accuracy-bench.json` | format intermédiaire produit par `run_eval.py --output-bench-format` ; consommé par `parse_criterion_report.py` |

## Annexe D — Checklist d'exécution architect → impl

```
□ D6.1.1  Re-run test pratique avec RUST_LOG=trace
□ D6.1.2  Vérifier FusionStats.round (pas un faux 0)
□ D6.1.3  Confirmer consecutive_high par round
□ D6.1.4  diagnostic consolidé dans docs/audits/d6_accuracy_validation.md

□ D6.2.1  ControllerConfig.min_rounds field
□ D6.2.2  Loop downgrade Converged → Continue
□ D6.2.3  Validation min_rounds <= max_rounds
□ D6.2.4  3 unit tests min_rounds

□ D6.3.1  Mode1Config::default() 6 seuils mis à jour
□ D6.3.2  Comment header + rationale
□ D6.3.3  demo_classify_isolated.py rounds >= 4 + variance conf

□ D6.4.1  run_eval.py sur golden support_triage
□ D6.4.2  accuracy >= 0.75, macro_f1 >= 0.70
□ D6.4.3  Si KO : escalader H3 / D7
□ D6.4.4  docs/audits/d6_accuracy_validation.md

□ D6.5.1  --output-bench-format X.json
□ D6.5.2  WANTED_PREFIXES + SLO_DEFINITIONS
□ D6.5.3  evaluate_slos() comparator gte/lte
□ D6.5.4  bench_runner.sh RUN_ACCURACY=1
□ D6.5.5  bench.yml job bench-accuracy
□ D6.5.6  BENCHMARKS.md § 8

□ D6.6.1  Re-run bench complet
□ D6.6.2  bench_baselines/v0.1.6/baseline.json
□ D6.6.3  _note + post-D6 documentation

□ Validation finale
  □ cargo test --workspace (full)
  □ cargo clippy --workspace --benches --tests -D warnings
  □ cargo bench --workspace -- --quick
  □ python eval/scripts/run_eval.py → 0 (pass gates)
  □ Test pratique : 8/8 ou 7/8 correct (vs 4/8 pré-D6)
```

---

**Fin de l'architecture D6.** Le sprinter peut implémenter en
suivant Annexe D, en revenant à §4 pour le détail du fix et §10
pour les SLO targets exactes.

> **Approbation requise avant implémentation** :
> - [ ] Architecte performance (revue technique du fix).
> - [ ] Owner `efe_core::swarm` (revue contrat ControllerConfig).
> - [ ] Owner `efe_server::mode1` (revue calibration consensus).
> - [ ] Lead infra (revue intégration CI bench-accuracy).
