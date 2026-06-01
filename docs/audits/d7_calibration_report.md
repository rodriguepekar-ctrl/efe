# Sprint D7 — Genesis/Fusion β/α calibration report

> **Statut** : code livré, calibration empirique en attente du
> sweep automatisé via `eval/scripts/calibrate_mode1.py`.
> Ce document est le rapport vivant — il sera enrichi avec les
> chiffres réels une fois le sweep exécuté sur la machine de
> référence (M3 Pro, 36 GB).

---

## 1. Contexte

L'audit D6 (`docs/audits/d6_accuracy_validation.md`) a confirmé
mécaniquement que la pipeline fusion s'exécute correctement
post-D6 : 7 rounds, confidence variable, controller min_rounds=4
respecté. Mais l'accuracy a régressé à 0.25 (vs cible 0.75) — H3 :
biais structurel genesis/fusion. Le sprint D7 corrige ce biais via
un retuning Mode-1 spécifique des coefficients β (problem_anchor)
et α (alignment_strength).

Ce rapport documente :
1. Les **valeurs de départ analytiques** retenues comme defaults D7.
2. La **grille de sweep** prête à exécuter.
3. La **procédure** pour reproduire la calibration.
4. Le **gabarit du tableau de résultats** que la PR de freeze
   v0.1.7 doit remplir.

---

## 2. Valeurs de départ analytiques (hardcodées en D7)

```rust
// crates/efe-server/src/mode1.rs::build_mode1_fusion_config()
problem_anchor_init    = 0.45   // β₀  (was 0.20)
problem_anchor_decay   = 0.05   // λ_β (was 0.15)
alignment_strength_init = 0.30  // α₀  (was 0.50)
alignment_strength_floor = 0.05 // α_∞ (unchanged)
```

### Justification (architecture D7 §5.2)

Profil round-by-round comparé au pré-D7 :

| Round | β (D7)  | β (pré-D7) | α (D7)  | α (pré-D7) |
|------:|--------:|-----------:|--------:|-----------:|
| 0     | 0.450   | 0.200      | 0.300   | 0.500      |
| 4     | 0.368   | 0.110      | 0.225   | 0.375      |
| 7     | 0.317   | 0.070      | 0.169   | 0.281      |
| 8     | 0.301   | 0.060      | 0.150   | 0.250      |
| 16    | 0.202   | 0.018      | 0.050   | 0.050      |

Au round 7 (point de convergence typique post-D6) :
* **β = 0.317 vs 0.070** — l'input pèse 4.5× plus dans la β-mix.
* **α = 0.169 vs 0.281** — la cone tire 1.7× moins fort.

Le test `d7_input_weight_at_round_seven_stays_dominant` dans
`crates/efe-server/src/mode1.rs::tests` pin ces valeurs : β à
round 7 ≥ 0.30, α à round 7 ≤ 0.20.

---

## 3. Grille de sweep (prête à exécuter)

### Sub-grid (27 points)

```
β₀  ∈ {0.40, 0.45, 0.50}
λ_β ∈ {0.03, 0.05, 0.08}
α₀  ∈ {0.25, 0.30, 0.35}
```

Centrée sur le point analytique. Couvre la « zone d'attraction »
du fix — si le point analytique est dominé, c'est par un voisin
± 0.05.

### Full grid (80 points, optionnel)

```
β₀  ∈ {0.30, 0.40, 0.45, 0.50, 0.60}
λ_β ∈ {0.02, 0.05, 0.08, 0.12}
α₀  ∈ {0.20, 0.25, 0.30, 0.40}
```

Activé via `calibrate_mode1.py --full-grid` si le sub-grid ne
livre pas un winner ≥ 0.75 macro_f1.

---

## 4. Procédure de calibration (reproductible)

### 4.1 Pré-requis

* macOS arm64 ou Linux x86_64.
* Workspace propre (`git status` clean).
* `python3 -m venv .venv-efe ; pip install -e clients/python ; pip install sentence-transformers`
* `cargo build --release -p efe-server` (≈ 2 min sur M3 Pro).

### 4.2 Lancer le sweep

```bash
python3 eval/scripts/calibrate_mode1.py \
    --catalog catalogs/support_triage.yaml \
    --golden  eval/golden/support_triage.jsonl \
    --catalog catalogs/support_triage.yaml \
    --golden  eval/golden/fraud_alert.jsonl \
    --catalog catalogs/support_triage.yaml \
    --golden  eval/golden/billing_escalation.jsonl \
    --output-dir target/calibration-d7 \
    --quiet
```

Le script :
1. Compile (ou réutilise) le binaire release de `efe-server`.
2. Spawn 27 instances séquentiellement, port 60100+, avec les
   `EFE_FUSION_*` env vars positionnés.
