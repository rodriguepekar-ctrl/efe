# EFE Phase 3 R3 — Chewing Savings Report (µ¢ precision)

> **Version** : `s9-r3-2026-05-30`  
> **Généré le** : 2026-05-30T10:22:52.492225+00:00  
> **Précision** : microcents (1 µ¢ = 1/1000 cent = 1/100 000 USD).
> Les valeurs `X.XXX ¢` agrègent fidèlement même les missions sub-cent
> (Gemini Flash 1k/1k tokens = 0.037 ¢ exact ↔ 1 ¢ arrondi en legacy).

---

## Résumé exécutif (µ¢ precision)

| Métrique | Baseline | Full (S4+S6) | Savings |
|---|---|---|---|
| Coût total (6 fixtures) | 2.379 ¢ | 1.727 ¢ | **-27 %** |
| Coût moyen / run | 0.397 ¢ | 0.288 ¢ | — |
| Latence moyenne | 4951 ms | 4049 ms | — |
| Qualité (pass rate) | 100 % | 100 % | — |
| Failure modes observés | 0 | 1 | — |

→ **EFE coûte ~1.38× moins cher que Claude direct** (calculé µ¢).

---

## Comparaison résolution legacy (cents) vs R3.S1 (µ¢)

| Métrique | Legacy (cents) | R3.S1 (µ¢) | Δ |
|---|---|---|---|
| Baseline total | 7 ¢ | 2.379 ¢ | -4.621 ¢ |
| Full total | 5 ¢ | 1.727 ¢ | -3.273 ¢ |
| Savings ratio | 28.6 % | 27.4 % | -1.2 pp |

---

## Détail par mode (µ¢ precision)

### Baseline (Claude direct, 0 SLM, 0 router)

- **6 runs**, total = 2.379 ¢, moy = 0.397 ¢/run
- Latence moy = 4951 ms, tokens moy = 251, qualité = 100 %

### S4 only (pré-mâchage Mission A)

- **6 runs**, total = 2.464 ¢, moy = 0.411 ¢/run
- Latence moy = 5296 ms, tokens moy = 261, qualité = 100 %

### S6 only (routing tuné par task_class)

- **6 runs**, total = 1.715 ¢, moy = 0.286 ¢/run
- Latence moy = 3721 ms, tokens moy = 304, qualité = 100 %

### Full (S4 + S6 combinés)

- **6 runs**, total = 1.727 ¢, moy = 0.288 ¢/run
- Latence moy = 4049 ms, tokens moy = 294, qualité = 100 %
- ⚠ 1 dégradations observées

### Speculative S7 (drafter cheap + verifier premium)

- **3 runs**, total = 0.144 ¢, moy = 0.048 ¢/run
- Latence moy = 1677 ms, tokens moy = 23, qualité = 100 %

---

_Rapport généré via `SavingsReport::to_markdown_v2()` (R3.S1+R3.S9)._
