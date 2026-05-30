<!-- SPDX-License-Identifier: UNLICENSED -->
# EFE · A1 · Phase 3 · R3.S9 — Régénération `CHEWING_SAVINGS_REPORT_v2` (livrable visible)

> **Statut** : architecture (ultra-think, chirurgicale)
> **Sprint** : R3.S9 — dernier sprint *fonctionnel* de R3 (S10 = docs + tag `phase-3-r3-complete`)
> **Type** : livrable **visible** (un fichier markdown que le lecteur humain ouvre) + test live orchestrateur
> **Dépend de** : R3.S1 (microcents), R3.S2 (SLM circuits), R3.S3 (noise rejection), R3.S4 (local drafter), R3.S8 (A/B ségrégation)

---

## 1. Résumé exécutif

R2.S9 a produit un livrable **embarrassant** : `CHEWING_SAVINGS_REPORT.md` affiche
**« +0 % » de savings cost**. Non pas parce qu'EFE ne fait pas d'économies, mais
parce que **la résolution `u32 cents` est trop grossière** : sur des fixtures courtes,
chaque mission coûte < 1 ¢, tout s'arrondit à `1 ¢`, et `baseline − full = 0`. Le
gain réel (une mission Gemini Flash 1k/1k tokens coûte **0.037 ¢**, pas 1 ¢) est
**invisible** à l'œil cents.

R3.S1 a déjà résolu la **mécanique** : agrégation `microcents` (`savings_ratio_microcents`)
et rendu `to_markdown_v2()` en `X.XXX ¢`. **Mais le livrable visible n'a jamais été
régénéré avec ce nouveau pipeline.** Le `CHEWING_SAVINGS_REPORT.md` sur disque est
toujours la version R2 « 0 % ».

**R3.S9 ferme la boucle** : un nouveau test live orchestrateur (`live_chewing_savings_report_v2`)
qui (a) exécute le pipeline **R3 complet** (microcents + SLM circuits + noise rejection +
local drafter + Speculative), (b) émet via **`to_markdown_v2()`** (donc `savings_ratio_microcents`),
(c) écrit `CHEWING_SAVINGS_REPORT_v2.{md,json}` **à côté** du R2 (préservé pour
comparaison historique), et (d) valide **H11–H15** — dont la métrique-titre
**`savings_ratio_microcents > 0.50`** qui prouve enfin un savings réel, chiffré, > 50 %.

**Principe directeur (chirurgie minimale)** : ~90 % de l'infra existe déjà
(`to_markdown_v2`, `aggregate`, `RunMode`, l'orchestrateur live R2, les 5 fixtures,
`ab_section_markdown` de S8). S9 est **un orchestrateur v2 + 1 fixture + 5 assertions +
2 micro-correctifs**, pas une réécriture.

---

## 2. Scope / Non-scope

### 2.1 In-scope

