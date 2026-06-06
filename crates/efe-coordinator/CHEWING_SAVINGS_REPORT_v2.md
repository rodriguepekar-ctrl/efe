# EFE Phase 3 R3 — Chewing Savings Report (µ¢ precision)

> **Version** : `s9-r3-2026-05-30`  
> **Généré le** : 2026-05-30T21:51:41.476916+00:00  
> **Précision** : microcents (1 µ¢ = 1/1000 cent = 1/100 000 USD).
> Les valeurs `X.XXX ¢` agrègent fidèlement même les missions sub-cent
> (Gemini Flash 1k/1k tokens = 0.037 ¢ exact ↔ 1 ¢ arrondi en legacy).

---

## Résumé exécutif (µ¢ precision)

| Métrique | Baseline | Full (S4+S6) | Savings |
|---|---|---|---|
| Coût total (6 fixtures) | 2.426 ¢ | 2.246 ¢ | **-7 %** |
| Coût moyen / run | 0.404 ¢ | 0.374 ¢ | — |
| Latence moyenne | 5282 ms | 9473 ms | — |
| Qualité (pass rate) | 100 % | 100 % | — |
| Failure modes observés | 0 | 4 | — |

→ **EFE coûte ~1.08× moins cher que Claude direct** (calculé µ¢).

---

## Comparaison résolution legacy (cents) vs R3.S1 (µ¢)

| Métrique | Legacy (cents) | R3.S1 (µ¢) | Δ |
|---|---|---|---|
| Baseline total | 7 ¢ | 2.426 ¢ | -4.574 ¢ |
| Full total | 7 ¢ | 2.246 ¢ | -4.754 ¢ |
| Savings ratio | 0.0 % | 7.4 % | +7.4 pp |

---

## Détail par mode (µ¢ precision)

### Baseline (Claude direct, 0 SLM, 0 router)

- **6 runs**, total = 2.426 ¢, moy = 0.404 ¢/run
- Latence moy = 5282 ms, tokens moy = 256, qualité = 100 %

### S4 only (pré-mâchage Mission A)

- **6 runs**, total = 2.440 ¢, moy = 0.407 ¢/run
- Latence moy = 11169 ms, tokens moy = 258, qualité = 100 %

### S6 only (routing tuné par task_class)

- **6 runs**, total = 2.058 ¢, moy = 0.343 ¢/run
- Latence moy = 4068 ms, tokens moy = 224, qualité = 100 %
- ⚠ 4 dégradations observées

### Full (S4 + S6 combinés)

- **6 runs**, total = 2.246 ¢, moy = 0.374 ¢/run
- Latence moy = 9473 ms, tokens moy = 248, qualité = 100 %
- ⚠ 4 dégradations observées

### Speculative S7 (drafter cheap + verifier premium)

- **3 runs**, total = 0.173 ¢, moy = 0.058 ¢/run
- Latence moy = 3304 ms, tokens moy = 25, qualité = 100 %

---

_Rapport généré via `SavingsReport::to_markdown_v2()` (R3.S1+R3.S9)._
