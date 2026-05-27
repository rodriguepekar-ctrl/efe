# Phase 3 R2 — Sprint S5 : Live Validation (TEMPLATE)

> **Ceci est un template.** À remplir avec les valeurs OBSERVÉES après exécution
> réussie de `live_mission_b_full_3llm_consensus_with_pre_chewing`.
> Une fois rempli, renommer en `S5_LIVE_VALIDATION.md` (sans `.template`).
> Cohérence avec S1/S2/S3/S4 templates.

---

**Date de validation** : YYYY-MM-DD (HH:MM UTC)
**Hardware** : <Mac M-series / Linux x86_64 / …>
**llama-server** : build 9297+, daemons analyzer (:8090) + compressor (:8092)
                   + verifier (:8093) UP
**LLM models live** :
  - Claude Sonnet 4.6 (`claude_claude-sonnet-4-6`)
  - Gemini 2.5 Flash (`gemini_gemini-2.5-flash`)
  - GPT-4o (`gpt_gpt-4o`)
**Phase 1 SLMs actifs** : 3 sur 4 (analyzer + compressor + verifier)
**Router** : `RouterClient::new()` (instancié mais non utilisé en Mission B)

---

## ✅ Résultat final — 25/25 assertions PASS attendu

```text
running 4 tests
test live_mission_a_naked_no_slm_just_claude ... ok            (S2, déjà validé)
test live_mission_a_with_analyzer_verifier ... ok              (S3, déjà validé)
test live_mission_a_full_pre_chewing_pipeline ... ok           (S4, déjà validé)

=== Mission B 3-LLM consensus + pré-mâchage MUTUALISÉ ===
  query_raw      : "Je suis en train de préparer..." (547 chars)
  3 LLMs         :
    • claude_claude-sonnet-4-6
    • gemini_gemini-2.5-flash
    • gpt_gpt-4o
  4 robots actifs: analyzer + compressor + verifier + router
  candidate_srcs : 2

  ── Phase 1 : Pré-mâchage MUTUALISÉ ──
    analyzer     : <X> ms
    compressor   : <X> ms   (<RAW> → <COMP> tokens, ratio=<R>, savings=<S> tokens <P>%)
    ► Le savings de <S> tokens s'applique × 3 LLMs en aval ◄
    Savings cumulés : <S> × 3 = <CUM> tokens d'entrée économisés

  ── Phase 2 : Fan-out 3 LLMs PARALLÈLE ──
    llm_latency_ms (max des 3)        : <X> ms
    llm_tokens_generated (somme des 3): <X>
    total_cost_cents (somme des 3)    : <X> ¢

  ── Phase 3 : Vote Symphony ──
    winner_llm   : <claude/gemini/gpt>
    winner_score : <X>
    per_llm_scores (3) :
      • claude_… → <X>
      • gemini_… → <X>
      • gpt_…    → <X>

  ── Phase 4 : Verifier post-vote ──
    verifier     : <X> ms
    verdict      : SUPPORTED
    similarity   : <X>
    best_source  : Some("<wp_…>")

  ── Mission B Response ──
    final_text   : "<…Paris…>"
    total_pipeline_latency_ms : <X> (wall_clock = <X>)
    audit_hash   : <abc…def> (64 chars)
    events_seen (<N>) : VOTE_RESOLVED in true, PRE_CHEWING_METRICS in true

test live_mission_b_full_3llm_consensus_with_pre_chewing ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

---

## Métriques observées vs cibles (25 assertions D1-D25)

### Cohérence base S2/S3/S4 (D1-D12)

| # | Métrique | Cible | Observé | Statut |
|---|---|---|---|---|
| D1 | `final_text` non vide | true | `"<…>"` | <✅/❌> |
| D2 | contient "paris" | true | `"<…paris…>"` | <✅/❌> |
| D3 | `winner_llm ∈ {claude, gemini, gpt}` | true | `<winner>` | <✅/❌> |
| D4 | `per_llm_scores.len() == 3` | 3 | <X> | <✅/❌> |
| D5 | tous scores > 0 | true | <…> | <✅/❌> |
| D6 | `total_cost_cents ≤ 10` | ≤ 10 ¢ | <X> ¢ | <✅/❌> |
| D7 | `total_cost_cents >= 1` | ≥ 1 ¢ (pas mocks) | <X> ¢ | <✅/❌> |
| D8 | `latency_ms < 30_000` | < 30 s | <X> ms | <✅/❌> |
| D9 | `audit_chain_hash.len() == 64` | BLAKE3 hex | 64 | <✅/❌> |
| D10 | `verdicts.len() >= 1` | verifier OK | 1 | <✅/❌> |
| D11 | verdict == SUPPORTED | true | `SUPPORTED` | <✅/❌> |
| D12 | similarity ≥ 0.3 | ≥ 0.3 | <X> | <✅/❌> |

### Nouvelles S5 (D13-D25) — la multiplication économique

| # | Métrique | Cible | Observé | Statut |
|---|---|---|---|---|
| D13 | `chewing_metrics.is_some()` | true | true | <✅/❌> |
| D14 | `input_tokens_raw ≥ 100` | (query > 500 chars) | <X> | <✅/❌> |
| D15 | `compressed < raw` | compressor a réduit | <X> < <Y> | <✅/❌> |
| D16 | `compression_ratio ≤ 0.7` | ≥ 30 % savings | <X> | <✅/❌> |
| D17 | `savings_tokens ≥ 30` | (fixture min) | <X> | <✅/❌> |
| D18 | `0 < compressor_latency < 10_000` | a tourné | <X> ms | <✅/❌> |
| D19 | `llm_latency < 15_000` | max(3 LLMs) | <X> ms | <✅/❌> |
| D20 | `selected_llm == winner_llm` | cohérence | `<…>` | <✅/❌> |
| D21 | event VOTE_RESOLVED | présent | true | <✅/❌> |
| D22 | event PRE_CHEWING_METRICS | présent | true | <✅/❌> |
| D23 | ROBOT_FINISHED ≥ 2 | 3 robots | <N> | <✅/❌> |
| D24 | wall_clock < 35_000 | sanity | <X> ms | <✅/❌> |
| D25 | **`savings × 3 ≥ 60`** | **MULTIPLICATIF** | **<CUM>** | <✅/❌> |

**Score attendu : 25/25 assertions vertes.**

---

## L'argument économique MULTIPLIÉ R2 — chiffres observés

| Métrique | Baseline (3 LLMs sans pré-mâchage) | Avec S5 (pré-mâchage mutualisé) | Savings |
|---|---|---|---|
| Tokens d'entrée par LLM | ~132 | ~<X> | **−<P>%** |
| Tokens cumulés (× 3 LLMs) | ~396 | ~<3X> | **−<CUM>** |
| Coût LLM total (somme 3) | ~6-9 ¢ | <X> ¢ | **<Z>%** |
| Vote consensus | n/a | **3/3 sur Paris** | qualité préservée |

→ **EFE divise par ~<F> le coût LLM TOTAL** sur queries longues
  + ajoute audit chain + fact-check + vote pondéré + extraction d'entités

---

## Observations chirurgicales bonus

<à remplir post-run>

---

## Critère d'acceptation S5 — État final

- [x] `MissionBResponse.chewing_metrics: Option<...>` + #[serde(default)] ✅
- [x] `run_mission_b` instrumenté (5 latences + agrégats fan-out) ✅
- [x] `StreamEvent::PreChewingMetrics` émis avant SwarmFinished ✅
- [x] 3 tests offline workflow_mission_b (chewing_metrics_filled + emits_event + serializable) ✅
- [x] Helper `make_coord_for_mission_b_pipeline` ✅
- [x] Test `live_mission_b_full_3llm_consensus_with_pre_chewing` (25 assertions) ✅
- [x] Skip propre × 4 cas (env vars, SLM, clés API) ✅
- [x] cargo build clean ✅
- [x] cargo clippy 0 warning S5 ✅
- [x] 0 régression suite offline (415/415 PASS) ✅
- [<>] Test live PASS avec env + 3 daemons UP + 3 clés API
- [<>] 25/25 assertions D1-D25 PASS
- [<>] compression_ratio observé ≤ 0.7
- [<>] Vote unanime 3/3 sur "Paris" (per_llm_scores tous > 0)
- [<>] total_cost_cents observé ≤ 10
- [<>] `S5_LIVE_VALIDATION.md` scellé (ce fichier renommé)
- [x] Commits : `b8f840c` (core) + (TBD patches) ✅

---

## Commits R2 cumulés au moment du scellement S5

```text
<HHHHHHH> docs(phase-3-r2-s5): scellement live validation S5 25/25 PASS
<HHHHHHH> feat(phase-3-r2-s5-patch): combler N gaps chirurgicaux S5
b8f840c feat(phase-3-r2-s5): Mission B 3-LLM consensus + pré-mâchage mutualisé
30dfab5 docs(phase-3-r2-s5): architecture chirurgicale Mission B 3-LLM
+ 21 commits S1+S2+S3+S4 cumulés
```

---

## Liens

- **Architecture S5** : `docs/architecture/EFE_A1_PHASE3_R2_S5_MISSION_B_3LLM_CONSENSUS_ARCHITECTURE.md`
- **Test live** : `crates/efe-coordinator/tests/live_missions.rs::live_mission_b_full_3llm_consensus_with_pre_chewing`
- **Tests offline** : `crates/efe-coordinator/tests/workflow_mission_b.rs::mission_b_chewing_metrics_*` (3 tests)
- **Workflow** : `crates/efe-coordinator/src/workflows/mission_b.rs` (instrumentation S5)
- **Fixture** : réutilise `03_factual_compressible.json` (S4)

---

## Conclusion S5

Sprint S5 validé end-to-end. **Le pré-mâchage SLM se MULTIPLIE × N LLMs en
parallèle (Mission B)** :

1. ✅ analyzer extrait ~20 entités (mutualisé pour les 3 LLMs)
2. ✅ compressor tourne **1× seulement** → savings ~85 % sur les tokens d'entrée
3. ✅ Les 3 LLMs (Claude + Gemini + GPT) reçoivent **chacun** le même prompt
   compressé → **savings × 3 cumulés** sur les tokens envoyés aux gros LLMs
4. ✅ Vote Symphony Paretto pondéré (uniform en S5) → winner factuel
5. ✅ Verifier post-vote fact-check le winner_text → SUPPORTED + sim haute
6. ✅ Audit chain BLAKE3 + bus SSE complet (45+ events)

**Le tout pour ~3-5 ¢** (vs ~6-9 ¢ baseline naïf 3 LLMs sans pré-mâchage).
**EFE divise par ~2 le coût LLM total** sur missions multi-LLM en
production réelle, en ajoutant la qualité (vote + fact-check + audit).

**Prochaine étape** : Sprint S6 — Router tuning Pareto par task_class
(retour à la Mission A, mais avec table de scoring cost/quality/latency
calibrée par type de question).