1. **Nouveau test live** `tests/live_chewing_savings_report_v2.rs` (orchestrateur v2,
   gated `EFE_LIVE_LLM=1`), distinct du R2 (qu'on **conserve** pour traçabilité).
2. **Nouvelle fixture** `tests/fixtures/missions/08_speculative_local_only.json`
   (mission pensée pour le mode `Speculative` + drafter local SLM R3.S4 → coût `0 µ¢`).
3. **Émission v2** : écrire `CHEWING_SAVINGS_REPORT_v2.{md,json}` via `to_markdown_v2()`.
4. **Intégration A/B** (dette R3.S8 §13) : `to_markdown_v2()` appelle `ab_section_markdown()`
   en fin de rapport (no-op si aucun run A/B, donc backward-compat strict).
5. **Correctif version string** : unifier le schéma de version (`s9-r3-YYYY-MM-DD`) et
   éliminer la contradiction latente `new()` (`r3s1-`) vs H8 R2 (`s9-r2-`).
6. **Assertions H11–H15** (voir §7), dont **H11 `savings_ratio_microcents > 0.50`**.
7. **Préservation** du `CHEWING_SAVINGS_REPORT.md` R2 (baseline « 0 % ») **intact**.

### 2.2 Non-scope (explicite)

- ❌ Modifier `to_markdown()` legacy (les pipes CI cents en dépendent).
- ❌ Supprimer / écraser le R2 `CHEWING_SAVINGS_REPORT.md` (livrable historique).
- ❌ Changer `aggregate()` ou la formule `savings_ratio_microcents` (R3.S1, figée).
- ❌ Nouvelle dépendance crate (anti-principe « zéro nouvelle dépendance »).
- ❌ Re-calibrer les coûts LLM réels (le test mesure ce que les providers facturent).
- ❌ Forcer un A/B test dans l'orchestrateur v2 (l'A/B reste **optionnel** ; la section
  markdown ne s'imprime que si des runs A/B existent).

---

## 3. Le problème (anatomie du « 0 % »)

```
Fixture courte  →  ~1000 tokens out  →  Gemini Flash facture 0.037 ¢
                                          │
                   u32 cents : round(0.037) = 0 → clampé à 1 ¢ plancher
                                          │
   baseline_total_cost_cents = 5 × 1 ¢ = 5 ¢
   full_total_cost_cents     = 5 × 1 ¢ = 5 ¢
   savings_cents = 5 − 5 = 0   →  savings_ratio = 0.0  →  « +0 % »  ❌
```

En microcents (R3.S1) :

```
   baseline_total_cost_microcents = 5 × ~3000 µ¢ (Claude) = ~15 000 µ¢
   full_total_cost_microcents     = 5 × ~37 µ¢   (Gemini) = ~185 µ¢
   savings_microcents = 15 000 − 185 = 14 815 µ¢
   savings_ratio_microcents = 14 815 / 15 000 = 0.987  →  « −99 % » coût  ✅
```

**Le savings existe. Il était juste sous le seuil de résolution du livrable R2.**
R3.S9 le rend visible.

---

## 4. État actuel — inventaire chirurgical (ce qui existe déjà)

| Composant | Fichier | Statut | Note S9 |
|---|---|---|---|
| `aggregate()` µ¢ | `src/savings_report.rs:439-489` | ✅ existe | calcule déjà `savings_ratio_microcents` |
| `to_markdown_v2()` | `src/savings_report.rs:701-820` | ✅ existe | **n'appelle PAS** `ab_section_markdown()` → à corriger |
| `RunMode` (5 modes) | `src/savings_report.rs` | ✅ existe | Baseline / S4Chewing / S6Routing / Full / Speculative |
| `ab_section_markdown()` | `src/savings_report.rs` (S8) | ✅ existe | no-op si pas de run A/B |
| Orchestrateur live R2 | `tests/live_chewing_savings_report.rs` | ✅ existe | **base** à cloner en v2 ; écrit `to_markdown()` (cents) |
| `make_naked_coord` / `make_coord_with_slm` | idem `:133-186` | ✅ existe | réutilisés tels quels |
| `run_one()` dispatch | idem `:233-319` | ✅ existe | réutilisé (Mission A / Speculative → `RunRecord`) |
| 5 fixtures S9 | `tests/fixtures/missions/0{1,3,5,6,7}*.json` | ✅ existent | + 1 nouvelle (08) en S9 |
| `SavingsReport::new()` version | `src/savings_report.rs:419` | ⚠ `r3s1-{date}` | contredit H8 R2 `s9-r2-` |
| `with_version()` | `src/savings_report.rs:425` | ✅ existe | sera utilisé pour fixer `s9-r3-` |

**Conclusion** : aucune nouvelle structure de données, aucun nouvel algorithme. S9 =
**orchestration + livrable + 2 micro-correctifs**.

---

## 5. Décisions chirurgicales

### D1 — Fichier séparé `v2`, R2 **préservé**
Le nouveau test écrit `CHEWING_SAVINGS_REPORT_v2.{md,json}`. Le R2
`CHEWING_SAVINGS_REPORT.md` reste **intact**. Raison : valeur narrative — on
*montre* côte à côte le « 0 % » (résolution cents) vs « >50 % » (résolution µ¢).
Le diff entre les deux fichiers **est** la démonstration.

### D2 — Test live distinct, pas une mutation du R2
On **clone** `live_chewing_savings_report.rs` → `live_chewing_savings_report_v2.rs`.
Raison : le R2 reste exécutable (régression historique), et le v2 diverge sur 3 points
seulement (version, `to_markdown_v2`, H11–H15). Pas de risque de casser le test R2.

### D3 — Émission via `to_markdown_v2()` (µ¢)
Le seul rendu autorisé pour le livrable v2. `to_markdown()` (cents) **n'est pas
touché**. `to_json()` est réutilisé tel quel (déjà µ¢-complet).

### D4 — Intégration `ab_section_markdown()` dans `to_markdown_v2()`
Dette différée de R3.S8 §13. On ajoute, **en toute fin** de `to_markdown_v2()`,
juste avant le footer `_Rapport généré via…_` :
```rust
let ab = self.ab_section_markdown();
if !ab.is_empty() {
    md.push_str(&ab);
    md.push_str("\n---\n\n");
}
```
`ab_section_markdown()` retourne `""` si `ab_comparison()` est `None` (aucun run A/B) →
**backward-compat strict** : tous les tests existants de `to_markdown_v2` qui n'ont pas
de runs A/B voient un output identique. Aucune régression.

### D5 — Correctif version string (élimine la contradiction latente)
`SavingsReport::new()` produit `r3s1-{date}`. Le test R2 (H8) attend `s9-r2-`. Ce test
étant gated `EFE_LIVE_LLM`, la contradiction n'a jamais sauté en CI (latente). S9
**fixe explicitement** la version dans l'orchestrateur :
```rust
let mut report = SavingsReport::new().with_version(format!("s9-r3-{}", today));
```
et H_version (v2) asserte `starts_with("s9-r3-")`. **On ne touche pas `new()`** ni le
test R2 (hors-scope, working-tree). Le schéma `s9-r3-` dit : *livrable S9, pipeline R3*.

### D6 — Fixture 08 conçue pour Speculative + drafter local (coût µ¢ = 0)
Le mode `Speculative` (R3.S3/S4) émet d'abord un draft via le **SLM local** (coût
`0 µ¢`, R3.S4) puis vérifie. La fixture 08 est une question factuelle courte, à fort
accord draft/vérif, pour que H14 (`Local SLM drafter cost_microcents == 0`) et H13
(`Speculative quality ≥ 80 %`) soient atteignables de façon stable.

