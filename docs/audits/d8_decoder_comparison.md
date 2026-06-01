# Sprint D8 — Decoder comparison report (Plan B activation)

> **Statut** : axe 1 (Plan B activation + measurement) **livré** ;
> axes 2 (real-data) et 3 (D8.5 opt-in code livré, validation
> empirique en attente) déclenchés selon flowchart D8.4.
> **Mesure live** : 2026-04-26 sur la machine de référence M3 Pro,
> binaire `target/release/efe-server` post-D7-freeze.

---

## 1. Contexte

Suite au freeze D7 (winner `(β₀, λ_β, α₀) = (0.45, 0.08, 0.25)`,
aggregate macro_f1 = 0.815), le résidu **H3c** persiste sur le
catalog `billing_escalation` : la classe `question` est confondue
avec `billing` (3/3 cas), capant le `min_per_catalog_macro_f1` à
0.6548. L'architecture D8 (`docs/bench/d8_hybrid_realdata_architecture.md`)
identifie 3 axes de réponse complémentaires :

1. **Axe 1 — Plan B activation** : industrialiser le `HybridClassifier`
   armé en D7.6 (raw cosine + swarm consensus) et **mesurer** son
   impact réel sur les 3 catalogs.
2. **Axe 2 — Real-data validation** : 500+ cas partenaire pilote
   anonymisés, re-baseline v0.1.8.
3. **Axe 3 (opt-in) — AnchorAwareTemplateRegistry** : refonte
   `efe-core::genesis::registry` qui modulé le NaSem prior par les
   anchors actifs.

Ce rapport documente **l'axe 1** ; l'axe 2 est en attente de
partenaire ; l'axe 3 est livré comme code opt-in (pas activé par
défaut) avec 10 tests unitaires.

---

## 2. Mesure live D8.1 — résultats bruts

Run de `eval/scripts/compare_decoders.py` sur les 3 catalogs
synthétiques actuels, contre le binaire post-D7 :

| Catalog | n | Decoder | Acc | macro-F1 | Mean conf correct | Latency mean (ms) |
|---------|--:|---------|----:|---------:|------------------:|------------------:|
| support_triage | 40 | raw_cosine | 0.7750 | 0.7692 | 0.2402 | 392 |
|                |    | swarm_only | 0.8750 | 0.8760 | 0.6940 | 1991 |
|                |    | **hybrid** | **0.8750** | **0.9077** | 0.6928 | 9 (cache) |
| fraud_alert | 30 | raw_cosine | 0.8333 | 0.7308 | 0.2681 | 161 |
|             |    | swarm_only | 0.9000 | 0.8825 | 0.6945 | 2035 |
|             |    | **hybrid** | **0.9333** | **0.9250** | 0.7015 | 10 (cache) |
| billing_escalation | 30 | raw_cosine | 0.6333 | 0.4448 | 0.2772 | 191 |
|                    |    | swarm_only | 0.8667 | 0.6548 | 0.6951 | 2191 |
|                    |    | **hybrid** | **0.9000** | **0.7326** | 0.6999 | 10 (cache) |

> ⚠ **Note latence** : le hybrid mesure ~10 ms parce que `EfeClient`
> a un cache LRU et le hybrid s'exécute 2ᵉ après le swarm-only sur
> les mêmes inputs. La latence architecturale uncached du hybrid =
> swarm_only latency + ~5 ms (raw cosine + branch decision).

### 2.1 Distribution des branches du hybrid

| Branch | support_triage | fraud_alert | billing_escalation | Total (% sur 100) |
|--------|--------------:|------------:|-------------------:|------------------:|
| `agree` | 35/40 (87.5 %) | 29/30 (96.7 %) | 29/30 (96.7 %) | 93/100 (93 %) |
| `raw_overrides_swarm` | 2/40 (5 %) | 1/30 (3.3 %) | 1/30 (3.3 %) | 4/100 (4 %) |
| `swarm_overrides_raw` | 0/40 | 0/30 | 0/30 | 0/100 |
| `reject` (uncertain) | 3/40 (7.5 %) | 0/30 | 0/30 | 3/100 (3 %) |

Le `swarm_overrides_raw` à 0 sur les 3 catalogs est intéressant : ça
signifie que dans **aucun cas** le swarm n'a une confiance haute
(≥ 0.85) sur un label différent du raw cosine. C'est cohérent avec
la confidence mesurée (~0.69 mean) — sous le seuil de 0.85.

---

## 3. Delta analysis chiffrée

| Métrique | Catalog | Hybrid - swarm_only |
|----------|---------|--------------------:|
| Acc | support_triage | 0.0000 |
| | fraud_alert | **+0.0333** |
| | billing_escalation | **+0.0333** |
| **Aggregate** | (mean) | **+0.0222** |
| macro-F1 | support_triage | **+0.0317** |
| | fraud_alert | **+0.0425** |
| | billing_escalation | **+0.0778** |
| **Aggregate** | (mean) | **+0.0507** |
| Min per-catalog F1 | (worst case) | **0.6548 → 0.7326** |

Le gain le plus marqué est sur `billing_escalation` (+0.078 macro_f1) —
exactement le résidu H3c que D7 ne pouvait pas résoudre. Le
`HybridClassifier` corrige ce résidu via la branche
`raw_overrides_swarm` qui permet au cosine pur de sauver les cas
swarm-confondants.

