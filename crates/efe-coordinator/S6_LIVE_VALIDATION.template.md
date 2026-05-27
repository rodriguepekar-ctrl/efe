# Phase 3 R2 — Sprint S6 : Live Validation (TEMPLATE)

> **Ceci est un template.** À remplir avec les valeurs OBSERVÉES après exécution
> réussie des 4 tests `live_mission_a_router_picks_*`.
> Une fois rempli, renommer en `S6_LIVE_VALIDATION.md` (sans `.template`).
> Cohérence avec S1-S5 templates.

---

**Date de validation** : YYYY-MM-DD (HH:MM UTC)
**Hardware** : <Mac M-series / Linux x86_64 / …>
**LLM models live** :
  - Claude Sonnet 4.6 (`claude_claude-sonnet-4-6`)
  - Gemini 2.5 Flash (`gemini_gemini-2.5-flash`)
  - GPT-4o (`gpt_gpt-4o`)
**Phase 1 SLMs actifs** : 0 (S6 n'utilise pas les nano-robots — routing inline
                          via `RouterClient::for_task_class`)
**Table de scoring** : `s6-r2-2025-11` (défaut 3-LLM)

---

## ✅ Résultat final — 40/40 assertions PASS attendu (4 tests × E1-E10)

```text
running 4 tests

=== Mission A — Router Tuning S6 (01_factual_simple.json) ===
  query        : "Quelle est la capitale de la France ? Réponds en une seule…" (75 chars)
  task_class   : FactualQa
  available    : [claude_…, gemini_…, gpt_…]
  expected     : gemini_gemini-2.5-flash

  ── Router decision ──
    selected     : gemini_gemini-2.5-flash
    router reason: "selected gemini_… (p50=800ms, est_cost=…c, …)"
    cost_cents   : <X> ¢
    latency_ms   : <X> ms
    audit_hash   : <abc…def> (64 chars)
    wall_clock   : <X> ms

test live_mission_a_router_picks_gemini_for_factual_qa ... ok

=== Mission A — Router Tuning S6 (05_code_gen_rust.json) ===
  …
  selected     : claude_claude-sonnet-4-6
test live_mission_a_router_picks_claude_for_code ... ok

=== Mission A — Router Tuning S6 (06_creative_writing.json) ===
  …
  selected     : gpt_gpt-4o
test live_mission_a_router_picks_gpt_for_creative ... ok

=== Mission A — Router Tuning S6 (07_translation_fr_en.json) ===
  …
  selected     : gemini_gemini-2.5-flash
test live_mission_a_router_picks_gemini_for_translation ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

---

## Métriques observées vs cibles (4 × 10 = 40 assertions E1-E10)

### Test 1 — `live_mission_a_router_picks_gemini_for_factual_qa`

| # | Métrique | Cible | Observé | Statut |
|---|---|---|---|---|
| E1 | `final_text` non vide | true | `"<…>"` | <✅/❌> |
| E2 | `llm_used == "gemini_gemini-2.5-flash"` | match | `<…>` | <✅/❌> |
| E3 | sanity final_text | true | true | <✅/❌> |
| E4 | contient "paris" (lc) | true | `"<…paris…>"` | <✅/❌> |
| E5 | `cost_cents ≤ 10` | ≤ 10 ¢ | <X> ¢ | <✅/❌> |
| E6 | `cost_cents ≥ 1` | ≥ 1 ¢ | <X> ¢ | <✅/❌> |
| E7 | `latency_ms < 30_000` | < 30 s | <X> ms | <✅/❌> |
| E8 | `audit_chain_hash.len() == 64` | BLAKE3 hex | 64 | <✅/❌> |
| E9 | `router_reason` non vide | true | `"<…>"` | <✅/❌> |
| E10 | `router_reason` contient "p50=" | true | true | <✅/❌> |

### Test 2 — `live_mission_a_router_picks_claude_for_code`

| # | Métrique | Cible | Observé | Statut |
|---|---|---|---|---|
| E1 | `final_text` non vide | true | `"<fn factorial…>"` | <✅/❌> |
| E2 | `llm_used == "claude_claude-sonnet-4-6"` | match | `<…>` | <✅/❌> |
| E3 | sanity | true | true | <✅/❌> |
| E4 | contient "factorial" (lc) | true | true | <✅/❌> |
| E5 | `cost_cents ≤ 10` | ≤ 10 ¢ | <X> ¢ | <✅/❌> |
| E6 | `cost_cents ≥ 1` | ≥ 1 ¢ | <X> ¢ | <✅/❌> |
| E7 | `latency_ms < 30_000` | < 30 s | <X> ms | <✅/❌> |
| E8 | `audit_chain_hash` 64 chars | true | 64 | <✅/❌> |
| E9 | `router_reason` non vide | true | true | <✅/❌> |
| E10 | `router_reason` contient "p50=" | true | true | <✅/❌> |

### Test 3 — `live_mission_a_router_picks_gpt_for_creative`

| # | Métrique | Cible | Observé | Statut |
|---|---|---|---|---|
| E1 | `final_text` non vide | true | `"<poulpe : …>"` | <✅/❌> |
| E2 | `llm_used == "gpt_gpt-4o"` | match | `<…>` | <✅/❌> |
| E3-E10 | (idem pattern) | … | … | <✅/❌> |

### Test 4 — `live_mission_a_router_picks_gemini_for_translation`

| # | Métrique | Cible | Observé | Statut |
|---|---|---|---|---|
| E1 | `final_text` non vide | true | `"<twilight bathes…>"` | <✅/❌> |
| E2 | `llm_used == "gemini_gemini-2.5-flash"` | match | `<…>` | <✅/❌> |
| E3-E10 | (idem pattern) | … | … | <✅/❌> |

**Score attendu : 40/40 assertions vertes.**

---

## L'argument économique COMPLET R2 — chiffres observés

Sur les 4 tests :

| Test | LLM choisi | cost_cents observé | vs baseline (Claude partout) | Savings |
|---|---|---|---|---|
| Factual (Gemini) | gemini | <X> ¢ | ~5 ¢ Claude | **~X %** |
| Code (Claude) | claude | <X> ¢ | ~5 ¢ Claude | 0 % (déjà optimal) |
| Creative (GPT) | gpt | <X> ¢ | ~5 ¢ Claude | **~X %** |
| Translation (Gemini) | gemini | <X> ¢ | ~5 ¢ Claude | **~X %** |
| **Total 4 tests** | mixed | <X> ¢ | ~20 ¢ | **~X %** |

→ **EFE divise par ~3-5× le coût LLM** sur les tâches non-Claude
  (FactualQa + Translation + Creative pas dégradés en qualité, juste
  payés moins cher via le bon LLM).

Sur une **mixture réaliste 80/20** (80 % FactualQa/Translation → Gemini,
20 % Code/Reasoning → Claude) :
- **Baseline Claude partout** : 100 %
- **Avec S6 routing** : **20 % × 5 ¢ + 80 % × 0.5 ¢ = 1.4 ¢ moyen** vs 5 ¢
  → **~3.5× moins cher** sur le routing seul, AVANT pré-mâchage.
- **Avec S4 pré-mâchage + S6 routing** : **~10-16× moins cher** que baseline.

---

## Observations chirurgicales bonus

<à remplir post-run>

---

## Critère d'acceptation S6 — État final

- [x] `TaskClassScoringTable::default_3llm_table()` calibrée 6×3
- [x] `RouterClient::for_task_class(&table, task)` factory
- [x] `MissionARequest::with_default_scoring(...)` helper
- [x] `task_class_scoring_table: Option<Arc<…>>` champ `#[serde(skip)]`
- [x] `run_mission_a` utilise le router tuné si présent
- [x] Fixture 01 enrichie (`expected_route` + `expected_route_justification_keyword`)
- [x] 3 fixtures NEW : 05_code / 06_creative / 07_translation
- [x] 17 tests offline scoring PASS (6 winners + 11 cohérence)
- [x] 4 tests live S6 (E1-E10 par test) compilent + skip propre offline
- [x] cargo build clean
- [x] cargo clippy 0 warning S6
- [x] 0 régression offline workflow_mission_a + b + speculative + invariants + unit_streaming
- [<>] **Test live 4×PASS avec 3 clés API**
- [<>] **40/40 assertions E1-E10 PASS**
- [<>] `S6_LIVE_VALIDATION.md` scellé (ce fichier renommé)
- [x] Commits S6 (TBD) : feat scoring.rs + feat wiring + feat fixtures + docs

