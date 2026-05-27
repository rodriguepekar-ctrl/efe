# Phase 3 R2 — Sprint S4 : Live Validation (TEMPLATE)

> **Ceci est un template.** À remplir avec les valeurs OBSERVÉES après exécution
> réussie de `live_mission_a_full_pre_chewing_pipeline`.
> Une fois rempli, renommer en `S4_LIVE_VALIDATION.md` (sans `.template`).
> Cohérence avec S1/S2/S3 templates.

---

**Date de validation** : YYYY-MM-DD (HH:MM UTC)
**Hardware** : <Mac M-series / Linux x86_64 / …>
**llama-server** : build 9297+, daemons analyzer (:8090) + compressor (:8092)
                   + verifier (:8093) UP
**Claude model** : `claude-sonnet-4-6` (DEFAULT_CLAUDE_MODEL)
**Phase 1 SLMs actifs** : **3 sur 4** (analyzer + compressor + verifier ;
                          reformulator :8091 UP mais inutilisé dans le workflow)

---

## ✅ Résultat final — 28/28 assertions PASS attendu

```text
running 3 tests
=== Mission A naked === (S2, déjà validé)
test live_mission_a_naked_no_slm_just_claude ... ok

=== Mission A with analyzer + verifier === (S3, déjà validé)
test live_mission_a_with_analyzer_verifier ... ok

=== Mission A FULL pre-chewing pipeline ===
  query_raw      : "Je suis en train de préparer une présentation..." (527 chars)
  llm_id         : claude_claude-sonnet-4-6
  4 robots actifs: analyzer + router + compressor + verifier
  candidate_srcs : 2

  ── Phase 1 : Pre-chewing SLM ──
    analyzer     : <X> ms
    router       : <X> ms   → claude_claude-sonnet-4-6  (reason: "<…>")
    compressor   : <X> ms   (<RAW> → <COMP> tokens, ratio=<R>, savings=<S> tokens <P>%)

  ── Phase 2 : LLM sur prompt compressé ──
    llm          : <X> ms   (<OUT> out tokens, cost <C> ¢)
    final_text   : "<La capitale de la France est Paris.>"

  ── Phase 3 : Verifier post-LLM ──
    verifier     : <X> ms
    verdict      : SUPPORTED
    similarity   : <X>
    best_source  : Some("wp_paris_fr")

  ── ChewingMetrics ──
    total_slm_latency_ms      : <X>
    total_pipeline_latency_ms : <X>
    wall_clock_ms             : <X>
    audit_hash   : <abc...def> (64 chars)
    events_seen (<N>) : [..., "PRE_CHEWING_METRICS", "SWARM_FINISHED"]
test live_mission_a_full_pre_chewing_pipeline ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

---

## Métriques observées vs cibles (S4 = 28 assertions C1-C28)

### Héritées S2/S3 (C1-C15)

| # | Métrique | Cible | Observé | Statut |
|---|---|---|---|---|
| C1 | final_text non vide | A1 | `<…>` | <✅/❌> |
| C2 | contient "paris" | A2 | `<…paris…>` | <✅/❌> |
| C3 | llm_used = claude | A3 | `claude_claude-sonnet-4-6` | <✅/❌> |
| C4 | cost ≤ 5 ¢ | A4 | <X> ¢ | <✅/❌> |
| C5-C9 | latency caps + audit + wall | A5-A9 | <…> | <…> |
| C10 | verdicts non vide | B10 | 1 | <✅/❌> |
| C11 | verdict = SUPPORTED | B11 | `SUPPORTED` | <✅/❌> |
| C12 | similarity ≥ 0.3 | B12 | <X> | <✅/❌> |
| C13-C15 | events analyzer + verifier | B13-B15 | tous présents | <…> |

### Nouvelles S4 (C16-C28) — le but premier de R2

| # | Métrique | Cible | Observé | Statut |
|---|---|---|---|---|
| C16 | `chewing_metrics.is_some()` | true | <…> | <✅/❌> |
| C17 | `input_tokens_raw ≥ 100` | (query > 500 chars) | <X> tokens | <✅/❌> |
| C18 | `input_tokens_compressed < raw` | compressor réduit | <X> < <Y> | <✅/❌> |
| C19 | **`compression_ratio ≤ 0.7`** | **≥ 30 % savings** | **<X>** | <✅/❌> |
| C20 | `savings_tokens ≥ 20` | (fixture min) | <X> | <✅/❌> |
| C21a | `compressor_latency > 0` | compressor a tourné | <X> ms | <✅/❌> |
| C21b | `compressor_latency < 10000` | sub-10s | <X> ms | <✅/❌> |
| C22 | `router_latency < 100` | Pareto in-process | <X> ms | <✅/❌> |
| C23 | `selected_llm == claude_id` | 1 seul candidat | `claude_…` | <✅/❌> |
| C24 | `router_reason` non vide | audit trail | `"<…>"` | <✅/❌> |
| C25 | tag PRE_CHEWING_METRICS | event émis | présent | <✅/❌> |
| C26 | `slm ≤ pipeline` | cohérence | <X> ≤ <Y> | <✅/❌> |
| C27 | `llm_latency ≥ 100` | plancher TLS | <X> ms | <✅/❌> |
| C28 | `latency_ms < 25000` | cap S4 | <X> ms | <✅/❌> |

**Score attendu : 28/28 assertions vertes.**

---

## Argument économique R2 — chiffres observés

| Métrique | Baseline (Claude direct sur paragraphe) | Avec pré-mâchage S4 | Savings |
|---|---|---|---|
| Tokens d'entrée LLM | ~<RAW> | ~<COMPRESSED> | **<S>** (**<P>%**) |
| Coût LLM | ~<X> ¢ | ~<Y> ¢ | **<Z>%** |
| Latence | ~<X> ms (1 call HTTP) | ~<Y> ms (3 SLM + LLM + verifier) | acceptable |

→ **EFE divise par ~<F> le coût LLM sur queries longues** tout en gardant
audit chain BLAKE3 + fact-check verifier + extraction d'entités.

C'est l'argument économique central qui justifie le R2.

---

## Observations chirurgicales bonus

<à remplir post-run>

---

## Critère d'acceptation S4 — État final

- [x] `src/metrics.rs` créé avec ChewingMetrics + 10 unit tests ✅
- [x] `StreamEvent::PreChewingMetrics` variant + tag() ✅
- [x] `MissionAResponse.chewing_metrics: Option<…>` + #[serde(default)] ✅
- [x] `run_mission_a` instrumenté ~50 LOC ✅
- [x] Fixture `03_factual_compressible.json` (527 chars > 500) ✅
- [x] Test `live_mission_a_full_pre_chewing_pipeline` (28 assertions) ✅
- [x] 4 tests OFFLINE couvrant chewing_metrics (G3 patch) ✅
- [x] cargo build clean ✅
- [x] cargo clippy 0 warning S4 ✅
- [x] Skip propre × 4 cas ✅
- [x] 0 régression suite offline ✅
- [<>] Test live PASS avec env + 3 daemons UP
- [<>] 28/28 assertions C1-C28 PASS
- [<>] compression_ratio observé ≤ 0.7
- [<>] savings_tokens observé ≥ 20
- [<>] Event PRE_CHEWING_METRICS observé dans le bus
- [<>] `S4_LIVE_VALIDATION.md` scellé (ce fichier renommé)
- [x] Commit `feat(phase-3-r2-s4): …` (a0bea13) ✅

---

## Commits R2 cumulés au moment du scellement S4

```text
<HHHHHHH> docs(phase-3-r2-s4): scellement live validation S4 28/28 PASS
<HHHHHHH> feat(phase-3-r2-s4-patch): combler N gaps chirurgicaux S4
a0bea13 feat(phase-3-r2-s4): full pre-chewing pipeline + ChewingMetrics
2884da9 docs(phase-3-r2-s4): architecture chirurgicale full pipeline + ChewingMetrics
4604a85 docs(phase-3-r2-s3): scellement live validation S3 16/16 PASS
+ commits S1+S2+S3
```

---

## Conclusion S4

Sprint S4 validé end-to-end. **Le pipeline pré-mâchage complet (4 robots
actifs : analyzer + router + compressor + verifier) + le module
`ChewingMetrics` prouvent quantitativement le but premier de R2** :

- **Tokens d'entrée du LLM** : réduits de <P>% via le compressor
- **Coût $** : divisé par ~<F> sur queries longues
- **Qualité** : verdict SUPPORTED maintenu via le verifier post-LLM
- **Audit** : chaîne BLAKE3 + 28+ events bus complets

**Prochaine étape** : Sprint S5 — Mission B (3-LLM vote Claude+Gemini+GPT)
avec pré-mâchage **mutualisé** (1 compresseur × 3 LLMs = 3× savings).
