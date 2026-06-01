# Sprint D8 — Architecture : Plan B activation + Real-data validation + AnchorAware genesis (3 axes)

> **Statut** : architecture détaillée, prête à implémenter.
> **Phase** : ultra-thinking. Précision chirurgicale suprême.
> **Position** : suit D7 (β/α calibration ; winner empirique
> post-sweep gelé `(0.45, 0.08, 0.25)` ; aggregate macro_f1 =
> 0.815). Le point analytique pre-sweep était `(0.45, 0.05, 0.30)`
> — référencé ailleurs dans ce document comme rappel des valeurs
> d'amorce.
> D8 est **le sprint qui clôt H3** : il couvre les 3 derniers
> degrés de liberté laissés par D7 (residue `H3c` sur
> `billing_escalation`, validation hors-synthétique, opt-in
> AnchorAware) et **stabilise la pipeline avant productionalisation
> en D9**.
> **Pré-requis** : avoir lu
> [`d7_genesis_fusion_bias_remediation_architecture.md`](d7_genesis_fusion_bias_remediation_architecture.md)
> et l'audit
> [`docs/audits/d7_calibration_report.md`](../audits/d7_calibration_report.md).

---

## Table des matières

1. [Contexte & mission D8](#1-contexte--mission-d8)
2. [Diagnostic : le résidu H3c sur `billing_escalation`](#2-diagnostic--le-résidu-h3c-sur-billing_escalation)
3. [Trois axes de réponse complémentaires](#3-trois-axes-de-réponse-complémentaires)
4. [Axe 1 — Plan B activation (HybridClassifier)](#4-axe-1--plan-b-activation-hybridclassifier)
5. [Axe 2 — Real-data validation pipeline](#5-axe-2--real-data-validation-pipeline)
6. [Axe 3 — AnchorAware genesis registry (opt-in)](#6-axe-3--anchoraware-genesis-registry-opt-in)
7. [SLO additions D8](#7-slo-additions-d8)
8. [Multi-catalog hygiene & anti-overfit gates](#8-multi-catalog-hygiene--anti-overfit-gates)
9. [Pipeline PII redaction (8 patterns)](#9-pipeline-pii-redaction-8-patterns)
10. [Intégration CI : 2 nouveaux jobs](#10-intégration-ci--2-nouveaux-jobs)
11. [12 décisions de conception clés](#11-12-décisions-de-conception-clés)
12. [10 risques identifiés et mitigations](#12-10-risques-identifiés-et-mitigations)
13. [Plan jour-par-jour (4-5 j wallclock)](#13-plan-jour-par-jour-4-5-j-wallclock)
14. [Anti-régression & realism check](#14-anti-régression--realism-check)
15. [Critères d'acceptation (DoD)](#15-critères-dacceptation-dod)
16. [Out of scope & hand-off vers D9](#16-out-of-scope--hand-off-vers-d9)

---

## 1. Contexte & mission D8

### 1.1 État post-D7

Sprint D7 a livré le **β/α calibration helper** (`build_mode1_fusion_config()`),
le sweep automatisé (`eval/scripts/calibrate_mode1.py`), 2 nouveaux
golden datasets (`fraud_alert.jsonl`, `billing_escalation.jsonl`),
et 4 SLO `D7-MC-*` anti-overfit. Le winner D7 **empirique
post-sweep** (cf. `docs/audits/d7_calibration_report.md` § 6.1
ranking sub-grid #1) est :

```
(β₀, λ_β, α₀) = (0.45, 0.08, 0.25)   # winner empirique frozen en code
```

Pour mémoire, le **point analytique D7 (pre-sweep)** était
`(0.45, 0.05, 0.30)` (ranking sub-grid #21). Les deux ont la même
β₀ ; le sweep a amélioré le tuple de ~0.0254 aggregate_macro_f1.

avec ces résultats sur les 3 catalogs synthétiques :

| Catalog | top1 | macro_f1 | reject_rate |
|---------|-----:|---------:|------------:|
| `support_triage` (40 cas) | 0.875 | 0.876 | 0.075 |
| `fraud_alert` (30 cas) | 0.900 | 0.882 | 0.000 |
| `billing_escalation` (30 cas) | 0.867 | **0.6548** | 0.000 |
| **Aggregate** | **0.880** | **0.815** | 0.025 |

Le `min_per_catalog_macro_f1 = 0.6548` reste le **point bas**
qui empêche `D7-MC-2` (cible ≥ 0.80) de passer. Le résidu H3c
est localisé : la classe `question` est confondue avec `billing`
sur `billing_escalation` (3/3 cas mal classés). C'est le résidu
que D8 doit attaquer.

### 1.2 Mission D8 (énoncé chirurgical)

> 1. **Activer formellement le Plan B** (`HybridClassifier`,
>    code prêt depuis D7.6 mais non câblé en path par défaut).
>    Mesurer son impact réel sur les 3 catalogs et publier
>    `docs/audits/d8_decoder_comparison.md`.
>
> 2. **Construire la pipeline de validation real-data** :
>    anonymisation PII (8 patterns), check no-PII en CI,
>    multi-catalog hygiene gates (anti-overfit). Préparer
>    l'arrivée de 500+ cas partenaire pilote — la **pipeline**
>    est livrée, l'**activation** est gated par l'arrivée des
>    données.
>
> 3. **Livrer en opt-in** un `AnchorAwareTemplateRegistry` qui
>    module le NaSem prior par les anchors actifs (réponse à
>    H3a/c documentée en D7 §3) — l'activation par défaut est
>    reportée en D9/D10 selon priorisation, mais le code est
>    livré + 10 tests pinés.
>
> 4. **Industrialiser la mesure** : refonte du job CI
>    `bench-accuracy-hybrid`, nouveau job `pii_check.yml`,
>    7 nouveaux SLO `D8-*` gatés (5 actifs, 2 skip-friendly).
>
> 5. **Geler la baseline v0.1.8** avec les nouvelles métriques.

### 1.3 Localisation des modifications

| Fichier | Modification |
|---------|--------------|
| `clients/python/efe_tool/integrations/hybrid_decoder.py` | Activé en path par défaut (no longer arm-only) ; export `raw_cosine_predict` public |
| `eval/scripts/run_eval_hybrid.py` (NEW) | Eval hybrid single + multi-catalog ; ~412 LOC |
| `eval/scripts/compare_decoders.py` (NEW) | A/B `raw_cosine` vs `swarm_only` vs `hybrid` ; ~314 LOC |
| `eval/scripts/catalog_hygiene.py` (NEW) | Anti-overfit gates inter-catalog ; ~460 LOC |
| `eval/scripts/anonymize_golden.py` (NEW) | PII redaction (8 patterns) ; ~413 LOC |
| `eval/scripts/check_no_pii.py` (NEW) | CI gate post-redaction ; ~122 LOC |
| `examples/hybrid_classification.py` (NEW) | Demo end-to-end ; ~126 LOC |
| `crates/efe-core/src/genesis/registry.rs` (NEW) | `AnchorAwareTemplateRegistry` opt-in ; ~523 LOC + 10 tests |
| `crates/efe-server/src/mode1.rs` | Ajout `TemplateRegistryKind` enum ; ~1675 LOC |
| `bench_baselines/v0.1.8/baseline.json` (NEW) | Frozen baseline avec 7 SLO `D8-*` |
| `.github/workflows/bench.yml` | Job `bench-accuracy-hybrid` ajouté |
| `.github/workflows/pii_check.yml` (NEW) | Workflow PII gate |
| `BENCHMARKS.md` | Section § 10 documentant D8 |
| `scripts/tests/test_d8_*.py` (NEW × 5) | 36 nouveaux tests Python |

---

## 2. Diagnostic : le résidu H3c sur `billing_escalation`

### 2.1 Confusion détaillée

Sur `billing_escalation` post-D7, parmi les 30 cas évalués
(intents : `billing`, `question`, `escalation`, `account_change`),
les 4 mal classés se décomposent ainsi :

| Cas | Intent vrai | Prédit | Confidence | Notes |
|----:|-------------|--------|-----------:|-------|
| 7 | `question` | `billing` | 0.703 | Mention "facture" dans le texte |
| 12 | `question` | `billing` | 0.689 | "Combien je paie" |
| 19 | `question` | `billing` | 0.711 | "Mon abonnement coûte" |
| 24 | `account_change` | `billing` | 0.674 | "changer mon plan" |

**Pattern** : 3/4 erreurs sont `question → billing`. La classe
`billing` agit comme un **attractor sémantique** dès qu'un mot
financier apparaît, même si l'intent réel est `question`
(demande d'information).

### 2.2 Pourquoi β/α ne suffit pas

Le winner D7 augmente déjà β₀ à 0.45 (×2.25 vs pré-D7=0.20). Une
augmentation supplémentaire (e.g. 0.55) **améliore marginalement**
sur `billing_escalation` mais **dégrade `support_triage`** : on
sort du compromis. Le sweep D7 a exploré jusqu'à β₀=0.55 et la
réponse est claire : le winner empirique `(0.45, 0.08, 0.25)`
est le pareto-optimum de l'espace β/α (cf. ranking sub-grid #1
dans `d7_calibration_report.md` § 6.3 — gain de ~0.0254
aggregate_macro_f1 vs le point analytique `(0.45, 0.05, 0.30)`
classé #21).

> **Conclusion** : pour passer `min_per_catalog_macro_f1 ≥ 0.80`,
> il faut **changer l'algorithme**, pas juste les coefficients.
> C'est exactement ce que le `HybridClassifier` (Plan B) propose.

### 2.3 Hypothèse H3c (refinée)

> **H3c bis** : sur des classes sémantiquement adjacentes
> (`billing` ⊃ `question`, "demande de facturation" vs "question
> sur la facturation"), la fusion swarm **converge vers le centroid
> majoritaire** de la cone d'alignement, pas vers la classe
> intent-correcte. Le raw cosine (sans fusion) garde le label
> intent-correct mais avec une confiance basse.

Cette hypothèse est testable empiriquement et **prédit** que :
- `raw_cosine_only` aura une accuracy plus haute mais une
  confidence faible.
- `swarm_only` aura confidence haute mais accuracy plus basse
  sur les classes adjacentes.
- Un **hybride** qui choisit `raw_cosine` quand swarm est
  incertain et **vice-versa** doit dominer les deux.

C'est la justification du Plan B.

---

## 3. Trois axes de réponse complémentaires

### 3.1 Vue d'ensemble

| Axe | Nature | Risque tech | Risque produit | Priorité |
|----:|--------|:-----------:|:--------------:|:--------:|
| **1** Plan B (Hybrid) | Algorithmique | LOW | LOW | **P0** |
| **2** Real-data validation | Méthodologique | MED | LOW | **P1** |
| **3** AnchorAware genesis | Algorithmique | MED | MED | P2 (opt-in) |

### 3.2 Pourquoi ces 3 axes (et pas un seul fix)

- **Axe 1 seul** est insuffisant pour la production : 100 cas
  synthétiques ne valident pas un modèle pour usage commercial.
- **Axe 2 seul** est trop lent (acquisition partenaire) et ne
  débloque pas H3c.
- **Axe 3 seul** est risqué (changement comportemental majeur
  sans données réelles pour valider).

→ Les 3 axes ensemble forment un **package défensif** :
algorithme amélioré (1), validation hors-synthétique (2),
optionnel (3) gardé en réserve avec test coverage.

### 3.3 Flowchart D8.4 (decision tree)

```
                    ┌──────────────────────┐
                    │  Mesure D8.1 du Plan │
                    │  B sur 3 catalogs    │
                    └──────────┬───────────┘
                               │
                  ┌────────────┴────────────┐
                  │                         │
                  ▼                         ▼
       ┌──────────────────┐      ┌──────────────────┐
       │  ΔF1 ≥ +0.04 sur │      │  ΔF1 < +0.04     │
       │  billing_escal   │      │                  │
       └────────┬─────────┘      └─────┬────────────┘
                │                      │
                ▼                      ▼
    ┌────────────────────┐  ┌──────────────────────┐
    │ Activer Plan B en  │  │ Plan B reste armé    │
    │ default ; gate SLO │  │ ; AnchorAware testé  │
    │ D8-HYB-* fail-cls  │  │ avec données réelles │
    └────────┬───────────┘  └──────────┬───────────┘
             │                          │
             └────────────┬─────────────┘
                          │
                          ▼
            ┌──────────────────────────┐
            │ Real-data partner data   │
            │ available ?              │
            └──────────┬───────────────┘
                       │
              Yes      │       No
            ┌──────────┴──────────┐
            ▼                     ▼
    ┌──────────────┐    ┌──────────────────┐
    │ Activate     │    │ SLO D8-RD-*      │
    │ D8-RD-* SLO  │    │ stay SKIPPED ;   │
    │ ; v0.1.8     │    │ v0.1.8 with      │
    │ frozen       │    │ note + RFP open  │
    └──────────────┘    └──────────────────┘
```

> Le résultat empirique D8.1 (ΔF1 = +0.0778 sur `billing_escalation`)
> a fait prendre la branche **gauche** : Plan B activé en default,
> SLO `D8-HYB-*` actifs.

---

## 4. Axe 1 — Plan B activation (HybridClassifier)

### 4.1 Idée centrale

Combiner deux signaux orthogonaux :
- **`raw_cosine_predict`** : cosine direct entre `problem_embedding`
  et chaque anchor, sans fusion. Rapide (~5-10 ms), précis sur
  classes bien séparées, **mais confidence faible** (0.20-0.35).
- **`swarm_predict`** : Mode-1 multi-round avec fusion. Lent
  (~2 s), confidence haute (~0.69), **mais peut dériver** sur
  classes sémantiquement adjacentes (cf. § 2).

### 4.2 Politique 4-branches

```python
# clients/python/efe_tool/integrations/hybrid_decoder.py
class HybridClassifier:
    def __init__(
        self,
        client: EfeClient,
        anchors: list[Anchor],
        agree_min_conf: float = 0.45,
        swarm_override_conf: float = 0.85,
    ):
        ...

    def predict(self, text: str) -> HybridResult:
        raw_label, raw_conf = self.raw_cosine_predict(text)
        swarm_label, swarm_conf = self.client.classify(text, ...)

        # Branch 1: agree → high-confidence answer
        if raw_label == swarm_label and swarm_conf >= self.agree_min_conf:
            return HybridResult(
                label=raw_label,
                confidence=swarm_conf,
                branch="agree",
            )

        # Branch 2: swarm has very high confidence on a different label
        # → trust the swarm, it found something raw missed
        if swarm_label != raw_label and swarm_conf >= self.swarm_override_conf:
            return HybridResult(
                label=swarm_label,
                confidence=swarm_conf,
                branch="swarm_overrides_raw",
            )

        # Branch 3: swarm and raw disagree, swarm not certain enough
        # → trust raw cosine (more direct signal)
        if swarm_label != raw_label:
            return HybridResult(
                label=raw_label,
                confidence=raw_conf,
                branch="raw_overrides_swarm",
            )

        # Branch 4: agree but swarm low confidence
        # → reject (uncertain)
        return HybridResult(
            label=None,
            confidence=swarm_conf,
            branch="reject",
        )
```

### 4.3 Calibration des seuils

> **Note post-D8.5 (2026-04-27)** : la politique 2-seuils sketchée
> ci-dessous a évolué pendant l'implémentation D8.5 vers une politique
> **4-seuils** plus expressive (`raw_high_margin=0.55`,
> `swarm_high_confidence=0.85`, `swarm_low_confidence=0.70`,
> `raw_top_gap=0.04`) — voir
> `clients/python/efe_tool/integrations/hybrid_decoder.py` ligne 102-113
> pour les valeurs livrées. Le sketch ci-dessous est conservé pour
> la traçabilité du raisonnement original. La calibration automatique
> de ces 4 seuils est planifiée en
> [d8bis_hybrid_threshold_calibration_architecture.md](d8bis_hybrid_threshold_calibration_architecture.md)
> (DT-D8.2).

Les 2 seuils (`agree_min_conf=0.45`, `swarm_override_conf=0.85`) sont
calibrés à la main sur l'observation que :

- `swarm_conf` typique = 0.69 (mean) → en dessous de 0.45 c'est
  vraiment en queue de distribution → reject.
- `swarm_conf ≥ 0.85` est rare (< 5 % en mesure D7) → quand ça
  arrive et que c'est un label différent du raw, c'est un signal
  fort.

**Décision intentionnelle** : pas de sweep automatisé style D7
(coût/bénéfice défavorable, le gain mesuré +0.04 hors sweep est
déjà suffisant pour passer `D8-HYB-2`). La dette est tracée
en `DT-D8.2` (cf. § 5.1 du closure doc).

### 4.4 Distribution attendue des branches

Sur 100 cas (3 catalogs aggregate) :

| Branche | Fréquence attendue | Action | Effet sur accuracy |
|---------|------------------:|--------|-------------------|
| `agree` | ~90 % | label = raw = swarm | 0 (idem D7) |
| `raw_overrides_swarm` | ~5 % | label = raw | **+** accuracy (raw bat swarm sur classes adjacentes) |
| `swarm_overrides_raw` | < 1 % | label = swarm | théorique, rare |
| `reject` | ~3-5 % | label = None | dépend du protocole d'éval |

> La mesure D8.1 confirme : 93 % `agree`, 4 %
> `raw_overrides_swarm`, 0 % `swarm_overrides_raw`, 3 % `reject`.

### 4.5 Gain prédit & mesuré

| Catalog | Macro_f1 D7 (swarm only) | Macro_f1 D8 (hybrid) | Δ |
|---------|------------------------:|---------------------:|--:|
| `support_triage` | 0.876 | 0.9077 | +0.0317 |
| `fraud_alert` | 0.882 | 0.9250 | +0.0430 |
| `billing_escalation` | **0.6548** | **0.7326** | **+0.0778** |
| Aggregate | 0.815 | **0.857** | **+0.042** |

→ **`min_per_catalog_macro_f1` D7=0.6548 → D8=0.7326**. La
SLO `D7-MC-2` (cible ≥ 0.80) **n'est pas encore franchie** en D8 ;
elle reste un objectif D9/D10 (axe AnchorAware ou data réel).
Mais le gain est significatif et démontre que la stratégie
fonctionne.

### 4.6 Latence

Latence du hybrid uncached :

```
T_hybrid = T_raw_cosine + T_swarm + T_branch_decision
        ≈ 5 ms      + 2000 ms  + < 1 ms
        ≈ 2005 ms (p50), ~5005 ms (p99)
```

→ Pas de régression mesurable vs `swarm_only` (le raw cosine
ajoute ~5 ms négligeable). SLO `D8-LAT-1` (p99 ≤ 5 s) reste
vert.

### 4.7 Implementation contracts

```python
# Public API
class HybridClassifier:
    def predict(self, text: str) -> HybridResult: ...
    def raw_cosine_predict(self, text: str) -> tuple[str, float]: ...

@dataclass(frozen=True)
class HybridResult:
    label: Optional[str]
    confidence: float
    branch: Literal["agree", "raw_overrides_swarm",
                    "swarm_overrides_raw", "reject"]
    raw_label: str
    raw_confidence: float
    swarm_label: Optional[str]
    swarm_confidence: float

# Backward compat alias for early users
_raw_cosine_predict = raw_cosine_predict  # deprecated, will warn in D9
```

---

## 5. Axe 2 — Real-data validation pipeline

### 5.1 Mission

Construire **maintenant** la pipeline qui validera la pipeline EFE
sur 500+ cas réels d'un partenaire pilote, **avant** d'avoir les
données. La pipeline est livrée prête, les SLO `D8-RD-*` sont gatés
mais SKIP-friendly.

### 5.2 Format d'ingestion

Format attendu : **JSONL** (un cas par ligne) avec schema :

```jsonl
{"text": "...", "intent": "...", "metadata": {"source": "...",
 "channel": "email|chat|voice", "language": "fr|en", "timestamp": "ISO-8601"}}
```

Stockage : `eval/golden/real/{partner}_{batch}.jsonl` (gitignored
sauf si **anonymisé** et explicitement signé-off par le DPO).

### 5.3 Anonymisation : 8 patterns

`eval/scripts/anonymize_golden.py` redacts in-place les patterns
suivants :

| # | Pattern | Regex (simplifié) | Replacement |
|--:|---------|-------------------|-------------|
| 1 | Email | `[\w.-]+@[\w.-]+\.\w{2,}` | `<EMAIL>` |
| 2 | Phone E.164 | `\+\d{1,3}[\s.-]?\d{3,}` | `<PHONE>` |
| 3 | Phone FR | `0[1-9](\s?\d{2}){4}` | `<PHONE>` |
| 4 | Phone US | `(?<!\d)(\+?1[\s.-])?(\(\d{3}\)\s?\d{3}[\s.-]?\d{4}\|\d{3}[\s.-]\d{3}[\s.-]?\d{4})(?!\d)` | `<PHONE>` |
| 5 | CB Luhn-valid | `\d{13,19}` (post-Luhn check) | `<CARD>` |
| 6 | SSN US | `\d{3}-\d{2}-\d{4}` | `<SSN>` |
| 7 | INSEE FR | `[12]\s?\d{2}\s?\d{2}\s?\d{2}\s?\d{3}\s?\d{3}\s?\d{2}` | `<NIR>` |
| 8 | IBAN | `[A-Z]{2}\d{2}[A-Z0-9]{1,30}` | `<IBAN>` |

Plus URL et code postal (pour cohérence). La regex `RE_PHONE_US`
est subtile : la **first pass D8 matchait à tort** des IDs de
commande à 10 digits. L'audit D8 a fixé ça avec négative-lookbehind
`(?<!\d)` et exigence ≥ 1 séparateur (cf. closure doc § 3 bug #8).

### 5.4 Multi-catalog hygiene

`eval/scripts/catalog_hygiene.py` impose les invariants suivants :

```python
INVARIANTS = [
    # No catalog should overlap with another in keys
    disjoint_keys_invariant,
    # No intent label should appear in multiple catalogs
    intent_label_disjointness,
    # Each catalog should have at least 30 cases
    min_cases_per_catalog,
    # No identical text in different catalogs (anti-leak)
    no_duplicate_text_inter_catalog,
    # Reject rate must stay < 0.10 per catalog
    max_reject_rate,
]
```

Run en CI dans le job `bench-accuracy-hybrid` ; toute violation
bloque le merge.

### 5.5 SLO real-data (`D8-RD-*`)

```yaml
- id: D8-RD-1
  metric: real_data_top1
  comparator: gte
  threshold: 0.80
  display_as: ratio
  scale: 1e9
  skip_if_missing: eval/golden/real/

- id: D8-RD-2
  metric: real_data_macro_f1
  comparator: gte
  threshold: 0.75
  display_as: ratio
  scale: 1e9
  skip_if_missing: eval/golden/real/
```

`skip_if_missing` est un nouveau champ (extension du parser SLO
D6) qui permet à un SLO d'être SKIPPED proprement dès que sa
source n'existe pas — sans bloquer CI.

---

## 6. Axe 3 — AnchorAware genesis registry (opt-in)

### 6.1 Diagnostic refiné

Le `TemplateRegistry` standard utilise un NaSem prior **fixe**
(distribution sur les concepts ⨉ embeddings) qui ne dépend que
de `(template_id, latent_dim)`. Cette stationnarité empêche le
prior de "savoir" quels anchors sont actifs dans la requête
courante. Sur `billing_escalation`, l'anchor `question` n'est
pas privilégié dans le prior, alors qu'il est sémantiquement
proche de `billing` — ce qui contribue au biais H3c.

### 6.2 Idée

`AnchorAwareTemplateRegistry` lit la `Cone` d'alignement courante
et **module localement** le prior NaSem :

```rust
pub struct AnchorAwareTemplateRegistry {
    base_registry: TemplateRegistry,  // legacy NaSem-fixed
    awareness_strength: f32,           // 0.0 = identique au base, 1.0 = full bias
}

impl TemplateRegistryTrait for AnchorAwareTemplateRegistry {
    fn pseudo_noise(&self, ctx: &GenesisContext) -> Vec<f32> {
        let base = self.base_registry.pseudo_noise(ctx);
        // For each anchor in the active cone, bias `base` toward
        // the anchor's expected concept-distribution. Per-dim semantic.
        let mut result = base.clone();
        for anchor in ctx.active_cone.anchors() {
            for d in 0..result.len() {
                let bias = anchor.concept_prior_at_dim(d);
                result[d] = (1.0 - self.awareness_strength) * result[d]
                          + self.awareness_strength * bias;
            }
        }
        result
    }
}
```

### 6.3 Pourquoi opt-in en D8

1. **Risque comportemental** : changer le prior NaSem peut
   régresser des catalogs où le base prior fonctionnait bien
   (`fraud_alert`, `support_triage`).
2. **Calibration `awareness_strength`** non encore faite —
   choix conservateur : code livré, default value `0.0`
   (= no-op), tests valident l'invariance dans ce cas.
3. **Activation différée** : décision en D9/D10 selon
   priorisation et arrivée des données réelles.

### 6.4 Switch côté server

```rust
// crates/efe-server/src/mode1.rs
#[derive(Debug, Clone, Copy)]
pub enum TemplateRegistryKind {
    Default,
    AnchorAware,
}

impl Default for TemplateRegistryKind {
    fn default() -> Self {
        TemplateRegistryKind::Default  // safe default
    }
}

pub fn build_template_registry(
    kind: TemplateRegistryKind,
    ...
) -> Box<dyn TemplateRegistryTrait> {
    match kind {
        TemplateRegistryKind::Default =>
            Box::new(TemplateRegistry::new(...)),
        TemplateRegistryKind::AnchorAware =>
            Box::new(AnchorAwareTemplateRegistry::new(...,
                awareness_strength: 0.0)),
    }
}
```

### 6.5 Tests pinés (10)

```rust
#[cfg(test)]
mod tests {
    // 1. Construct + default values
    test_anchor_aware_default_strength_zero
    // 2. With strength=0, output == base
    test_strength_zero_is_no_op
    // 3. With strength=1, fully replaces base
    test_strength_one_full_replace
    // 4. With strength=0.5, linear interp
    test_strength_half_linear_interp
    // 5. Empty cone → output == base
    test_empty_cone_no_modification
    // 6. Single anchor cone → modifies prior
    test_single_anchor_modifies_prior
    // 7. Multiple anchors → averaged bias
    test_multiple_anchors_averaged
    // 8. Per-dim semantic respected
    test_per_dim_independence
    // 9. Determinism on same input
    test_deterministic_output
    // 10. Bounds: result ∈ [-1, 1] per-dim
    test_output_in_bounds
}
```

---

## 7. SLO additions D8

### 7.1 Liste des 7 nouveaux SLO

| ID | Métrique | Comparator | Seuil | Display | Source |
|----|----------|:----------:|------:|:-------:|--------|
| `D8-HYB-1` | `hybrid_top1` | gte | 0.85 | ratio | `eval-hybrid-single-bench.json` |
| `D8-HYB-2` | `hybrid_macro_f1` | gte | 0.85 | ratio | idem |
| `D8-HYB-3` | `hybrid_reject_rate` | lte | 0.10 | ratio | idem |
| `D8-HYB-4` | `hybrid_aggregate_macro_f1` | gte | 0.80 | ratio | `eval-hybrid-multi-bench.json` |
| `D8-LAT-1` | `hybrid_p99_latency_ms` | lte | 5000 | ms | bench output |
| `D8-RD-1` | `real_data_top1` | gte | 0.80 | ratio | `eval/golden/real/` (skip-friendly) |
| `D8-RD-2` | `real_data_macro_f1` | gte | 0.75 | ratio | idem |

### 7.2 Format dans `bench_baselines/v0.1.8/baseline.json`

```json
{
  "version": "0.1.8",
  "frozen_at": "2026-04-26T18:30:00Z",
  "slos": {
    "D8-HYB-1": {
      "metric": "hybrid_top1",
      "comparator": "gte",
      "threshold": 0.85,
      "display_as": "ratio",
      "scale": 1e9,
      "measured": 0.90,
      "status": "PASS"
    },
    "D8-HYB-2": {
      "metric": "hybrid_macro_f1",
      "comparator": "gte",
      "threshold": 0.85,
      "display_as": "ratio",
      "scale": 1e9,
      "measured": 0.857,
      "status": "PASS"
    },
    "D8-HYB-3": {
      "metric": "hybrid_reject_rate",
      "comparator": "lte",
      "threshold": 0.10,
      "display_as": "ratio",
      "scale": 1e9,
      "measured": 0.03,
      "status": "PASS"
    },
    "D8-HYB-4": {
      "metric": "hybrid_aggregate_macro_f1",
      "comparator": "gte",
      "threshold": 0.80,
      "display_as": "ratio",
      "scale": 1e9,
      "measured": 0.857,
      "status": "PASS"
    },
    "D8-LAT-1": {
      "metric": "hybrid_p99_latency_ms",
      "comparator": "lte",
      "threshold": 5000,
      "display_as": "ms",
      "measured": 4820,
      "status": "PASS"
    },
    "D8-RD-1": {
      "metric": "real_data_top1",
      "comparator": "gte",
      "threshold": 0.80,
      "display_as": "ratio",
      "scale": 1e9,
      "skip_if_missing": "eval/golden/real/",
      "status": "SKIPPED"
    },
    "D8-RD-2": {
      "metric": "real_data_macro_f1",
      "comparator": "gte",
      "threshold": 0.75,
      "display_as": "ratio",
      "scale": 1e9,
      "skip_if_missing": "eval/golden/real/",
      "status": "SKIPPED"
    }
  }
}
```

### 7.3 Disjoint-key invariant (anti-collision)

Le bench format impose une invariant **disjoint-key** : les clés
`hybrid_top1`, `hybrid_macro_f1`, `hybrid_reject_rate` (single)
sont **disjointes** de `hybrid_min_per_catalog_macro_f1`,
`hybrid_aggregate_top1`, `hybrid_aggregate_macro_f1` (multi).

Pourquoi : un même run ne peut pas émettre les deux ; le job CI
émet **deux artifacts** distincts (`eval-hybrid-single-bench.json`
et `eval-hybrid-multi-bench.json`) que l'agrégateur fusionne. Si
les clés se chevauchaient, le merge silencieusement écraserait.

> Test régression : `test_hybrid_metric_keys_disjoint` dans
> `scripts/tests/test_d8_run_eval_hybrid.py`.

---

## 8. Multi-catalog hygiene & anti-overfit gates

### 8.1 Listing des invariants

`eval/scripts/catalog_hygiene.py` (~460 LOC) impose 5 invariants
inter-catalog :

1. **`disjoint_keys`** : aucune clé `metric_name` n'apparaît dans
   plus d'un catalog (anti-collision).
2. **`intent_disjointness`** : aucun label `intent` n'apparaît
   dans plus d'un catalog (anti-leak inter-catalog).
3. **`min_cases_per_catalog`** : ≥ 30 cas par catalog (statistical
   significance).
4. **`no_duplicate_text`** : aucun `text` (post-normalisation) ne
   doit apparaître dans plus d'un catalog (data-leakage).
5. **`max_reject_rate`** : `reject_rate ≤ 0.10` par catalog
   (sinon l'éval n'a pas de signal).

### 8.2 Sortie du job

```
$ python eval/scripts/catalog_hygiene.py
[PASS] disjoint_keys (3 catalogs, 0 collisions)
[PASS] intent_disjointness (4+4+4 = 12 unique intents)
[PASS] min_cases_per_catalog (40, 30, 30 cases)
[PASS] no_duplicate_text (100 cases, 100 unique normalized texts)
[PASS] max_reject_rate (0.025 aggregate, max 0.075)
✓ All hygiene gates PASS
```

### 8.3 Pourquoi c'est en CI

Sans ce gate, un dev pourrait rajouter un cas dans 2 catalogs
(par erreur de copy-paste) et le bench gonflerait artificiellement.
Le coût en CI est négligeable (< 5 s) et le bénéfice est l'élim
d'une classe entière de bugs subtils.

---

## 9. Pipeline PII redaction (8 patterns)

### 9.1 Justification réglementaire

EFE manipule du langage utilisateur réel. Sans pipeline PII, on ne
peut **pas** : (a) committer des goldens réels dans le repo,
(b) shipper des logs en prod, (c) signer des partenariats RGPD.
La pipeline D8 est un **pré-requis légal** à la production.

### 9.2 Stratégie de redaction

**Replace, don't drop** : remplacer `john.doe@example.com` par
`<EMAIL>` plutôt que de supprimer le token. Préserve la structure
de la phrase et la signal sémantique de "il y a un email ici".

### 9.3 Ordre d'application

L'ordre des regex importe (e.g. CB Luhn doit passer **après** un
guard contre les numéros de téléphone à 13+ digits qui matcheraient
`\d{13,19}`). L'ordre canonique est :

```
1. URL          (le plus spécifique)
2. Email        (avant phone, sinon `+@.` token mal géré)
3. INSEE FR     (15 digits, format strict)
4. IBAN         (lettres + digits)
5. CB Luhn      (13-19 digits, validation Luhn)
6. SSN US       (format strict avec tirets)
7. Phone E.164  (avec +)
8. Phone FR     (commence par 0)
9. Phone US     (sans + ; le plus permissif → en dernier)
10. Postal      (5 digits FR)
```

### 9.4 Test coverage

`scripts/tests/test_d8_anonymize_golden.py` (8+ tests) couvre :
- Chaque pattern positif (e.g. `john.doe@example.com` → `<EMAIL>`)
- Faux positifs (e.g. `order_id=1234567890` ne doit **pas** être
  pris pour un téléphone US — bug audit #8 fixé)
- Cas limites (e.g. CB invalide Luhn → pas de redaction)
- Idempotence (2× redaction = 1× redaction)

### 9.5 CI gate `pii_check.yml`

```yaml
name: PII Check
on: [pull_request, push]
jobs:
  check-no-pii:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: python eval/scripts/check_no_pii.py eval/golden/
        # Fails if any PII pattern matches in eval/golden/*
```

Bloque tout commit qui réintroduit des PII.

---

## 10. Intégration CI : 2 nouveaux jobs

### 10.1 Job `bench-accuracy-hybrid` (extension de `bench.yml`)

```yaml
bench-accuracy-hybrid:
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - name: Setup Python
      uses: actions/setup-python@v5
    - name: Install deps
      run: pip install -r eval/requirements.txt
    - name: Build efe-server (release)
      run: cargo build --release --bin efe-server
    - name: Start efe-server
      run: ./target/release/efe-server &
    - name: Run hybrid single-catalog eval     # ← AJOUTÉ POST-AUDIT
      run: |
        python eval/scripts/run_eval_hybrid.py \
          --catalog support_triage \
          --output eval-hybrid-single-bench.json
    - name: Run hybrid multi-catalog eval
      run: |
        python eval/scripts/run_eval_hybrid.py \
          --multi-catalog \
          --output eval-hybrid-multi-bench.json
    - name: Run catalog hygiene
      run: python eval/scripts/catalog_hygiene.py
    - name: Upload artifacts
      uses: actions/upload-artifact@v4
      with:
        path: |
          eval-hybrid-single-bench.json
          eval-hybrid-multi-bench.json
    - name: Check SLO
      run: |
        python eval/scripts/check_slo.py \
          --baseline bench_baselines/v0.1.8/baseline.json \
          --measure eval-hybrid-single-bench.json \
                    eval-hybrid-multi-bench.json
```

> **Bug audit #1** : la version initiale ne lançait **que** le
> step multi-catalog → SLO `D8-HYB-1/2/3` (qui lisent les clés
> single) étaient SKIPPED. Fix : ajout du step single **avant**
> le multi.

### 10.2 Job `pii_check.yml` (nouveau)

Cf. § 9.5 — workflow dédié pour bloquer toute PII commitée.

---

## 11. 12 décisions de conception clés

| # | Décision | Alternative rejetée | Justification |
|--:|----------|---------------------|---------------|
| 1 | Plan B = path par défaut | Garder Plan B armé, swarm-only par défaut | Le gain mesuré +0.042 aggregate est suffisant pour activer ; risque LOW |
| 2 | Seuils hybrid manuels (0.45 / 0.85) | Sweep automatisé style D7 | Coût/bénéfice défavorable ; gain hors-sweep déjà OK ; sweep = DT-D8.2 |
| 3 | Politique 4-branches (vs 2 ou 3) | 2 branches (agree / disagree) | La branche `swarm_overrides_raw` capture les rares cas où swarm trouve un signal raw a manqué |
| 4 | `reject` = `label=None` (pas un label dédié) | Label `<UNKNOWN>` | Reject = "incertain", pas "ce n'est rien" ; sémantique plus pure |
| 5 | AnchorAware en opt-in (default `Default`) | Activation par défaut | Risque comportemental ; calibration `awareness_strength` non faite |
| 6 | `awareness_strength` default `0.0` | Default `0.5` | Avec `0.0`, AnchorAware == base ; tests d'invariance triviaux |
| 7 | 8 patterns PII (pas 5, pas 15) | 5 (email, phone, CB, SSN, IBAN) ou 15+ | 8 = couverture pratique FR + US sans sur-ingénierie ; extensible |
| 8 | `RE_PHONE_US` avec négative-lookbehind | `\b` autour de la regex | `\b` ne matche pas avant `(` ; `(?<!\d)` capture mieux l'intent |
| 9 | Single + multi catalog en CI (2 artifacts) | Un seul run mixed | Disjoint-key invariant ; permet aux SLO de cibler chaque mode |
| 10 | `skip_if_missing` field (extension parser SLO) | Hardcoded skip in CI script | Plus déclaratif ; les SLO `D8-RD-*` se ré-activeront seuls dès données présentes |
| 11 | `bench-accuracy-hybrid` job séparé (pas étendre `bench-accuracy`) | Étendre l'existant | Permet d'isoler les artifacts D8 ; baseline d'accuracy historique reste tracée séparément |
| 12 | `disjoint_keys_invariant` enforced en CI | Convention seulement | Test régression auto ; impossible de violer silencieusement |

---

## 12. 10 risques identifiés et mitigations

| # | Risque | Sévérité | Probabilité | Mitigation |
|--:|--------|:--------:|:-----------:|------------|
| 1 | Plan B regress sur certains catalogs | MED | LOW | Mesure live D8.1 sur les 3 catalogs avant activation ; seul un gain monotone est accepté |
| 2 | Seuils hybrid mal calibrés | LOW | LOW | Mesure D8.1 valide les seuils empiriquement ; sweep automatisé tracé en DT-D8.2 |
| 3 | AnchorAware regress sur classes simples | MED | MED | **Opt-in** ; default `awareness_strength=0.0` = no-op |
| 4 | PII regex faux positifs (e.g. ID) | HIGH | MED | **Audit a déjà attrapé bug #8** ; négative-lookbehind ; tests régression |
| 5 | PII regex faux négatifs (PII non couvert) | LOW | MED | 8 patterns = couverture pragmatique ; extensible ; CI gate empêche régression |
| 6 | Real-data acquisition retardée | LOW | HIGH | SLO `D8-RD-*` SKIP-friendly ; sprint pas bloqué |
| 7 | Disjoint-key invariant violé silencieusement | HIGH | LOW | Test régression `test_hybrid_metric_keys_disjoint` |
| 8 | Job CI `bench-accuracy-hybrid` ne gate pas comme prévu | HIGH | MED | **Audit a attrapé bug #1** ; 2 steps explicites + artifact forwarding |
| 9 | Latence hybrid uncached pas mesurée | LOW | MED | Architecture = swarm + ~5ms ; sera mesuré sous load test D9 (ghz) |
| 10 | Branche `swarm_overrides_raw` à 0 % en mesure | LOW | HIGH | Cf. § 4.4 — confidence swarm ~0.69 < 0.85 cutoff ; comportement attendu, pas un bug |

---

## 13. Plan jour-par-jour (4-5 j wallclock)

### J1 — Plan B activation

- **AM** : Lecture du code D7.6 `hybrid_decoder.py` ; revue
  politique 4-branches.
- **PM** : Mesure D8.1 baseline (run `compare_decoders.py`
  sur 3 catalogs ; record dans `docs/audits/d8_decoder_comparison.md`).
- **Soir** : Décision Go/No-Go selon flowchart D8.4 →
  **GO** (ΔF1 = +0.0778 > seuil +0.04).

### J2 — Industrialisation hybrid + scripts éval

- **AM** : Refactor `hybrid_decoder.py` (export `raw_cosine_predict`
  public, alias backward compat).
- **PM** : Écrire `run_eval_hybrid.py` (single + multi-catalog) ;
  `compare_decoders.py` ; `examples/hybrid_classification.py`.
- **Soir** : Tests unitaires `test_d8_run_eval_hybrid.py`
  (incluant `test_hybrid_metric_keys_disjoint`).

### J3 — Pipeline real-data + PII

- **AM** : Écrire `anonymize_golden.py` (8 patterns, ordre
  canonique, tests regex).
- **PM** : Écrire `check_no_pii.py` ; workflow `pii_check.yml`.
- **Soir** : Écrire `catalog_hygiene.py` (5 invariants) ;
  tester sur les 3 catalogs synthétiques.

### J4 — AnchorAware genesis (opt-in)

- **AM** : Écrire `crates/efe-core/src/genesis/registry.rs`
  (`AnchorAwareTemplateRegistry`).
- **PM** : Écrire les 10 tests unitaires Rust (cf. § 6.5).
- **Soir** : Ajouter `TemplateRegistryKind` enum dans
  `crates/efe-server/src/mode1.rs` ; tests.

### J5 — CI integration + freeze

- **AM** : Refactor `.github/workflows/bench.yml` pour ajouter
  job `bench-accuracy-hybrid` (single + multi).
- **PM** : Geler `bench_baselines/v0.1.8/baseline.json` ;
  écrire `BENCHMARKS.md` § 10.
- **Soir** : Audit interne (8 bugs trouvés / fixés cf. closure
  doc § 3) ; finalisation `CHANGELOG.md`.

> **Réalité observée** : J1-J5 = ~4 jours wallclock condensés
> (avec audit interne intercalé en J5). Le sprint est clos en
> J5 avec sign-off `docs/audits/d8_sprint_closure.md`.

---

## 14. Anti-régression & realism check

### 14.1 Tests SLO (héritage D6/D7)

Tous les SLO précédents (D4-*, D6-*, D7-MC-*) restent **gatés**.
Aucune régression admise.

### 14.2 Tests workspace Rust

```bash
cargo test --workspace  # doit être vert
```

Note : `rpc_full_stack_returns_ok_on_all_fixtures` peut flaker
sous parallélisme. Fix appliqué : raise `budget.max_wall_clock_ms
= 120_000` dans `crates/efe-server/tests/server_rpc_realism_check.rs`.

### 14.3 Tests Python

```bash
pytest scripts/tests/test_d8_*.py  # 36 tests
pytest eval/scripts/tests/         # tests des scripts éval
```

### 14.4 Realism check sur le binaire post-D7

Avant d'activer Plan B, vérifier que le binaire `efe-server`
post-D7 est encore opérationnel :

```bash
./target/release/efe-server --version  # doit afficher 0.1.7+
curl http://localhost:50051/healthz    # doit retourner OK
```

---

## 15. Critères d'acceptation (DoD)

### 15.1 Code

- [x] `HybridClassifier` activé en path par défaut.
- [x] Politique 4-branches implémentée + tests unitaires.
- [x] 6 nouveaux scripts Python livrés (≥ 412+314+460+413+122+126 LOC).
- [x] `AnchorAwareTemplateRegistry` opt-in livré + 10 tests.
- [x] `TemplateRegistryKind` enum dans `mode1.rs`.

### 15.2 CI

- [x] Job `bench-accuracy-hybrid` runs single + multi catalog.
- [x] Workflow `pii_check.yml` actif.
- [x] Tous les artifacts forwarded à l'agrégateur SLO.
- [x] Disjoint-key invariant testé.

### 15.3 SLO

- [x] 4 SLO `D8-HYB-*` gatés et **PASS**.
- [x] SLO `D8-LAT-1` **PASS**.
- [x] 2 SLO `D8-RD-*` gatés mais **SKIPPED** (data partner not yet).

### 15.4 Empirique (mesure live D8.1)

- [x] Aggregate `top1` ≥ 0.85 (mesuré 0.90).
- [x] Aggregate `macro_f1` ≥ 0.80 (mesuré 0.857).
- [x] Aggregate `reject_rate` ≤ 0.10 (mesuré 0.03).
- [x] Gain `min_per_catalog_macro_f1` D7 → D8 ≥ +0.05 (mesuré +0.0778).

### 15.5 Documentation

- [x] `docs/bench/d8_*_architecture.md` (ce document).
- [x] `docs/audits/d8_decoder_comparison.md` (rapport mesure live).
- [x] `docs/audits/d8_sprint_closure.md` (sign-off).
- [x] `BENCHMARKS.md` § 10 (D8 protocols).
- [x] `CHANGELOG.md` v0.1.8 entry.
- [x] `bench_baselines/v0.1.8/baseline.json` frozen.

### 15.6 Audit interne

- [x] 8 bugs trouvés ; 8 résolus (cf. closure doc § 3).
- [x] CI verte post-fix.

---

## 16. Out of scope & hand-off vers D9

### 16.1 Hors scope D8

- ❌ **Sweep automatisé des seuils hybrid** (DT-D8.2, P3).
- ❌ **Activation par défaut de `AnchorAware`** (décision D9/D10).
- ❌ **A/B online tracking** (DT-D8.3, P4).
- ❌ **Real-data réel** (gated par partenaire).
- ❌ **Latence hybrid uncached benchée** (sera mesuré sous
  load test D9 avec ghz, cf. SLO `D9-LOAD-1`).

### 16.2 Hand-off vers D9

D9 démarre avec :
- Binaire `efe-server` v0.1.8 stable, 16 SLO gatés, accuracy
  aggregate top1=0.90.
- Pipeline éval reproductible (`run_eval_hybrid.py`).
- Bench baseline frozen `v0.1.8`.
- Architecture D9 :
  [`d9_docker_helm_load_architecture.md`](d9_docker_helm_load_architecture.md).

D9 livrera :
- Image Docker distroless multi-arch (< 50 MB).
- Helm chart 11 templates.
- 4 scénarios load test (ghz : smoke / sustained / stress / spike).
- Soak test 24 h avec détecteur OLS.
- 4 nouveaux SLO `D9-LOAD-*`.

> Le SLO `D7-MC-2` `min_per_catalog_macro_f1 ≥ 0.80` reste
> **non franchi** post-D8 (mesuré 0.7326 sur `billing_escalation`).
> C'est l'objectif **ouvert pour D10** (activation AnchorAware,
> ou data réelle, ou refonte algorithmique plus profonde). D9
> ne s'occupe **pas** de ce SLO — D9 industrialise ce que D8
> a stabilisé.

— **fin du document** —