### 3.1 Cas spécifiques où le hybrid bat swarm_only

Du `target/decoder_comparison/report.json` (`only_swarm_correct = 5`,
`hybrid_better_than_swarm = 4` cumulés sur 3 catalogs) :

* `be019` : "Where do I find a record of my past invoices?"
  - swarm : `billing` (faux ; expected = `question`)
  - raw : `question` (correct, marge 0.04)
  - hybrid : **`question`** via `raw_overrides_swarm`
* `fa015` : "How do I enable two-factor authentication?"
  - swarm : `billing` (faux ; expected = `question`)
  - raw : `question` (correct)
  - hybrid : **`question`**
* (similaire sur 2 autres cas)

### 3.2 Cas où le hybrid PERD vs swarm_only

`hybrid_worse_than_swarm = 2` (cumulés sur 3 catalogs) — 2 cas où
le swarm a la bonne réponse mais le hybrid a basculé en `reject`
faute d'agreement raw+swarm. Ces cas auraient profité d'un seuil
`raw_top_gap` plus permissif. **Action follow-up D8.6** : tightening
du `swarm_high_confidence` ou loosening du `raw_high_margin` →
peut-être une ½ j de re-tuning si on veut squeezer 1-2 % de plus.

---

## 4. SLO compliance D8

Tous les **4 SLOs `D8-HYB-*`** PASS :

| SLO | Target | Measured | Pass |
|-----|-------:|---------:|:----:|
| D8-HYB-1 (acc support_triage) | ≥ 0.80 | 0.875 | ✅ |
| D8-HYB-2 (macro_f1 support_triage) | ≥ 0.75 | 0.908 | ✅ |
| D8-HYB-3 (reject_rate support_triage) | ≤ 0.15 | 0.075 | ✅ |
| D8-HYB-4 (min macro_f1 across 3) | ≥ 0.60 | 0.733 | ✅ |

Les **2 SLOs `D8-RD-*` + 1 SLO `D8-LAT-2`** restent **SKIPPED** —
real-data partner golden non encore acquis (cf. §6).

---

## 5. Décision gate D8.4 — interprétation

Per le flowchart de l'architecture D8 :

```
aggregate_macro_f1 hybrid (synthétique) = 0.857 ≥ 0.85 → Path A
  → Freeze v0.1.8 (ce qui est fait)
  → Plan recommandé : déployer hybrid en production par défaut côté client
  → AnchorAware (D8.5) reste opt-in, livré comme code prêt-à-activer
  → CI fail-closed flip recommandé après 3 main runs PASS
```

Le **chemin A** est emprunté. La voie alternative (D8.5 forcée comme
default) n'est **pas justifiée par les données synthétiques** : le
gain marginal du hybrid suffit. Le code D8.5 reste en attente d'une
validation real-data (D8.2) qui pourrait justifier de l'activer si
le hybrid plafonne en production.

---

## 6. Étapes restantes D8.2 (real-data partner)

Pour passer le gate D8-RD-* :

1. Démarcher 5 partenaires pilotes en parallèle.
2. Une fois NDA signé : extract de ≥ 500 cas labellisés.
3. Run `eval/scripts/anonymize_golden.py` (PII redaction + audit log
   hors-repo).
4. Run `eval/scripts/check_no_pii.py` (CI gate, fail any leak).
5. Run `eval/scripts/run_eval_hybrid.py --multi-catalog` sur les 3
   catalogs synthétiques + 1 catalog partenaire.
6. Re-baseline `bench_baselines/v0.1.8/baseline.json` avec les
   mesures réelles.
7. Si real_data macro_f1 ≥ 0.85 : ship Phase 1 milestone (cf.
   `docs/roadmap.md`).
8. Si 0.75-0.85 : ship avec caveat + planifier D8.5 activation.
9. Si < 0.75 : escalader D8.5 (default → AnchorAware).

---

## 7. Out-of-scope D8 confirmé

| Item | Sprint | Justification |
|------|--------|---------------|
| Production Helm chart | D9 | Sprint dédié, ~3 j |
| First customer onboarding | D14 | Phase 2 roadmap |
| 2-stage intent classifier (action vs info) | D-Mode-2 | Out-of-scope MVP |
| GPU backend CUDA | E1 / Phase 5 | Phase 2 architecture |
| Custom embedder marketplace | Phase 6 | Vision long-terme |

---

## 8. Métriques de succès

À 30 jours après merge D8 :

> *« 3 catalogs synthétiques + 1 catalog partenaire labellisé.
> Aggregate macro_f1 ≥ 0.80 sur les 4. Plan B (HybridClassifier) est
> documenté avec example Python ; les 16 SLOs (5 D6 + 4 D7 + 7 D8)
> sont gated dans le CI workflow ; aucun PII n'est leak ; le job
> `bench-accuracy*` est passé en fail-closed après 3 main runs PASS. »*

Si cette affirmation tient, D8 a rempli sa mission : **EFE Mode 1
est productize, validé sur données réelles, avec un Plan B mesuré
et un Plan B+ (D8.5) prêt à activer si nécessaire — le sprint D9
(Docker + Helm) peut démarrer sans dette technique résiduelle**.