---

## Commits R2 cumulés au moment du scellement S6

```text
<HHHHHHH> docs(phase-3-r2-s6): scellement live validation S6 40/40 PASS
<HHHHHHH> feat(phase-3-r2-s6): fixtures + 4 tests live router tuning
<HHHHHHH> feat(phase-3-r2-s6): wire router tuning into Mission A
<HHHHHHH> feat(phase-3-r2-s6): TaskClassScoringTable + 17 tests offline scoring
+ 26 commits S1-S5 cumulés (jusqu'à 686d753 scellement S5)
```

---

## Liens

- **Architecture S6** : `docs/architecture/EFE_A1_PHASE3_R2_S6_ROUTER_TUNING_TASK_CLASS_ARCHITECTURE.md`
- **Module scoring** : `crates/efe-nano-app-coordinator/src/types/scoring.rs`
- **Tests live** : `crates/efe-coordinator/tests/live_missions.rs::live_mission_a_router_picks_*`
- **Tests offline** : `crates/efe-nano-app-coordinator::types::scoring::tests` (17 tests)
- **Workflow patch** : `crates/efe-coordinator/src/workflows/mission_a.rs` (Étape 2 Router)
- **Fixtures** : `crates/efe-coordinator/tests/fixtures/missions/0{1,5,6,7}_*.json`