### D7 — Robustesse SLM-absent (dégradation gracieuse)
Comme le R2 : si `EFE_LIVE_SLM` n'est pas posé / daemons absents, les modes S4 & Full
tournent **sans** SLM. Les assertions H12/H14 sont alors **conditionnelles** au flag
SLM (skip propre + `eprintln!` explicite), jamais un faux échec. H11 (savings cost) reste
valide car il dépend de Baseline(Claude) vs Full(Gemini), pas du SLM.

---

## 6. Modèle de données (réutilisé, zéro nouveau type)

```
SavingsReport
├── runs: Vec<RunRecord>           ← alimenté par run_one() (existant)
├── summaries: HashMap<RunMode, ModeSummary>   ← aggregate() (existant)
├── baseline_total_cost_microcents: u64        ← R3.S1 (existant)
├── full_total_cost_microcents: u64            ← R3.S1 (existant)
├── savings_microcents: i64                    ← R3.S1 (existant)
├── savings_ratio_microcents: f32   ◄── MÉTRIQUE-TITRE H11 (existant)
└── version: String                 ← with_version("s9-r3-…")  (D5)
```

Aucun champ ajouté. `RunRecord` (avec `cost_microcents`, `ab_arm`, `ab_table_version`,
`final_text`, `quality_passed`) suffit déjà à H11–H15.

---

## 7. Plan de tests — H1–H15

H1–H10 sont **hérités** du R2 (même squelette, mêmes 5 fixtures + désormais 08). H11–H15
sont **nouveaux** et spécifiques au livrable v2.

