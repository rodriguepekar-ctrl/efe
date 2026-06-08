# EFE Phase 3 — agrégateur réel : a-priori mesuré + cahier des charges a-posteriori

> Suite d'ADR-SWARM-001 (le pari = l'agrégateur). 2499 prompts × 11 modèles, **30 splits** (moy ± σ). Monolithe = 0.851 @ 0.003321. θ=0.5. Généré 2026-06-06T16:03:29.503965+00:00.

## Verdict

**SPEC** — a-priori insuffisant (selector K=7=0.485 ≈ rand 0.455, loin du plafond OR 0.909). Cible actionnable : un VÉRIFICATEUR a-posteriori de précision v* ≥ 87% sur K=7 candidats → bat le monolithe (0.851) à 3.2× moins cher. → build Phase 3 : jury/débat sur les RÉPONSES, pas routage sur features

## A-priori mesuré (selector kNN) vs plafond OR · spec vérificateur v*

| K | coût (somme) | vote | rand | selector (a-priori, ±σ) | OR (plafond) | v* requis |
|---|---|---|---|---|---|---|
| 3 | 0.0002523 (↓ vs mono) | 0.433 | 0.436 | 0.402 ± 0.026 | 0.695 | 160% |
| 5 | 0.0006093 (↓ vs mono) | 0.412 | 0.437 | 0.487 ± 0.027 | 0.848 | 101% |
| 7 | 0.001047 (↓ vs mono) | 0.432 | 0.455 | 0.485 ± 0.026 | 0.909 | 87% |
| 9 | 0.003459 | 0.542 | 0.504 | 0.484 ± 0.027 | 0.940 | 80% |

## Comment lire

- **selector** = agrégateur A-PRIORI réel (kNN appris sur les features → choisit un candidat). MESURÉ sur vraies données, splits répétés.
- **v\*** = précision MINIMALE d'un vérificateur A-POSTERIORI (qui lit les réponses) pour que l'essaim batte le monolithe. Forme close : `v* = (mono − rand)/(OR − rand)`.
- Si **selector ≈ rand** → la sélection sur features ne capte pas la valeur (écho R0) → il faut un vérificateur qui exploite les RÉPONSES (jury, débat, gold-test-gate sur tâches vérifiables).

## ⚠️ Portée

Le **selector** est pleinement mesuré (vrai ML, vraies features). Le **v\*** est un CAHIER DES CHARGES (forme close), pas une mesure : RouterBench ne fournit pas le TEXTE des réponses, donc un vrai vérificateur/jury n'est pas mesurable ici. Le test pleinement réel = run modèles réels (`swarm_slm.rs` + agrégateur branché) sur une tâche vérifiable, à exécuter quand un backend réel (llama-server / provider) est disponible. Pas de latence dans RouterBench.