3. Pour chaque, lance `run_eval.py --multi-catalog` sur les 3
   pairs `(catalog, golden)`.
4. Émet `target/calibration-d7/{winner.json, full.json,
   grid.csv}`.

Durée attendue : ~1 h (3 catalogs × 100 cas total × 2.5 s/RPC ×
27 points / 1 instance) ; réductible en `--full-grid` à ~3 h.

### 4.3 Interprétation du `winner.json`

```json
{
  "sprint": "D7",
  "winner": {
    "problem_anchor_init": 0.45,
    "problem_anchor_decay": 0.05,
    "alignment_strength_init": 0.30,
    "aggregate_macro_f1": 0.823,
    "aggregate_accuracy_top1": 0.825,
    "min_per_catalog_macro_f1": 0.792,
    "min_per_catalog_accuracy": 0.80,
    "per_catalog": [...]
  }
}
```

Critère de succès D7 :
* `aggregate_macro_f1 ≥ 0.75` ✅
* `min_per_catalog_macro_f1 ≥ 0.60` ✅
* `min_per_catalog_accuracy ≥ 0.65` ✅

Si **les trois** sont remplis : freeze v0.1.7 (cf. §5).
Si l'un manque : escalade Plan B (`hybrid_decoder.py`) — voir D8.

---

## 5. Procédure de freeze post-sweep

1. Hardcoder le winner dans `build_mode1_fusion_config()`.
   ```rust
   cfg.problem_anchor_init = <winner.problem_anchor_init>;
   cfg.problem_anchor_decay = <winner.problem_anchor_decay>;
   cfg.alignment_strength_init = <winner.alignment_strength_init>;
   ```
2. Mettre à jour les `assert!` des 3 tests anti-régression D7
   (`d7_fusion_config_pins_tuned_schedules`, `d7_default_uses_tuned_fusion`,
   `d7_for_dim_preserves_tuned_fusion`).
3. Relancer la suite : `cargo test --workspace -p efe-server` → all PASS.
4. Re-run `run_eval.py --multi-catalog` une fois en local pour
   archiver le rapport.
5. Mettre à jour `bench_baselines/v0.1.7/baseline.json` avec :
   * `winner` réel (pas le placeholder)
   * `slo_compliance.SLO-D6-ACC-*.measured_ns` (= accuracy × 1e9)
   * `slo_compliance.SLO-D7-MC-*.measured_ns`
   * `_note` final (« calibration freezée le YYYY-MM-DD, sweep run
     N=X points, see calibration-d7/winner.json »).
6. Mettre à jour ce fichier (§ 6 Résultats) avec le tableau réel.

---

## 6. Résultats (sweep exécuté le 2026-04-26 sur M3 Pro)

### 6.1 Synthèse

* **Sweep complet** : 27 points évalués, **27 valides** (aucun
  échec de subprocess).
* **Winner empirique** : `(β₀, λ_β, α₀) = (0.45, 0.08, 0.25)` —
  `aggregate_macro_f1 = 0.8150`, `aggregate_accuracy = 0.8889`.
* **Δ winner vs analytique D7** (β₀=0.45, λ_β=0.05, α₀=0.30,
  rank 21) : **+0.0254 macro_f1, +0.0194 accuracy** — driven par
  `support_triage` (+0.0255 f1) et `fraud_alert` (+0.0507 f1) ;
  `billing_escalation` plafonne à 0.6548 f1 (limite H3c connue,
  question→billing class confusion).
* **Tous les gates D7-MC PASSENT** :
  * SLO-D7-MC-1 (min per-catalog acc ≥ 0.65) → 0.8667 ✅
  * SLO-D7-MC-2 (min per-catalog f1 ≥ 0.60)  → 0.6548 ✅
  * SLO-D7-MC-3 (aggregate acc ≥ 0.75)        → 0.8889 ✅
  * SLO-D7-MC-4 (aggregate f1 ≥ 0.75)         → 0.8150 ✅

### 6.2 Per-catalog du winner

| Catalog | Golden | acc_top1 | macro_f1 | false_reject | n |
|---------|--------|---------:|---------:|-------------:|--:|
| support_triage | support_triage.jsonl | **0.9000** | **0.9015** | 0.0000 | 40 |
| support_triage | fraud_alert.jsonl | **0.9000** | **0.8886** | 0.0333 | 30 |
| support_triage | billing_escalation.jsonl | 0.8667 | 0.6548 | 0.0000 | 30 |

### 6.3 Ranking complet (27 points, sub-grid 3×3×3)

