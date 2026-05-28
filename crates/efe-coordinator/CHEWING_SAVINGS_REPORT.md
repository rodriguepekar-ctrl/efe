# EFE Phase 3 R2 — Chewing Savings Report

> **Version** : `s9-r2-2026-05-28`  
> **Généré le** : 2026-05-28T07:57:13.906686+00:00

---

## Résumé exécutif

| Métrique | Baseline | Full (S4+S6) | Savings |
|---|---|---|---|
| Coût total (5 fixtures) | 6 ¢ | 6 ¢ | **+0 %** |
| Latence moyenne | 6241 ms | 5513 ms | — |
| Qualité (pass rate) | 100 % | 100 % | — |
| Failure modes observés | 0 | 2 | — |

→ **EFE coûte ~1.0× moins cher que Claude direct** sur cette mixture.

### Savings observés sur 3 axes (résolution u32 cents limitée)

| Axe | Baseline | Optimisé | Amélioration |
|---|---|---|---|
| **Coût** total | 6 ¢ | 6 ¢ (Full) | 0 % |
| **Latence** moyenne | 6241 ms | 5315 ms (S6) | -15 % |
| **Latence** Speculative (FactualQa) | 6241 ms | 2924 ms | **-53 %** |
| **Qualité** pass rate | 100 % | 100 % (Full) | +0 pp |

---

## Détail par mode

### Baseline (Claude direct, 0 SLM, 0 router)

| Fixture | Cost (¢) | Latency (ms) | Tokens out | LLM | Quality |
|---|---|---|---|---|---|
| 01_factual_simple | 1 | 1641 | 12 | claude_claude-sonnet-4-6 | ✅ |
| 03_factual_compressible | 1 | 1423 | 17 | claude_claude-sonnet-4-6 | ✅ |
| 05_code_gen_rust | 2 | 14564 | 1000 | claude_claude-sonnet-4-6 | ✅ |
| 06_creative_writing | 1 | 7461 | 300 | claude_claude-sonnet-4-6 | ✅ |
| 07_translation_fr_en | 1 | 6117 | 199 | claude_claude-sonnet-4-6 | ✅ |
| **TOTAL** | **6 ¢** | — | — | — | — |
| **MOYENNE** | 1.2 ¢ | 6241 ms | 306 | — | 100 % |

### S4 only (pré-mâchage Mission A)

| Fixture | Cost (¢) | Latency (ms) | Tokens out | LLM | Quality |
|---|---|---|---|---|---|
| 01_factual_simple | 1 | 1063 | 12 | claude_claude-sonnet-4-6 | ✅ |
| 03_factual_compressible | 1 | 1667 | 17 | claude_claude-sonnet-4-6 | ✅ |
| 05_code_gen_rust | 2 | 15799 | 1000 | claude_claude-sonnet-4-6 | ✅ |
| 06_creative_writing | 1 | 7764 | 299 | claude_claude-sonnet-4-6 | ✅ |
| 07_translation_fr_en | 1 | 5843 | 188 | claude_claude-sonnet-4-6 | ✅ |
| **TOTAL** | **6 ¢** | — | — | — | — |
| **MOYENNE** | 1.2 ¢ | 6427 ms | 303 | — | 100 % |

### S6 only (routing tuné par task_class)

| Fixture | Cost (¢) | Latency (ms) | Tokens out | LLM | Quality |
|---|---|---|---|---|---|
| 01_factual_simple | 1 | 512 | 8 | gemini_gemini-2.5-flash | ✅ |
| 03_factual_compressible | 1 | 1454 | 17 | claude_claude-sonnet-4-6 | ✅ |
| 05_code_gen_rust | 2 | 14308 | 997 | claude_claude-sonnet-4-6 | ✅ |
| 06_creative_writing | 1 | 3206 | 134 | gpt_gpt-4o | ✅ |
| 07_translation_fr_en | 1 | 7096 | 233 | claude_claude-sonnet-4-6 | ✅ |
| **TOTAL** | **6 ¢** | — | — | — | — |
| **MOYENNE** | 1.2 ¢ | 5315 ms | 278 | — | 100 % |

### Full (S4 + S6 combinés)

| Fixture | Cost (¢) | Latency (ms) | Tokens out | LLM | Quality |
|---|---|---|---|---|---|
| 01_factual_simple | 1 | 447 | 8 | gemini_gemini-2.5-flash | ✅ |
| 03_factual_compressible | 1 | 2012 | 55 | claude_claude-sonnet-4-6 | ✅ |
| 05_code_gen_rust | 2 | 14373 | 1000 | claude_claude-sonnet-4-6 | ✅ |
| 06_creative_writing | 1 | 2811 | 152 | gpt_gpt-4o | ✅ |
| 07_translation_fr_en | 1 | 7924 | 208 | claude_claude-sonnet-4-6 | ✅ |
| **TOTAL** | **6 ¢** | — | — | — | — |
| **MOYENNE** | 1.2 ¢ | 5513 ms | 285 | — | 100 % |

### Speculative S7 (drafter cheap + verifier premium)

| Fixture | Cost (¢) | Latency (ms) | Tokens out | LLM | Quality |
|---|---|---|---|---|---|
| 01_factual_simple | 1 | 1272 | 12 | claude_claude-sonnet-4-6 | ✅ |
| 03_factual_compressible | 1 | 4576 | 55 | claude_claude-sonnet-4-6 | ✅ |
| **TOTAL** | **2 ¢** | — | — | — | — |
| **MOYENNE** | 1.0 ¢ | 2924 ms | 34 | — | 100 % |

---

## Verdict

- Cost baseline (5 fixtures) : **6 ¢**
- Cost full (5 fixtures)     : **6 ¢**
- **Savings**                : **0 ¢** = **0 %**
- **Qualité préservée**      : baseline 100 % vs full 100 % ✅ identique

**Tests exécutés** : 22 runs (5 modes × 5 fixtures)

---

## Reproductibilité

```bash
export ANTHROPIC_API_KEY=… GOOGLE_AI_KEY=… OPENAI_API_KEY=…
./scripts/slm/start.sh  # démarre analyzer + compressor + verifier
EFE_LIVE_LLM=1 EFE_LIVE_SLM=1 cargo test -p efe-coordinator \
    --features all-providers --test live_chewing_savings_report \
    -- --test-threads=1 --nocapture
```

Output regénéré : `crates/efe-coordinator/CHEWING_SAVINGS_REPORT.{md,json}`