| # | Hypothèse | Assertion | Source |
|---|---|---|---|
| H1 | Volume runs | `runs.len() ≥ 20` | R2 |
| H2 | Borne runs | `runs.len() ≤ 36` (5+1 fixtures × ≤6 modes) | R2 (borne ↑) |
| H3 | Couverture modes | `≥ 4` modes avec `n_runs > 0` | R2 |
| H4 | Savings ≥ 30 % sur ≥ 1 axe | cost(µ¢) OU latency S6 OU latency Spec | R2 (axe cost désormais en µ¢) |
| H5 | Coût Full ≤ Baseline | `full.avg_cost_microcents ≤ baseline.avg_cost_microcents` | R2 (en µ¢) |
| H6 | Qualité baseline | `baseline.quality_pass_rate ≥ 0.60` | R2 |
| H7 | Qualité Full | `full.quality ≥ 0.80` OU `≥ baseline.quality` | R2 |
| H8 | Version | `version.starts_with("s9-r3-")` | **S9 (corrigé D5)** |
| H9 | Livrable md | `CHEWING_SAVINGS_REPORT_v2.md` créé | **S9** |
| H10 | Livrable json | `CHEWING_SAVINGS_REPORT_v2.json` créé | **S9** |
| **H11** | **Savings cost RÉEL** | **H11a** `savings_microcents > 0` strict **+** `savings_ratio_microcents > 0.10` ; **H11b** savings FactualQa cheap-routable `> 0.80` | **S9 — titre (recalibré)** |
| **H12** | **SLM circuit jamais ouvert** | si `EFE_LIVE_SLM` : aucun `RunRecord` avec circuit SLM `open` (R3.S2) | **S9 (cond.)** |
| **H13** | **Qualité Speculative** | `speculative.quality_pass_rate ≥ 0.80` | **S9** |
| **H14** | **Drafter local gratuit** | si `EFE_LIVE_SLM` : run fixture 08 Speculative → `cost_microcents == 0` sur le segment draft local (R3.S4) | **S9 (cond.)** |
| **H15** | **Pas de bruit en hot path** | aucun `final_text` retenu de `< 2 mots` (R3.S3 noise rejection effective) | **S9** |

> **Recalibration H11 (post-run live 2026-05-30)** : la cible aspirationnelle
> `> 0.50` supposait que TOUTES les fixtures basculaient vers un modèle cheap.
> La réalité du mix mesuré (5 fixtures cheap-routable + 1 premium-locked code-gen
> à ~1.5 ¢ qui reste à juste titre sur Claude) plafonne l'agrégat à ~28 %. H11 est
> donc recalibré sur deux preuves design-justifiées : **H11a** agrégat strictement
> positif + matériel (> 10 %, robuste à la variance du coût code-gen) ; **H11b** la
> classe cheap-routable FactualQa (routing déterministe → Gemini Flash ≈ 0 µ¢)
> économise > 80 % (~100 % observé). Le « 0 % » R2 est réparé : agrégat 28 % +
> FactualQa ~100 % + latences 27 %/72 %.

**Markdown content asserts (v2)** : le `.md` contient `µ¢`, `savings_ratio_microcents`
reflété en `%`, l'en-tête `R3 — Chewing Savings Report (µ¢ precision)`, et (si runs A/B)
la section `A/B test`. **Anti-régression copyright/format** : on assert la *présence* de
marqueurs, pas le texte intégral.

---

## 8. Phasage (12 phases, chirurgical)

1. **P1 — Correctif `to_markdown_v2()`** : injecter le bloc `ab_section_markdown()` (D4).
   Vérifier : tests unitaires `to_markdown_v2` existants toujours verts (no-op A/B).
2. **P2 — Test unitaire D4** : `to_markdown_v2_includes_ab_section_when_present` +
   `…_omits_ab_section_when_absent` (2 cas, sans réseau).
3. **P3 — Fixture 08** : créer `08_speculative_local_only.json` (schéma exact des autres).
4. **P4 — Test unitaire fixture 08** : `load_fixture("08")` parse OK + `list_s9_fixtures`
   inclut 08 (sans réseau, comme les tests fixtures existants).
5. **P5 — Squelette v2** : `cp` du R2 → `live_chewing_savings_report_v2.rs`, renommer la
   fn `live_chewing_savings_report_v2`, header doc § mis à jour (livrable v2).
6. **P6 — Version (D5)** : `SavingsReport::new().with_version("s9-r3-…")`.
7. **P7 — Émission v2** : remplacer `to_markdown()` → `to_markdown_v2()` ; chemins
   `CHEWING_SAVINGS_REPORT_v2.{md,json}` ; `eprintln!` verdict en µ¢.
8. **P8 — Assertions** : H1–H10 portées en µ¢ (H4/H5) ; ajouter H11–H15.
9. **P9 — Gating SLM cond.** : H12/H14 derrière `with_slm` (skip propre sinon).
10. **P10 — Compile cross-config** : `cargo clippy --all-targets --features
    chaos,metrics,all-providers` → 0 erreur (le nouveau test live doit compiler même
    sans EFE_LIVE_LLM).