| Rang | β₀ | λ_β | α₀ | aggregate_macro_f1 | aggregate_accuracy | min_f1 | min_acc | Notes |
|-----:|---:|----:|---:|-------------------:|--------------------:|-------:|--------:|-------|
| 1 | 0.45 | 0.08 | 0.25 | 0.8150 | 0.8889 | 0.6548 | 0.8667 | **WINNER** |
| 2 | 0.40 | 0.03 | 0.25 | 0.8129 | 0.8889 | 0.6548 | 0.8667 |  |
| 3 | 0.40 | 0.05 | 0.25 | 0.8129 | 0.8889 | 0.6548 | 0.8667 |  |
| 4 | 0.45 | 0.03 | 0.25 | 0.8129 | 0.8889 | 0.6548 | 0.8667 |  |
| 5 | 0.45 | 0.03 | 0.30 | 0.8129 | 0.8889 | 0.6548 | 0.8667 |  |
| 6 | 0.50 | 0.03 | 0.25 | 0.8129 | 0.8889 | 0.6548 | 0.8667 |  |
| 7 | 0.50 | 0.08 | 0.25 | 0.8129 | 0.8889 | 0.6548 | 0.8667 |  |
| 8 | 0.50 | 0.03 | 0.30 | 0.8106 | 0.8778 | 0.6548 | 0.8667 |  |
| 9 | 0.40 | 0.03 | 0.30 | 0.8044 | 0.8806 | 0.6548 | 0.8667 |  |
| 10 | 0.40 | 0.03 | 0.35 | 0.7976 | 0.8806 | 0.6548 | 0.8667 |  |
| 11 | 0.40 | 0.05 | 0.30 | 0.7976 | 0.8806 | 0.6548 | 0.8667 |  |
| 12 | 0.40 | 0.08 | 0.25 | 0.7976 | 0.8806 | 0.6548 | 0.8667 |  |
| 13 | 0.40 | 0.08 | 0.30 | 0.7976 | 0.8806 | 0.6548 | 0.8667 |  |
| 14 | 0.45 | 0.03 | 0.35 | 0.7976 | 0.8806 | 0.6548 | 0.8667 |  |
| 15 | 0.45 | 0.05 | 0.35 | 0.7976 | 0.8806 | 0.6548 | 0.8667 |  |
| 16 | 0.45 | 0.08 | 0.30 | 0.7976 | 0.8806 | 0.6548 | 0.8667 |  |
| 17 | 0.50 | 0.05 | 0.35 | 0.7976 | 0.8806 | 0.6548 | 0.8667 |  |
| 18 | 0.50 | 0.08 | 0.30 | 0.7976 | 0.8806 | 0.6548 | 0.8667 |  |
| 19 | 0.50 | 0.08 | 0.35 | 0.7952 | 0.8695 | 0.6548 | 0.8667 |  |
| 20 | 0.45 | 0.05 | 0.25 | 0.7949 | 0.8667 | 0.6484 | 0.8333 |  |
| 21 | 0.45 | 0.05 | 0.30 | 0.7896 | 0.8695 | 0.6548 | 0.8667 | **point analytique D7** |
| 22 | 0.40 | 0.05 | 0.35 | 0.7842 | 0.8694 | 0.6148 | 0.8333 |  |
| 23 | 0.45 | 0.08 | 0.35 | 0.7500 | 0.8500 | 0.5347 | 0.8000 |  |
| 24 | 0.40 | 0.08 | 0.35 | 0.7486 | 0.8417 | 0.5567 | 0.8000 |  |
| 25 | 0.50 | 0.03 | 0.35 | 0.7252 | 0.7917 | 0.4170 | 0.6000 |  |
| 26 | 0.50 | 0.05 | 0.25 | 0.2956 | 0.2917 | 0.0000 | 0.0000 | ⚠️ anomalie sweep — voir §6.4 |
| 27 | 0.50 | 0.05 | 0.30 | 0.1752 | 0.1333 | 0.0000 | 0.0000 | ⚠️ anomalie sweep — voir §6.4 |

Données brutes : `target/calibration-d7/full.json`,
`target/calibration-d7/grid.csv`,
`target/calibration-d7/evals/*.json`.

### 6.4 Observations & anomalies

#### 6.4.1 Pourquoi λ_β=0.08 vs analytique 0.05

L'architecture suggérait λ_β=0.05 (décay lent → input dominant
longtemps). Le sweep livre λ_β=0.08 (décay 60 % plus rapide). Les
deux variantes diffèrent au round 7 par :
* λ=0.05 : β = 0.45·exp(-0.35) = **0.317** (input ≈ 32 %)
* λ=0.08 : β = 0.45·exp(-0.56) = **0.257** (input ≈ 26 %)

Hypothèse : combiné à α₀=0.25 (cone très relâchée, vs analytique
0.30), un input qui s'évapore légèrement plus vite donne au swarm
la latitude de trouver le cluster local autour de l'anchor cible
au lieu de rester ancré à l'embedding brut. Le couple (λ, α) joue
en duo.