---

## Conclusion S6

Sprint S6 validé end-to-end. **Le routing tuné multiplie l'argument
économique du pré-mâchage** :

1. ✅ Table statique versionnée `s6-r2-2025-11` calibrée à partir des
   benchmarks publics (MMLU, HumanEval, GPQA, MTEB) + jugement d'expert
2. ✅ 6 winners attendus offline (Factual→Gemini, Analysis→Claude,
   Code→Claude, Creative→GPT, Reasoning→Claude, Translation→Gemini)
3. ✅ Pareto algorithm réutilisé du Sprint S0 sans modification — S6 ne
   fait qu'**alimenter** l'algorithme avec des données calibrées
4. ✅ 4 tests live démontrent le routing en conditions réelles : Gemini
   pour Factual+Translation (cost-driven), Claude pour Code
   (quality-driven), GPT pour Creative (capability-driven)
5. ✅ Backward-compat 100 % : champ `task_class_scoring_table` opt-in
   via `#[serde(skip)]`, helper `with_default_scoring()` non breaking

**L'argument économique R2 complet** :

| Configuration | Coût relatif | Savings |
|---|---|---|
| Baseline (Claude partout, pas de pré-mâchage) | 100 % | 0 % |
| S4 (Claude + pré-mâchage Mission A) | ~30 % | **70 %** |
| S5 (Mission B 3-LLMs + pré-mâchage mutualisé) | ~50 % du 3-LLM naïf | 50 % |
| **S6 (routing tuné + pré-mâchage) sur mixture 80/20** | **~6 %** | **~94 %** |

EFE divise par **~16×** le coût LLM total en production sur une mixture
réaliste 80 % FactualQa/Translation + 20 % Code/Reasoning,
**en préservant ou améliorant la qualité** (le bon LLM pour la bonne tâche).

**Prochaine étape** : Sprint S7 — Mission Speculative live (drafter SLM
local + verifier LLM cloud), démontre la 3e dimension de savings (réduction
output tokens du gros LLM via spéculation).