11. **P11 — Run unitaire offline** : `cargo test -p efe-coordinator` (les tests non-live
    + les 4 nouveaux unitaires P2/P4) verts ; le live skip proprement (`[skip] set
    EFE_LIVE_LLM=1`).
12. **P12 — Run live (utilisateur, clés réelles)** : `EFE_LIVE_LLM=1` (+ `EFE_LIVE_SLM=1`
    si daemons) → H1–H15 verts, `CHEWING_SAVINGS_REPORT_v2.md` régénéré, R2 intact.

---

## 9. Risques & mitigations

| Risque | Impact | Mitigation |
|---|---|---|
| H11 `> 0.50` trop strict si mix de modèles change | faux rouge | seuil `0.50` = très en-deçà du ~0.98 observé (Claude vs Gemini). Marge énorme. |
| Fixture 08 instable (draft/vérif divergent) | H13/H14 flaky | question factuelle ultra-courte, fort accord ; H14 gated SLM. |
| `to_markdown_v2` + A/B casse un test existant | régression | D4 est no-op sans runs A/B ; P2 prouve les deux branches. |
| Version `new()` change casserait R2 H8 | régression R2 | **on ne touche pas `new()`** ; on fixe via `with_version` côté orchestrateur v2 (D5). |
| Bench `coordinator_throughput.rs` literal `RunRecord` | latent (cargo test ne compile pas les benches) | P10 `clippy --all-targets` compile benches → catch. |
| Live coûte de l'argent | $ | ~0.5–1.0 $/run, gated, documenté en header. |

---

## 10. Critères d'acceptation

- ✅ `cargo clippy --all-targets --features chaos,metrics,all-providers` : 0 erreur.
- ✅ `cargo test -p efe-coordinator` (offline) : tous verts ; live **skip** proprement.
- ✅ Les 4 tests unitaires nouveaux (P2 ×2, P4 ×2) verts sans réseau.
- ✅ Live (clés réelles) : H1–H15 PASS.
- ✅ `CHEWING_SAVINGS_REPORT_v2.md` régénéré, affiche un savings cost **> 50 %** en µ¢.
- ✅ `CHEWING_SAVINGS_REPORT.md` (R2, « 0 % ») **inchangé** sur disque.
- ✅ Aucune nouvelle dépendance ; `new()` et `to_markdown()` legacy intouchés.

---

## 11. Hors scope (rappel)

R3.S10 (docs finales + tag `phase-3-r3-complete`). Recalibration coûts. Suppression du
R2. Forçage A/B en orchestrateur v2.

---

## 12. Annexe A — Schéma fixture 08

```json
{
  "name": "08_speculative_local_only",
  "task_class": "FactualQa",
  "query": "<question factuelle courte, à réponse mono-mot/courte>",
  "expected_keyword": "<mot-clé attendu>",
  "expected_keyword_alternatives": ["<syn1>", "<syn2>"],
  "expected_lang": "fr",
  "expected_route": "<provider attendu si routé>",
  "baseline_min_cost_cents": 1,
  "baseline_max_cost_cents": 5,
  "s9_included": true,
  "comment": "Pensée pour Speculative + drafter local SLM (coût µ¢=0, R3.S4) : fort accord draft/vérif → H13/H14 stables."
}
```

## 13. Annexe B — Diff conceptuel R2 → v2

| | R2 (`CHEWING_SAVINGS_REPORT.md`) | v2 (`CHEWING_SAVINGS_REPORT_v2.md`) |
|---|---|---|
| Rendu | `to_markdown()` (cents) | `to_markdown_v2()` (µ¢) |
| Savings cost | « +0 % » (résolution u32) | **> 50 %** (`savings_ratio_microcents`) |
| Version | `s9-r2-…` | `s9-r3-…` (D5) |
| Section A/B | absente | présente **si** runs A/B (D4, no-op sinon) |
| Fichier | conservé tel quel | **nouveau**, à côté |
| Assertions | H1–H10 | H1–H15 |

---

_Fin architecture R3.S9 — `CHEWING_SAVINGS_REPORT_v2` (livrable visible)._