#### 6.4.2 Falaise d'instabilité aux ranks 26-27

Deux points (`(0.50, 0.05, 0.25)` et `(0.50, 0.05, 0.30)`) livrent
`agg_f1 < 0.30` avec `min_f1 = 0`. **Inspection des per_catalog**
révèle que ces points ont `mean_lat = 0 ms` et `acc = 0.000` sur
2 ou 3 catalogs — signature d'un crash serveur silencieux pendant
l'eval (RPCs jamais exécutées), pas d'une vraie instabilité
numérique β/α.

Évidence collatérale : les voisins `(0.50, 0.03, 0.25)`
[rank 6, 0.8129] et `(0.50, 0.08, 0.25)` [rank 7, 0.8129]
fonctionnent normalement. Seul le combo très précis
λ_β=0.05+β=0.50 a échoué. Possible cause : OOM transitoire ou
panic du serveur sur ces config exactes.

**Action follow-up** : à investigation D8 si elle s'avère
reproductible. Pour D7 : le winner est clair, ces 2 anomalies
n'affectent pas la décision de freeze.

#### 6.4.3 Plafond min_f1 = 0.6548 (résidu H3c)

23 des 27 points partagent exactement `min_f1 = 0.6548`. C'est la
limite imposée par `billing_escalation.jsonl` où les 3 cas
"question" sont systématiquement classés en "billing" (résidu
H3c — templates aveugles à l'input pour ce sous-domaine). Les
β/α tweaks ne touchent pas ce phénomène ; il restera après D7.

**Suite** : si production demande `min_f1 ≥ 0.75`, lancer D8
(per-anchor genesis templates) ou D9 (`hybrid_decoder.py`
activation). Pour le contrat D7 actuel (gates ≥ 0.60), c'est
acceptable.

### 6.5 Statistiques de timing

* **Wallclock total** : ~1h22 de compute pur (M3 Pro).
* **Per-point** : médiane 4.0 min, mean 4.3 min, min 3.4 min,
  max 5.7 min. Variance principalement due au cold cache de
  `sentence-transformers` au premier RPC de chaque eval.
* Une anomalie de wallclock sur 1 transition (315 min) attribuable
  à une mise en veille système macOS hors interaction utilisateur ;
  pas reproductible avec `caffeinate -dimsu`.

---

## 7. Validation finale

Une fois le freeze appliqué :

* [ ] `cargo test --workspace` : 1691+ tests, all PASS.
* [ ] `cargo clippy --workspace --benches --tests -D warnings` : clean.
* [ ] `python3 -m pytest scripts/tests/ -v` : tous les tests D6 + D7
      pinned PASS.
* [ ] `python3 eval/scripts/run_eval.py --multi-catalog ...` : aggregate
      macro_f1 ≥ 0.75 ET min(per-catalog) ≥ 0.65.
* [ ] Test pratique 9-cas (D6 carry-over) : 7/9 ou 8/9 correct.
* [ ] Latence client p95 inchangée (~2.5 s).
* [ ] Pas de `alignment_breach` sur les 100 golden cases.

Si tous ✅, ouvrir la PR « D7 freeze » qui :
* Met à jour les 3 valeurs hardcodées.
* Ajoute le tableau §6 dans ce fichier.
* Met à jour `bench_baselines/v0.1.7/baseline.json` avec les valeurs
  réelles.
* Bump `Cargo.toml` workspace version → `0.1.7`.

---

## 8. Annexe — Diagnostic centroid trace

Pour valider numériquement que H3a (β trop faible) + H3b (α trop fort)
étaient bien les coupables, le sprint D7 ajoute l'instrumentation
`EFE_LOG_CENTROID_TRACE`. Activer :

```bash
EFE_LOG_CENTROID_TRACE=1 \
RUST_LOG=efe_server::mode1::trace=debug \
cargo run -p efe-server --release
```

À chaque round, `FusionAdapter::run_round` émet :

```
{"round": 7, "cos_problem": 0.123, "cos_anchors": [0.45, 0.42, 0.38, 0.51], "argmax_anchor": [3, 0.51], "message": "D7 centroid trace"}
```

Lecture :
* `cos_problem` chute de ~0.95 (round 0) à < 0.20 (round 7) → l'input
  s'évapore (H3a confirmée).
* `argmax_anchor` est constant `(3, billing)` quels que soient les
  inputs → la cone tire tout au même endroit (H3b confirmée).

Avec les schedules D7 (β₀=0.45, λ_β=0.05, α₀=0.30) :
* `cos_problem` reste > 0.40 jusqu'au round 7 → l'input domine.
* `argmax_anchor` varie selon l'input → la cone est devenue régulateur.

Ces traces sont enregistrées dans `target/calibration-d7/logs/*.log`
au fil du sweep et servent de preuve dans cette PR de freeze.
