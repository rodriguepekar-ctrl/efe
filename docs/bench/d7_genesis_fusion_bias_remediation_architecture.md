# Sprint D7 — Architecture : Genesis/Fusion bias remediation (H3 fix)

> **Statut** : architecture détaillée, prête à implémenter.
> **Phase** : ultra-thinking. Précision chirurgicale suprême.
> **Position** : suit D6 (consensus pacing fix + accuracy SLO infra).
> D7 est **le sprint qui débloque la valeur produit** : faire passer
> `SLO-D6-ACC-1` du rouge (0.25 mesuré, 0.75 cible) au vert.
> **Pré-requis** : avoir lu
> [`d6_consensus_fix_accuracy_slo_architecture.md`](d6_consensus_fix_accuracy_slo_architecture.md)
> et l'audit
> [`docs/audits/d6_accuracy_validation.md`](../audits/d6_accuracy_validation.md)
> qui établit l'évidence H3 (biais structurel genesis/fusion).

---

## Table des matières

1. [Contexte & mission](#1-contexte--mission)
2. [Diagnostic numérique : où le signal d'input se perd](#2-diagnostic-numérique--où-le-signal-dinput-se-perd)
3. [Cinq sous-hypothèses H3a–H3e classées par évidence](#3-cinq-sous-hypothèses-h3ah3e-classées-par-évidence)
4. [Cinq approches de fix avec arbitrage](#4-cinq-approches-de-fix-avec-arbitrage)
5. [Approche recommandée : β/α tuning + fallback hybride](#5-approche-recommandée--βα-tuning--fallback-hybride)
6. [Architecture du fix : Layer 1 (β) + Layer 2 (α)](#6-architecture-du-fix--layer-1-β--layer-2-α)
7. [Calibration loop : sweep automatisé](#7-calibration-loop--sweep-automatisé)
8. [Multi-catalog validation (anti-overfit)](#8-multi-catalog-validation-anti-overfit)
9. [12 décisions de conception clés](#9-12-décisions-de-conception-clés)
10. [10 risques identifiés et mitigations](#10-10-risques-identifiés-et-mitigations)
11. [Plan jour-par-jour (3-4 j wallclock)](#11-plan-jour-par-jour-3-4-j-wallclock)
12. [Calibration & SLO post-D7](#12-calibration--slo-post-d7)
13. [Anti-régression & realism check](#13-anti-régression--realism-check)
14. [Intégration CI : switch fail-closed](#14-intégration-ci--switch-fail-closed)
15. [Critères d'acceptation (DoD)](#15-critères-dacceptation-dod)
16. [Out of scope & roadmap follow-ups](#16-out-of-scope--roadmap-follow-ups)

---

## 1. Contexte & mission

### 1.1 État post-D6

Sprint D6 a livré 4 layers de fix (controller `min_rounds`, hardened
consensus thresholds, accuracy SLO infrastructure, telemetry round
display) et a **prouvé mécaniquement** que la fusion s'exécute
maintenant 7 rounds en moyenne (vs 0 visible pré-D6). Confidence
varie 0.695-0.713 par input (vs 0.575 constant pré-D6) — la pipeline
**fonctionne**.

**Et pourtant l'accuracy a régressé** :

| Mesure | Pré-D6 | Post-D6 |
|--------|:------:|:-------:|
| Top-1 accuracy (golden 40 cas) | 4/8 ≈ 0.50* | **10/40 = 0.25** |
| Macro-F1 | ~0.40* | **0.10** |
| Confusion : tous prédits → `billing` | 4/8 cases | **40/40 cases** |
| Confidence variance | 0 (constant) | ~0.007 (varies) |

*Le pré-D6 était mesuré sur 9 cas hand-curated, post-D6 sur les 40
golden. Le pré-D6 sur 40 aurait été ~13/40 par symétrie.

**L'embedder seul** (cosine nearest sur les anchors, sans aucune
fusion) atteindrait 8/8 sur le hand-curated set et probablement
35-38/40 sur le golden. **L'écart entre embedder seul (95 %+) et
EFE Mode 1 actuel (25 %) est ce que D7 doit récupérer.**

### 1.2 Mission D7 (énoncé chirurgical)

> 1. **Mesurer chiffré** où le signal d'input se perd dans la
>    pipeline fusion : par round, calculer le cosine du centroïde
>    actuel vs (a) le problem_embedding initial, (b) chaque anchor
>    de la cone d'alignement.
>
> 2. **Identifier le coupable principal** parmi les 5 sous-hypothèses
>    H3a-H3e (β trop faible, α trop fort, templates aveugles à
>    l'input, attention saturée, projection trop agressive).
>
> 3. **Calibrer empiriquement** les coefficients β₀/λ_β et
>    α₀/floor de `Mode1Config.fusion` via un sweep automatisé sur
>    le golden set, jusqu'à atteindre `accuracy_top1 >= 0.75`.
>
> 4. **Valider sur 2 catalogs additionnels** (`fraud_alert` et
>    `billing_escalation`, déjà présents dans
>    `clients/python/efe_tool/scenarios/builtin/`) pour éviter
>    l'overfit sur `support_triage`.
>
> 5. **Switcher le job CI `bench-accuracy` à fail-closed** une fois
>    `accuracy_top1 >= 0.75` stable sur 3 catalogs. Toute régression
>    future bloque le merge.
>
> 6. **Plan B (fallback) si l'approche β/α tuning ne dépasse pas
>    65 %** : l'approche hybride (raw cosine + swarm pour
>    confidence) est documentée et code prêt en `D7.6` — elle
>    récupère 95 %+ d'accuracy en sacrifiant l'« essaim émergent »
>    comme pure source de vérité de classification.

### 1.3 Localisation du fix

| Fichier | Modification |
|---------|--------------|
| `crates/efe-core/src/fusion/config.rs` | Aucune (les defaults libres restent inchangés pour backward compat) |
| `crates/efe-server/src/mode1.rs::Mode1Config::default()` | `fusion: FusionConfig::default()` → `fusion: build_mode1_fusion_config()` (helper local avec β/α tunés) |
| `eval/scripts/calibrate_mode1.py` | **Nouveau** — sweep loop (β₀, λ_β, α₀, floor) sur golden set |
| `eval/golden/fraud_alert.jsonl` | **Nouveau** — golden ~30 cas fraude/non-fraude (extrait des scenarios) |
| `eval/golden/billing_escalation.jsonl` | **Nouveau** — golden ~30 cas billing |
| `eval/scripts/run_eval.py` | Accepter `--multi-catalog` flag pour itérer sur N catalogs |
| `scripts/parse_criterion_report.py::SLO_DEFINITIONS` | Tighter targets D6-ACC-1..3 si D7 livre > 0.85 |
| `.github/workflows/bench.yml` | `continue-on-error: true` → `false` sur job `bench-accuracy` |
| `bench_baselines/v0.1.7/baseline.json` | placeholder + post-merge real freeze |
| `clients/python/efe_tool/integrations/hybrid_decoder.py` | **Optionnel** — Plan B fallback si calibration ne suffit pas |

---

## 2. Diagnostic numérique : où le signal d'input se perd

### 2.1 La β-decay est exponentielle et féroce

`FusionConfig::default()` (cf. `fusion/config.rs:60-79`) :

```text
problem_anchor_init     = 0.20    # β₀
problem_anchor_decay    = 0.15    # λ_β (exponential decay rate)
alignment_strength_init = 0.50    # α₀
alignment_strength_floor = 0.05   # α floor
max_rounds              = 16
```

`beta_for_round(cfg, r) = β₀ · exp(-λ_β · r)`

| Round | β | Coût input dans β-mix |
|------:|--:|----------------------:|
| 0 | 0.200 | 20 % du latent absorbe l'input |
| 1 | 0.172 | 17.2 % |
| 2 | 0.148 | 14.8 % |
| 3 | 0.128 | 12.8 % |
| **4** (D6 floor) | **0.110** | **11 %** |
| 5 | 0.094 | 9.4 % |
| 6 | 0.081 | 8.1 % |
| **7** (D6 typical convergence) | **0.070** | **7 %** |
| 8 | 0.060 | 6 % |
| 16 | 0.018 | 1.8 % |

**Au round 7, l'input ne pèse plus que 7 % dans la β-mix**. Les
93 % restants viennent du round précédent — qui lui-même avait
intégré 8 % d'input au round 6 — et ainsi de suite. Effet
multiplicatif : la mémoire de l'input décroît exponentiellement.

### 2.2 La α-projection compose le problème

`alpha_for_round(cfg, r) = max(floor, α₀ · (1 - r / R_max))`

| Round | α | Force de projection vers convex hull anchors |
|------:|--:|---------------------------------------------:|
| 0 | 0.500 | 50 % vers convex hull |
| 4 | 0.375 | 37.5 % |
| 7 | 0.281 | 28.1 % |
| 8 | 0.250 | 25 % |
| 16 | 0.050 (floor) | 5 % |

**Au round 7, α = 0.28** : à chaque sweep, les latents sont tirés
de 28 % vers le convex hull center. **En 7 rounds, le tirage
cumulé** (sans correction par β qui s'évapore en parallèle)
**~ 1 - (1 - 0.28)⁷ = 88 %** du chemin vers la centroïde des
anchors. Combiné à la β-decay, le centroïde post-fusion est
quasi-indépendant de l'input.

### 2.3 Le compromis math : « émancipation vs ancrage »

La doctrine implicite dans le schedule actuel :
* **β** doit décroître : le swarm doit s'« émanciper » de l'input
  pour explorer l'espace consensus. *Hypothèse architecturale.*
* **α** doit décroître aussi : la cone doit relâcher la prise pour
  que les agents trouvent leur propre cluster. *Mais c'est un vœu
  pieux* — α ne décroît qu'après round 8 (la moitié de
  max_rounds), donc 7 rounds × 0.28-0.50 de tirage cone à 0.05
  floor pas atteint.

**Pour Mode 1 (classification par anchors), cette doctrine est
inversée par rapport au besoin** :
* L'input EST la signature de classe — il doit RESTER dominant tout
  au long.
* La cone EST la cible — la projection doit être *forte mais
  équilibrée*.

D7 retune les schedules pour **Mode 1 spécifiquement**, sans toucher
les defaults libraries (qui restent valides pour des modes futurs où
l'« émancipation » serait souhaitable).

---

## 3. Cinq sous-hypothèses H3a–H3e classées par évidence

### H3a — β décline trop vite (priorité : **TRÈS HAUTE, P=85 %**)

Le calcul §2.1 montre β=0.07 au round 7. **Évidence numérique
directe**. Le fix : β₀ ≥ 0.40 et/ou λ_β ≤ 0.05.

**Test diagnostic** : forcer β = 0.30 constant, mesurer accuracy.

### H3b — α tire trop fort vers le convex hull (P=75 %)

§2.2 montre α=0.28 au round 7 (encore haut). Combiné à H3a, le
centroid est attiré vers le centre indépendamment de l'input.

**Test diagnostic** : forcer α = 0.10 constant, mesurer accuracy.

### H3c — Templates aveugles à l'input (P=50 %)

`StubTemplateRegistry::mvp_defaults(dim)` produit 3 templates fixes
(NaSem, NaLog, NaCrit) qui ne dépendent pas de l'input. Le genesis
spawne 1024 agents avec des latents dérivés de ces templates →
**état initial constant pour toutes les requêtes**.

**Test diagnostic** : tracer la cosine du centroïde post-genesis
(round 0) vs problem_embedding pour 4 inputs distincts. Si tous
sont quasi-identiques → templates dominent.

### H3d — Attention saturée vers une dimension dominante (P=30 %)

L'attention `compute_scores(latent, mask, scores, params)` calcule
des scores `(h_i · h_j) / sqrt(d) / temperature`. Si les latents
post-genesis ont une dimension dominante (par effet du
`HeuristicMetaAnalyzer`), l'attention focalise dessus → tous les
agents convergent sur le même cluster.

**Test diagnostic** : mesurer la variance de l'attention à round 1
vs round 7. Si elle s'effondre rapidement → saturation.

### H3e — Le clip_max=32 n'est pas le frein (P=10 %)

`clip_max` cap les valeurs absolues. À 32, le clip ne déclenche
quasiment jamais sur des latents normalement bornés. Probabilité
faible, listée pour exhaustivité.

### Stratégie : adresser H3a + H3b ensemble

H3a et H3b sont **multiplicatifs** : β faible amplifie l'effet de
α haut, et inversement. Les fixer séparément ne suffit pas. D7
retune les deux en tandem.

H3c est secondaire : si H3a+H3b sont fixés et l'accuracy reste sous
70 %, on attaque H3c. Sinon, H3c reste un "follow-up D8".

---

## 4. Cinq approches de fix avec arbitrage

| Approche | Description | Pro | Con | Effort |
|---------:|------------|-----|-----|-------:|
| **A** | Augmenter β (H3a fix isolé) | Localisé, retest rapide | Insuffisant seul (H3b reste) | 0.5 j |
| **B** | Diminuer α (H3b fix isolé) | Localisé | Insuffisant seul (H3a reste) | 0.5 j |
| **C** | Hybride decoder (cosine raw + swarm seulement pour confidence) | 95 %+ accuracy garanti, embedder seul est déjà 95 %+ | Sacrifie le rôle "classification par essaim émergent" — l'EFE devient un wrapper sur cosine search | 2 j |
| **D** | Per-anchor genesis spawn (initialiser les agents par anchor) | Adresse H3c directement | Refonte template_registry + spawner | 4-5 j |
| **E** | **Tandem β + α tune via sweep empirique sur Mode1Config.fusion** | Adresse H3a + H3b ensemble, ne touche pas le code libraire (juste defaults Mode 1) | Calibration peut overfit support_triage → mitigé par multi-catalog | 2 j |

### Choix : Approche E (principale) + C (fallback)

* **Layer 1 — Mode1Config.fusion override** : β₀ haut + λ_β faible
  + α₀ modéré + floor maintenu. Sweep automatisé pour trouver
  `(β*, α*)` qui maximise macro-F1 sur 3 catalogs.
* **Layer 2 — Validation multi-catalog** : exiger ≥ 0.75 sur les 3
  golden (support_triage, fraud_alert, billing_escalation).
* **Plan B documenté** (`hybrid_decoder.py` prêt à activer via
  `Mode1Config.decoder_mode = HybridCosine`) si E plafonne sous
  0.65.

**Pourquoi pas D directement** : 4-5 jours d'effort, refonte
template_registry, risque de casser plus de tests. À garder pour
D8 si E ne livre pas et C est insatisfaisant.

---

## 5. Approche recommandée : β/α tuning + fallback hybride

### 5.1 Architecture finale du fix

```text
┌──────────────────────────────────────────────────────────┐
│ Mode1Config::default()                                   │
│ ─────────────────────                                    │
│   dim_latent: 384                                        │
│   consensus: <D6-hardened thresholds>                    │
│   controller: min_rounds=4 (D6)                          │
│   fusion: build_mode1_fusion_config()  ← D7 NEW          │
└──────────────────────────────────────────────────────────┘
                           │
                           ▼
┌──────────────────────────────────────────────────────────┐
│ build_mode1_fusion_config() (Sprint D7)                  │
│ ─────────────────────────────────────                    │
│   max_rounds: 16  (unchanged)                            │
│   problem_anchor_init: 0.45     ← was 0.20 (D7 layer 1)  │
│   problem_anchor_decay: 0.05    ← was 0.15 (D7 layer 1)  │
│   alignment_strength_init: 0.30 ← was 0.50 (D7 layer 2)  │
│   alignment_strength_floor: 0.05 (unchanged)             │
│   temperature: 1.0  (unchanged)                          │
│   ... <other fields unchanged>                           │
└──────────────────────────────────────────────────────────┘
```

### 5.2 Calcul du nouveau profil β/α

Avec les nouveaux defaults :

| Round | β (D7) | β (pré-D7) | α (D7) | α (pré-D7) |
|------:|-------:|-----------:|-------:|-----------:|
| 0 | 0.450 | 0.200 | 0.300 | 0.500 |
| 4 | 0.368 | 0.110 | 0.225 | 0.375 |
| 7 | 0.317 | 0.070 | 0.169 | 0.281 |
| 8 | 0.301 | 0.060 | 0.150 | 0.250 |
| 16 | 0.202 | 0.018 | 0.050 | 0.050 |

**Round 7 : β = 0.317** (vs 0.070 pré-D7) — **× 4.5 plus de poids
input**. **α = 0.169** (vs 0.281) — **× 1.7 moins de tirage cone**.

L'effet combiné : à chaque round, le centroïde absorbe 32 % d'input
et est tiré de 17 % vers la cone — équilibre où l'input reste
dominant, la cone agit comme régulateur.

### 5.3 Ces valeurs sont des **points de départ analytiques**

Le sweep empirique (D7.3) explore une grille `(β₀ ∈ {0.30, 0.40,
0.45, 0.50, 0.60}) × (λ_β ∈ {0.02, 0.05, 0.08, 0.12}) ×
(α₀ ∈ {0.20, 0.25, 0.30, 0.40})` → 80 combinations à tester sur
40 cas chacune ⇒ 3200 RPCs. À ~2.5 s/RPC sur M3 Pro ⇒ **2.2 h
calibration**. Réductible via :
* (a) sub-sample golden à 16 cas (4/classe) pour 1ʳᵉ passe ⇒ ~50 min
* (b) sub-sample grille à 20 combinations (early stopping) ⇒ ~15 min
* (c) parallélisation tokio ⇒ ~5 min sur M3 Pro

D7.3 utilise (a) + (c). Le résultat est validé sur le golden
complet à la fin.

---

## 6. Architecture du fix : Layer 1 (β) + Layer 2 (α)

### 6.1 Pourquoi un helper local (vs muter `FusionConfig::default()`)

`FusionConfig::default()` est consommé par TOUS les benches/tests
qui construisent un fusion sans Mode1. Le muter casserait le
harnais (tests / D1 / D3 / mock benches).

**D7 introduit `mode1::build_mode1_fusion_config()`** — une fonction
locale qui produit un `FusionConfig` avec les overrides Mode 1
spécifiques. Mode1Config.fusion = build_mode1_fusion_config(). Le
reste du workspace est inchangé.

### 6.2 Code shape (sans implémentation)

```text
// crates/efe-server/src/mode1.rs

/// **Sprint D7** — Mode-1-specific FusionConfig with β/α tuned for
/// classification-by-anchors usage (vs the library default's
/// "emancipation" doctrine which deserts the input over rounds).
///
/// Calibrated empirically via `eval/scripts/calibrate_mode1.py`
/// on 3 golden catalogs. The values below are post-D7 final ;
/// see `bench_baselines/v0.1.7/baseline.json` for the achieved
/// accuracy.
fn build_mode1_fusion_config() -> FusionConfig {
    let mut cfg = FusionConfig::default();
    // D7 layer 1 — keep the input dominant across rounds.
    cfg.problem_anchor_init = 0.45;
    cfg.problem_anchor_decay = 0.05;
    // D7 layer 2 — relax the cone projection so the input has
    // room to discriminate before being pulled to convex hull.
    cfg.alignment_strength_init = 0.30;
    // Floor unchanged at 0.05 — alignment must remain non-zero
    // to guarantee the centroid stays in cone for the alignment
    // breach check.
    cfg
}

impl Default for Mode1Config {
    fn default() -> Self {
        // ... <D6 consensus + controller code unchanged>
        Self {
            dim_latent: 384,
            fusion: build_mode1_fusion_config(),  // ← D7 change
            consensus,
            controller,
            prealloc_arenas: 4,
            max_arenas: 8,
            n_streams: 4,
        }
    }
}
```

### 6.3 Tests anti-régression à pinner

```text
// In crates/efe-server/src/mode1.rs::tests

#[test]
fn d7_fusion_config_pins_tuned_schedules() {
    let f = build_mode1_fusion_config();
    assert!((f.problem_anchor_init  - 0.45).abs() < 1e-6);
    assert!((f.problem_anchor_decay - 0.05).abs() < 1e-6);
    assert!((f.alignment_strength_init  - 0.30).abs() < 1e-6);
    assert!((f.alignment_strength_floor - 0.05).abs() < 1e-6);
    assert_eq!(f.max_rounds, 16);
}

#[test]
fn d7_default_uses_tuned_fusion() {
    let c = Mode1Config::default();
    assert!((c.fusion.problem_anchor_init - 0.45).abs() < 1e-6);
}
```

---

## 7. Calibration loop : sweep automatisé

### 7.1 Architecture du calibrate script

**Fichier nouveau** : `eval/scripts/calibrate_mode1.py` (~300 LOC).

```text
INPUTS:
  --catalog catalogs/support_triage.yaml
  --golden eval/golden/support_triage.jsonl
  --grid β₀=[0.30,0.40,0.45,0.50,0.60] λ_β=[0.02,0.05,0.08,0.12]
        α₀=[0.20,0.25,0.30,0.40]
  --early-stop-after 5  # stop sub-grid if 5 consecutive worse
  --concurrency 4       # 4 servers in parallel on different ports
  --output target/calibration-d7.json

STEPS:
  1. Build a release efe-server binary.
  2. For each (β₀, λ_β, α₀) tuple :
       a. Spawn an efe-server with EFE_FUSION_β₀=…, EFE_FUSION_λ_β=…
          (requires injecting these as env vars — see §7.2)
       b. Run eval/scripts/run_eval.py against it on golden set.
       c. Record (accuracy, macro_f1, mean_conf_correct, latency).
       d. Kill server.
  3. Find the (β*, λ*, α*) that maximises macro_f1 (tie-break
     by mean_conf_correct).
  4. Emit the winning tuple + full grid CSV.

OUTPUT (calibration-d7.json):
  {
    "winner": {
      "problem_anchor_init": 0.45,
      "problem_anchor_decay": 0.05,
      "alignment_strength_init": 0.30,
      "macro_f1": 0.823,
      "accuracy_top1": 0.825,
      ...
    },
    "grid": [
      {"problem_anchor_init": 0.30, "problem_anchor_decay": 0.02, ..., "macro_f1": 0.71},
      ...
    ]
  }
```

### 7.2 Comment injecter β/α dans l'efe-server

Trois options :
* **Option A — env vars** : ajouter `EFE_FUSION_PROBLEM_ANCHOR_INIT`
  etc. à `ServerConfig::from_env()`. Pro : pas de rebuild par
  variant. Con : nouvelle surface ServerConfig à maintenir.
* **Option B — multi-build** : compiler N binaires avec différents
  hardcoded values via `cfg!(feature = "calibration_X")`. Pro :
  pas de surface env. Con : 80 builds × 25 s = 33 min juste pour
  build.
* **Option C — runtime override via CLI flag** : `efe-server
  --fusion-beta-init 0.45 ...`. Pro : flexible. Con : nouvelle
  surface CLI.

**Décision** : **Option A** pour D7. Les env vars sont la convention
serveur déjà (cf. `EFE_LISTEN_ADDR`, `EFE_AUTH_ENABLED`). On ajoute
4 vars : `EFE_FUSION_BETA_INIT`, `EFE_FUSION_BETA_DECAY`,
`EFE_FUSION_ALPHA_INIT`, `EFE_FUSION_ALPHA_FLOOR`. Si absentes,
defaults D7 s'appliquent.

### 7.3 Heuristique d'early-stopping

Pour réduire les 80 → ~25 combinations effectives :
* **Sub-sample golden à 16 cas** (4 par classe support_triage).
* **Greedy axis** : tester d'abord β₀=0.45, λ_β=0.05 (les
  analytiques §5.2). Si macro_f1 > 0.65, sweeper finement autour.
  Si < 0.50, élargir.
* **Skip combinations dominées** : si β₀=0.45 / λ_β=0.05 est
  pire que β₀=0.40 / λ_β=0.05 pour α=0.30, alors β₀=0.45 /
  λ_β=0.05 / α=0.20 est à fortiori dominé.

---

## 8. Multi-catalog validation (anti-overfit)

### 8.1 Pourquoi 3 catalogs

Calibrer (β*, λ*, α*) UNIQUEMENT sur `support_triage` produit un
**risque d'overfit** : les valeurs trouvées pourraient marcher
pour les anchors fraud/bug/question/billing mais échouer sur un
autre catalog (différent embedder, différentes anchors, etc.).

**D7 valide** sur 3 catalogs, exigeant que **les 3 atteignent
≥ 0.75 accuracy_top1** :

* `support_triage` (4 classes, multilingue EN+FR, 40 cas) — déjà
  présent.
* `fraud_alert` — extrait des scenarios builtin (~30 cas, 2-3
  classes) — **D7.4 nouveau**.
* `billing_escalation` — extrait des scenarios builtin (~30 cas,
  2-3 classes) — **D7.4 nouveau**.

### 8.2 Construction des golden additionnels

Les scenarios builtin
(`clients/python/efe_tool/scenarios/builtin/{fraud_alert,
billing_escalation, support_triage_bilingual}/`) contiennent déjà
des données labellisées. D7.4 :

1. Lit les fichiers `.yaml` / `.jsonl` des scenarios.
2. Convertit en format JSONL golden compatible
   (`{id, catalog, text, expected, language}`).
3. Émet vers `eval/golden/{fraud_alert,billing_escalation}.jsonl`.

### 8.3 Aggregate accuracy : critère de validation

`run_eval.py --multi-catalog` :
* Itère sur N catalogs.
* Pour chaque, compute accuracy + macro_f1.
* Émet : per-catalog metrics + aggregate (mean across catalogs).

**Gate D7** : aggregate macro_f1 ≥ 0.75 ET min(catalogs) ≥ 0.65.

---

## 9. 12 décisions de conception clés

| # | Décision | Alternative rejetée | Rationale |
|---|----------|---------------------|-----------|
| D1 | Approche E (β/α tune Mode 1 specific) | A/B isolés ; D refonte template | E adresse H3a+H3b en tandem sans casser libraires |
| D2 | Helper `build_mode1_fusion_config()` (vs muter library default) | muter `FusionConfig::default()` | bench/test legacy reposent sur les library defaults ; ne PAS les casser |
| D3 | β₀ = 0.45, λ_β = 0.05 (analytique) | 0.30 / 0.10 (modéré), 0.60 / 0.02 (extrême) | sweep §7 affinera ; ces points de départ donnent β=0.32 à round 7 — input reste dominant |
| D4 | α₀ = 0.30 (analytique) | 0.10 (très bas), 0.50 (legacy) | équilibre input/cone — laisse la cone réguler sans dominer |
| D5 | Env vars EFE_FUSION_* (Option A) pour calibration | CLI flag, multi-build | convention serveur existante ; pas de nouveau code de parsing |
| D6 | Multi-catalog validation OBLIGATOIRE (3 catalogs) | calibration sur support_triage seul | anti-overfit ; toute future PR qui touche fusion teste 3 catalogs |
| D7 | Sub-sample golden à 16 cas pour calibration grid | 40 cas full sweep | 3× speedup sans perte significative (4 cas/classe est statistiquement décent) |
| D8 | macro-F1 comme métrique d'optimisation (pas accuracy) | accuracy top-1 | macro-F1 tient compte du déséquilibre per-class ; accuracy peut tromper sur classes minoritaires |
| D9 | Plan B `hybrid_decoder.py` documenté mais NON implémenté en D7 | implémenter aussi le fallback | YAGNI — n'introduire le hybrid que si E plafonne ; sinon il reste comme architecture documentée pour D8 |
| D10 | CI bench-accuracy switch fail-closed UNIQUEMENT après 3 runs main consécutifs PASS | switch immédiat post-merge D7 | prudence — laisser le temps de détecter une régression CI environment-specific |
| D11 | Tests anti-régression PIN les valeurs β/α calibrées | aucun test sur les valeurs (juste accuracy) | sans pin, une PR qui mute β à 0.20 sans le savoir casse l'accuracy → on veut un test rouge avant le bench-accuracy |
| D12 | Out-of-scope : H3c (templates aveugles) | l'attaquer aussi | si E livre, H3c devient inutile ; si E plafonne, H3c devient D8 dédié |

---

## 10. 10 risques identifiés et mitigations

### R1 — Le sweep ne trouve pas (β*, α*) ≥ 0.75

**Impact** : D7 livre une amélioration partielle (ex 0.55) mais
n'atteint pas le gate. Le SLO `D6-ACC-1` reste FAIL en CI.

**Mitigation** :
* Plan B documenté §4 (hybrid decoder) prêt à activer en D7.6.
* Si plan B aussi insuffisant : escalade D8 (refonte templates).
* En tout état de cause, D7 livre une amélioration mesurable —
  le SLO bouge de 0.25 vers une cible non-zéro.

### R2 — Overfit à `support_triage`

**Impact** : (β*, α*) trouvés overfit. Sur fraud_alert / billing,
accuracy < 0.65.

**Mitigation** :
* Multi-catalog validation OBLIGATOIRE (§8) : gate aggregate.
* Si overfit observé, sweep sur les 3 catalogs SIMULTANÉMENT
  (slower mais robust) au lieu de calibrer sur 1 puis valider.

### R3 — Le sweep prend trop de temps en CI

**Impact** : 2.2 h ne tient pas dans CI 30 min budget.

**Mitigation** :
* Calibration n'EST PAS lancé par CI à chaque PR — c'est un
  one-shot dev local pour fixer les defaults.
* Le résultat (β*, α*) est hardcodé dans
  `build_mode1_fusion_config()`.
* CI vérifie juste l'accuracy avec ces defaults via `run_eval.py`
  (40 cas × 2.5 s = ~2 min).

### R4 — Numerical instability avec β élevé

**Impact** : β=0.45 fait que 45 % de l'input se mixe par round.
Si l'input embedding a une norme > 1, les latents peuvent croître
sans borne → numerical_explosion.

**Mitigation** :
* `clip_max=32` est le frein. Les v2 fixtures embedder normalize
  L2 → norme = 1 ⇒ pas de risque d'explosion.
* Tests proptest dans `crates/efe-core/tests/swarm_properties.rs`
  vérifient finite outputs sur N rounds — ils tournent à chaque
  PR.

### R5 — α=0.30 plus faible casse l'alignment_breach detection

**Impact** : si la cone devient « molle », des centroïdes hors-cone
ne sont plus correctement projetés → alignment_breach errors.

**Mitigation** :
* `alignment_strength_floor = 0.05` (inchangé) garantit α ≥ 0.05
  toujours.
* `cone_policy.max_projection_distance` du catalog est la limite
  hard ; si breach, le serveur retourne `FAILED_PRECONDITION`,
  pas un mauvais classement.
* Test pratique D6 a montré que l'adversarial reject (alignment
  breach for "weather") fonctionne — on vérifie en D7 que ça
  reste vrai.

### R6 — `fraud_alert` / `billing_escalation` golden inexistants

**Impact** : §8 dépend de l'extraction depuis les scenarios. Si
ces scenarios ne fournissent pas de données labellisées suffisantes,
multi-catalog validation est sub-optimale.

**Mitigation** :
* Inspection préliminaire des
  `clients/python/efe_tool/scenarios/builtin/*/` (D7.0).
* Si insuffisant : générer 30 cas synthétiques par catalog via
  Claude API (existing `bridges/claude.py`) — coût ~$0.50, 1 jour.
* En dernier recours : valider sur support_triage uniquement avec
  un disclaimer explicite dans `bench_baselines/v0.1.7/_note`.

### R7 — Switch CI fail-closed casse les PRs en flight

**Impact** : merger D7 déclenche un fail-closed sur toute PR
ouverte qui ne touche pas la fusion.

**Mitigation** :
* D10 décision : switch fail-closed APRÈS 3 main runs PASS.
* `continue-on-error: true` reste en place pendant la calibration
  D7 ; flippé manuellement par PR séparé après stabilisation.

### R8 — Les env vars EFE_FUSION_* polluent la surface ServerConfig

**Impact** : si elles restent en prod, elles permettent à un
opérateur mal informé de re-loosen la calibration.

**Mitigation** :
* Marquer les env vars **DEV ONLY** dans `config.rs` doc.
* Optionnel : guard via `cfg!(debug_assertions)` ou feature flag
  `calibration` — les env vars ne sont lues qu'en dev.
* Documenté : `EFE_FUSION_*` n'apparaît PAS dans la doc Helm /
  k8s prod.

### R9 — Le sweep ne trouve qu'un local optimum

**Impact** : la grille (5 × 4 × 4) peut louper un optimum global.

**Mitigation** :
* Grille analytiquement choisie autour du raisonnement §5.
* Si winner est en bord de grille, élargir et re-sweeper.
* Optionnel : Bayesian optimization (scikit-optimize) au lieu de
  grid — D7.7 polish si on a le temps.

### R10 — Calibration nécessite efe-server multiple instances

**Impact** : 4 instances en parallèle = 4 × 1 GiB pool = 4 GiB
RAM. CI ubuntu-latest = 7 GB. Tight.

**Mitigation** :
* Calibration tourne en LOCAL (M3 Pro 36 GB), pas en CI.
* CI verify single-config accuracy.
* Si calibration en CI un jour : limiter à 2 parallel.

---

## 11. Plan jour-par-jour (3-4 j wallclock)

### D7.0 (½ j) — Préparation

* Inspection des scenarios builtin pour multi-catalog. Identifier
  ce qui est exploitable.
* Si insuffisant : générer fraud_alert.jsonl + billing_escalation.jsonl
  via script.
* Confirmer : 3 catalogs disponibles ⇒ poursuivre. Sinon : R6
  fallback.

### D7.1 (½ j) — Instrumentation diagnostic

* Ajouter `EFE_LOG_CENTROID_TRACE=1` env var qui active un
  `tracing::debug!` après chaque round dans `mode1.rs::FusionAdapter`.
* Le log émet le cosine du centroïde courant vs (a) problem_embedding,
  (b) chaque anchor — vector de N_anchors+1 nombres.
* Utiliser pour confirmer chiffré : « le signal d'input se perd
  entre round 0 et round 7 ».

### D7.2 (½ j) — Env vars EFE_FUSION_*

* Ajouter à `ServerConfig::from_env()` les 4 var (β₀, λ_β, α₀,
  floor).
* Si présentes, les appliquer dans `Mode1Config::for_dim` /
  `default()`.
* Test : 1 RPC avec env var ⇒ verify behavior change.

### D7.3 (1 j) — Calibration loop

* Implémenter `eval/scripts/calibrate_mode1.py` (§7).
* Sub-grid initial (analytique §5) puis full sweep si nécessaire.
* Sortie : `target/calibration-d7.json` avec winner et grid CSV.

### D7.4 (½ j) — Multi-catalog golden

* Inspecter scenarios builtin.
* Convertir → `eval/golden/fraud_alert.jsonl` et
  `eval/golden/billing_escalation.jsonl`.
* Sanity-check : embedder fingerprint match avec catalog file.

### D7.5 (½ j) — Application du fix

* Hardcoder le winner dans `build_mode1_fusion_config()`.
* Ajouter les 2 tests anti-régression (§6.3).
* `cargo test --workspace` : tout PASS.
* Re-run `eval/scripts/run_eval.py` sur les 3 catalogs : vérifier
  ≥ 0.75 sur chacun.

### D7.6 (½ j) — Plan B (hybrid_decoder) — ARMÉ mais NON activé

* Coder `clients/python/efe_tool/integrations/hybrid_decoder.py` :
  un `HybridClassifier` qui prend (centroid_from_swarm,
  catalog_anchors) et retourne `(label, confidence)` via cosine
  raw + swarm confidence as tie-breaker.
* Tester en isolation : raw cosine atteint > 0.95 sur les 3
  golden.
* DOCUMENTÉ comme « plan B armé ; activé via
  `Mode1Config.decoder_mode = HybridCosine` SI E ne suffit pas ».
* Aucun changement à `Mode1Config::default()` en D7 — D8 ferait
  ce switch si nécessaire.

### D7.7 (½ j) — Tooling + baseline + CI

* `parse_criterion_report.py` : tighter SLO targets D6-ACC-1..3
  si winner > 0.85 (ex 0.85 → SLO target 0.80, marge 5 points).
* `bench_baselines/v0.1.7/baseline.json` : freeze winner +
  achieved metrics.
* `BENCHMARKS.md` : section § 9 « D7 — Genesis/Fusion bias
  remediation ».
* `docs/audits/d7_calibration_report.md` : rapport complet du
  sweep.

### D7.8 (¼ j) — Switch CI fail-closed

* Après 3 main runs PASS d'affilée (post-merge D7).
* PR séparée qui flippe `continue-on-error: true` → `false` sur
  job `bench-accuracy`.
* À partir de ce moment : toute régression accuracy bloque le
  merge.

---

## 12. Calibration & SLO post-D7

### 12.1 Cibles attendues

Si D7 livre comme prévu :

| ID | Bench key | Pré-D7 | Post-D7 attendu | SLO target post-D7 |
|----|-----------|-------:|----------------:|-------------------:|
| `SLO-D6-ACC-1` | top1 | 0.25 (FAIL) | **0.85+** | ≥ 0.80 (gate tightened) |
| `SLO-D6-ACC-2` | macro_f1 | 0.10 (FAIL) | **0.80+** | ≥ 0.75 (gate tightened) |
| `SLO-D6-ACC-3` | mean_conf_correct | 0.71 (PASS) | 0.75+ | ≥ 0.70 |
| `SLO-D6-ACC-4` | false_reject_rate | 0.00 (PASS) | < 0.10 | ≤ 0.20 (tightened) |
| `SLO-D6-LAT-1` | mean_latency | 2.47 s (PASS) | ~2.5 s | ≤ 3.0 s (unchanged) |

**Note** : la latence ne change pas significativement post-D7 — les
tweaks β/α n'ajoutent pas de rounds, les rounds count reste ~7.

### 12.2 Si winner < 0.85 mais ≥ 0.75

Conserver les SLO targets D6 inchangés (0.75 / 0.70 / 0.65 /
0.25). Documenter dans `_note` que D7 livre exactly the SLO sans
marge — toute régression future est immédiatement détectée.

### 12.3 Si winner < 0.75 (escalade)

Activer le plan B hybrid_decoder en D8. SLO targets restent. Le
gate continue de signaler le problème.

---

## 13. Anti-régression & realism check

**Fichier nouveau** :
`crates/efe-server/tests/d7_calibration_realism.rs`

### Test 1 — `d7_pinned_fusion_thresholds`

```text
let f = build_mode1_fusion_config();
assert!((f.problem_anchor_init - 0.45).abs() < 1e-6);
assert!((f.problem_anchor_decay - 0.05).abs() < 1e-6);
assert!((f.alignment_strength_init - 0.30).abs() < 1e-6);
```

Tout PR qui mute ces valeurs sans update de la calibration
expérimentale échoue.

### Test 2 — `d7_default_propagates_tuned_fusion`

```text
let c = Mode1Config::default();
assert!((c.fusion.problem_anchor_init - 0.45).abs() < 1e-6);
```

### Test 3 — `d7_for_dim_preserves_tuned_fusion`

```text
let c = Mode1Config::for_dim(768);
assert!((c.fusion.problem_anchor_init - 0.45).abs() < 1e-6);
```

### Test 4 — `d7_no_alignment_breach_on_golden_subset`

Test d'intégration léger (5 cas hand-curated) qui vérifie que la
nouvelle calibration ne déclenche PAS d'alignment_breach sur des
inputs valides (R5 mitigation).

---

## 14. Intégration CI : switch fail-closed

### 14.1 Pré-D7 (D6 era)

```yaml
bench-accuracy:
  ...
  continue-on-error: true   # <-- accepts FAIL silently
```

### 14.2 Post-D7 (transition)

```yaml
bench-accuracy:
  ...
  continue-on-error: true   # <-- KEEP for 3 main runs to validate stability
  # AFTER 3 consecutive PASS on main : open PR to remove this line
```

### 14.3 Post-D7 stable

```yaml
bench-accuracy:
  ...
  # continue-on-error removed — fail-closed
```

À ce point, `SLO-D6-ACC-1..3` sont des **gates dur** : toute PR
qui régresse accuracy > 5 % bloque le merge.

---

## 15. Critères d'acceptation (DoD)

### 15.1 Code & build

- [ ] `crates/efe-server/src/mode1.rs` : helper
      `build_mode1_fusion_config()` ajouté ; `Mode1Config::default()`
      utilise ce helper ; valeurs hardcodées sont les winner du
      sweep.
- [ ] `crates/efe-server/src/config.rs` : 4 env vars
      `EFE_FUSION_*` lues depuis env (DEV ONLY).
- [ ] `cargo build --workspace --release` clean.
- [ ] `cargo clippy --workspace --benches --tests -D warnings` clean.

### 15.2 Tests

- [ ] `crates/efe-server/tests/d7_calibration_realism.rs` créé,
      4 tests, tous PASS.
- [ ] `cargo test --workspace` (1 691+ tests + nouveaux), tous
      PASS.

### 15.3 Eval

- [ ] `eval/scripts/calibrate_mode1.py` créé et fonctionnel.
- [ ] `target/calibration-d7.json` produit avec winner.
- [ ] `eval/golden/fraud_alert.jsonl` ≥ 30 cas labellisés.
- [ ] `eval/golden/billing_escalation.jsonl` ≥ 30 cas labellisés.
- [ ] `run_eval.py --multi-catalog` fonctionne (3 catalogs).
- [ ] Aggregate macro_f1 ≥ 0.75.
- [ ] Min(per-catalog accuracy) ≥ 0.65.

### 15.4 Tooling

- [ ] SLO targets D6-ACC-1..3 tightened si winner > 0.85.
- [ ] `bench_baselines/v0.1.7/baseline.json` placeholder créé.
- [ ] (Post-merge stable) baseline v0.1.7 freezée avec valeurs
      réelles.

### 15.5 Documentation

- [ ] `BENCHMARKS.md` § 9 « D7 — Genesis/Fusion bias remediation »
      ajoutée.
- [ ] `docs/audits/d7_calibration_report.md` créé avec tableau
      complet du sweep + winner.
- [ ] `clients/python/efe_tool/integrations/hybrid_decoder.py` :
      plan B armé (NOT activé).

### 15.6 CI

- [ ] (Initial) job `bench-accuracy` reste `continue-on-error: true`
      pendant 3 main runs.
- [ ] (Post-validation) PR séparée flippe à fail-closed.

### 15.7 Validation finale

- [ ] `accuracy_top1` ≥ 0.75 sur les 3 golden.
- [ ] No `alignment_breach` sur les golden (regression test).
- [ ] Test pratique 9 cas (D6 carry-over) : 7/8 ou 8/8 correct
      (vs 2/8 pré-D7).

---

## 16. Out of scope & roadmap follow-ups

### 16.1 Hors-scope D7

* **H3c (templates aveugles à l'input)** — D8 si E plafonne.
* **H3d (attention saturation)** — investigation D8.
* **Bayesian optimization du sweep** — D7.7 polish.
* **CUDA backend** — Phase 2.
* **Multi-mode (Mode 2/3)** — quand ces modes existent.
* **Production deployment** — couvert par sprint séparé (cf.
  recommandation P2 du roadmap général).

### 16.2 Roadmap après D7

| Sprint | Cible | Justification |
|--------|-------|---------------|
| **D8** | Templates per-anchor genesis (H3c fix) | Si E plafonne sous 0.85 |
| **D9** | Hybrid decoder activation (plan B armé) | Alternative si D8 trop lourd |
| **D10** | Streaming RPC bench (`execute_streaming`) | Letter B follow-up |
| **D11** | Production deployment validation | Helm + k8s + chaos |

### 16.3 Métrique de succès post-D7

À 30 jours après merge D7 :

> *« Toute régression > 5 % sur l'accuracy top-1 sur l'un des 3
> catalogs golden produit un fail bench-accuracy CI dans les
> 24 h post-merge. Le bench accuracy est gated FAIL-CLOSED. La
> baseline v0.1.7 est freezée avec accuracy_top1 ≥ 0.85 et la
> calibration (β*, α*) est documentée comme reproductible via
> `eval/scripts/calibrate_mode1.py`. »*

Si cette affirmation tient, D7 a rempli sa mission — **EFE Mode 1
est un classificateur opérationnel, pas un placeholder bench-vert
fonctionnellement-rouge**.

---

## Annexe A — Trace centroid pseudocode

```text
// Within MvpCpuFusion::run_round_sync, after Phase 6 (clip) :
if std::env::var("EFE_LOG_CENTROID_TRACE").is_ok() {
    let centroid = compute_centroid(els, arena);
    let cos_problem = cosine(centroid, spec.problem_embedding);
    let cos_anchors: Vec<f32> = spec.alignment_cone.anchors
        .iter()
        .map(|a| cosine(centroid, a))
        .collect();
    tracing::debug!(
        round,
        cos_problem,
        cos_anchors = ?cos_anchors,
        "centroid trace"
    );
}
```

Activation : `EFE_LOG_CENTROID_TRACE=1` + `RUST_LOG=efe_server::mode1::trace=debug` (le `target:` du `tracing::debug!` est `efe_server::mode1::trace`, choisi pour permettre un filtrage fin sans inonder le log applicatif).

## Annexe B — Calibration sweep matrix (full)

| Param | Values | Count |
|-------|--------|------:|
| `problem_anchor_init` (β₀) | 0.30, 0.40, 0.45, 0.50, 0.60 | 5 |
| `problem_anchor_decay` (λ_β) | 0.02, 0.05, 0.08, 0.12 | 4 |
| `alignment_strength_init` (α₀) | 0.20, 0.25, 0.30, 0.40 | 4 |
| **Total** | — | **80** |

Sub-grid (early-stop) : si analytique (β₀=0.45, λ=0.05, α=0.30)
> 0.65 macro_f1, sweeper finement {0.40, 0.45, 0.50} × {0.03, 0.05,
0.07} × {0.25, 0.30, 0.35} = 27 combinations seulement.

## Annexe C — Glossary

| Terme | Définition |
|-------|------------|
| **β (problem_anchor)** | Coefficient de β-mix : poids du problem_embedding dans le latent du round suivant. Schedule = `β₀ · exp(-λ_β · r)`. |
| **α (alignment_strength)** | Coefficient de project_rows : force de tirage du latent vers la convex hull des anchors. Schedule = `max(floor, α₀ · (1 - r/R_max))`. |
| **β-decay** | Le phénomène par lequel l'input embedding « s'évapore » exponentiellement à mesure que les rounds progressent. |
| **anchor blending** | Effet d'attraction du centroïde post-projection vers le convex hull des anchors. Cf. D6 audit. |
| **multi-catalog validation** | Politique de calibration : exiger qu'un fix accuracy passe sur ≥ 3 catalogs distincts pour éviter l'overfit. |
| **fail-closed** | Configuration CI où un FAIL bloque le merge (vs `continue-on-error: true` qui le laisse passer). |
| **hybrid decoder** | Plan B : classify via cosine raw + swarm seulement pour confidence. Sacrifice l'« essaim émergent » pour 95 %+ accuracy. |

## Annexe D — Checklist d'exécution architect → impl

```
□ D7.0.1  Inspecter scenarios builtin (fraud_alert, billing_escalation)
□ D7.0.2  Décider source de données : extracts vs synthétique

□ D7.1.1  EFE_LOG_CENTROID_TRACE env var
□ D7.1.2  Trace tracing::debug dans FusionAdapter post-round
□ D7.1.3  Re-run test pratique avec trace, valider H3a + H3b chiffrés

□ D7.2.1  4 env vars EFE_FUSION_* dans ServerConfig::from_env
□ D7.2.2  Application via Mode1Config::for_dim
□ D7.2.3  Test : RPC change with env var

□ D7.3.1  eval/scripts/calibrate_mode1.py squelette
□ D7.3.2  Sub-grid analytique (3×3×3 = 27 points)
□ D7.3.3  Sweep + winner extraction
□ D7.3.4  Si winner < 0.65 : full grid (80 points)

□ D7.4.1  fraud_alert.jsonl golden (≥30 cas)
□ D7.4.2  billing_escalation.jsonl golden (≥30 cas)
□ D7.4.3  run_eval.py --multi-catalog flag

□ D7.5.1  build_mode1_fusion_config() helper
□ D7.5.2  Mode1Config::default() use it
□ D7.5.3  4 tests anti-régression
□ D7.5.4  cargo test all green
□ D7.5.5  Multi-catalog accuracy ≥ 0.75

□ D7.6.1  hybrid_decoder.py armé (NON activé)
□ D7.6.2  Test isolated : raw cosine ≥ 0.95
□ D7.6.3  Doc : « activable via Mode1Config.decoder_mode = HybridCosine »

□ D7.7.1  SLO targets tightened (si winner > 0.85)
□ D7.7.2  bench_baselines/v0.1.7/baseline.json
□ D7.7.3  BENCHMARKS.md § 9
□ D7.7.4  docs/audits/d7_calibration_report.md

□ D7.8.1  3 main runs PASS observation
□ D7.8.2  PR séparée : continue-on-error → false
□ D7.8.3  Confirm fail-closed gate active

□ Validation finale
  □ cargo test --workspace : all pass
  □ cargo clippy --workspace --benches --tests -D warnings : clean
  □ Multi-catalog : 3/3 ≥ 0.75
  □ Test pratique 9 cas : 7/8 ou 8/8
  □ Latence client p95 inchangée (~2.5 s)
  □ Aucun alignment_breach sur les golden
```

---

**Fin de l'architecture D7.** Le sprinter peut implémenter en
suivant Annexe D, en revenant à §6 pour le détail du fix et §7
pour l'algorithm de calibration.

> **Approbation requise avant implémentation** :
> - [ ] Architecte performance (revue technique du fix β/α).
> - [ ] Owner `efe_core::fusion` (revue impact sur kernels).
> - [ ] Owner `efe_server::mode1` (revue scope du Mode 1 specific
>       override).
> - [ ] Lead infra (revue bascule fail-closed CI).
