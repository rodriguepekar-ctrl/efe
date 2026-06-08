# ADR-SWARM-002 : Generate-Verify gagne sur les tâches VÉRIFIABLES dans l'enveloppe de capacité des petits modèles

- **Status** : Accepted · **Date** : 2026-06-08
- **Phase** : 2 (Fleet & topologies) · **Gate** : Lot S2
- **Related** : ADR-SWARM-001 (le pari = l'agrégateur a-posteriori) ; S1 ; AGG ; handler async WS2

## Contexte

ADR-SWARM-001 a relocalisé le pari sur un **vérificateur a-posteriori** (≥ 87 %) et laissé UNE
question ouverte : *un vérificateur réel sur les K candidats bat-il le monolithe sur (qualité ×
coût) ?* **Lot S2** tranche cette question pour la topologie **T1 Generate-Verify** (K générateurs
bon marché ∥ + vérificateur a-posteriori) sur **3 familles** de tâches, afin de décider QUELLES
topologies Phase 2 construire (le gate : « ne construire que ce qui gagne »).

Prérequis substrat codé : le **handler ASYNC** (`StreamingSwarm::async_agent`, concurrence bornée
par sémaphore, quiescence préservée) — il branche les agents SLM Mode C dans le streaming
(vérifié : 24 inférences `LlmClient` async, concurrence ≤ 8, quiescence atteinte).

## Décision

1. **T1 Generate-Verify est VALIDÉE — mais bornée par la difficulté de la tâche relative à la
   capacité des petits modèles.** Le gain n'est PAS universel.
2. **Territoire défendable = tâches VÉRIFIABLES dans l'enveloppe de capacité des cheap** : là, un
   vérificateur **déterministe** atteint le plafond OR et bat le monolithe à coût moindre.
3. **Phase 2 construit T1 ciblée sur ce territoire** (+ T2 pipeline pour la décomposition qui
   ramène les tâches dures DANS l'enveloppe). Les cas hors-enveloppe (F2) et non-vérifiables (F3)
   sont **gatés** derrière (a) plus/meilleurs générateurs (remonter OR) et (b) un meilleur juge
   (remonter v) — non construits sans preuve.

## Évidence (Lot S2 — `lot_s2.py`, K=7, 2000 tâches/famille, 200 bootstraps, moy ± σ)

| Famille | single | vote | **verifier (T1)** | OR | monolithe | coût | verdict |
|---|---|---|---|---|---|---|---|
| **F1 vérifiable facile** | 0,250 | 0,416 | **0,900 ± 0,006** | 0,900 | 0,801 | 2,9× moins cher | **GO** |
| **F2 vérifiable dur** | 0,076 | 0,086 | 0,495 | 0,493 | 0,719 | 2,9× moins cher | **NO-GO** |
| **F3 non-vérifiable** | 0,176 | 0,280 | 0,723 | 0,798 | 0,765 | 2,9× moins cher | **INCONCLUS** |

**Lecture chirurgicale** :
- **F1 — GO franc** : vérificateur déterministe = OR (par construction sur tâche vérifiable) =
  **0,90 > monolithe 0,80 à 2,9× moins cher**. C'est le territoire gagnant.
- **F2 — NO-GO de CAPACITÉ** : même le **plafond OR (0,49) < monolithe (0,72)**. Sur les tâches
  dures, 7 petits modèles **ne contiennent pas** la bonne réponse assez souvent. Aucun
  vérificateur ne sauve ça → il faut **plus de générateurs** ou des **modèles cheap plus forts**,
  ou **décomposer** (T2) pour ramener la tâche dans l'enveloppe.
- **F3 — INCONCLUS** : la capacité existe (OR 0,80 > monolithe 0,76) mais le **juge (v=0,85)** ne
  l'extrait pas (0,72 < 0,76). Reproduit AGG : *l'agrégateur est le goulot*. Un juge meilleur
  (jury/débat `efe-multi-llm`) ou vérifiable décompose le problème.

→ **Verdict : GO ciblé (1/3 familles).** T1 gagne là où (capacité cheap suffisante) ET
(vérificateur fort, idéalement déterministe).

## Justification

1. **Cohérence avec toute la chaîne** : Lot 1 (méthode ⊘), R0 (routage = goulot), S1 (capacité
   réelle mais vote ⊘), AGG (a-priori ⊘, spec v\*≥87 %), **S2 (T1 gagne sur vérifiable, bornée
   par capacité)**. Chaque gate a rétréci ET précisé le territoire — discipline cheap-first.
2. **Le déterministe rend F1/F2 LOGIQUEMENT certains** : `verifier == OR` par construction (on
   peut checker un candidat). Le GO de F1 et le NO-GO de F2 ne dépendent PAS de la simulation —
   seul le niveau d'OR dépend des modèles réels.
3. **Scope défendable et fundable** : « essaim bon marché vérifiable » (code qui passe les tests,
   maths, extraction validée par schéma, QA ancrée) — marché réel, moat = vérificateur +
   provenance, **non cannibalisable**.

## Conséquences

### Positives
- **Topologie winner identifiée et bornée** : T1 sur tâches vérifiables → construire en 1ʳᵉ classe.
- **T2 pipeline justifiée** : décomposer une tâche dure (F2) en sous-tâches DANS l'enveloppe.
- Handler async livré → agents SLM Mode C dans le streaming (substrat Phase 2 prêt).

### Négatives
- **Pas de win universel** : sur tâches dures (F2) l'essaim cheap perd — le claim « essaim bat
  monolithe » doit être qualifié « sur tâches vérifiables dans l'enveloppe de capacité ».
- **Non-vérifiable (F3) reste ouvert** : dépend d'un juge fort (v) — à mesurer en réel.
- **Gossip toujours gaté** (aucune preuve), Mode B toujours stub (aucun besoin mesuré).

## Portée & Caveats (ce qui affinerait/rouvrirait)
- **Générateurs faillibles SIMULÉS** (modèle honnête des petits modèles corrélés, hypothèses
  S1/AGG). F1/F2 : déterministe → `verifier==OR` certain ; seul le niveau d'OR exige les modèles
  réels. F3 : le **niveau du juge (v)** exige une mesure réelle.
- **K=7 fixe** : remonter K déplace l'enveloppe (F2 pourrait passer GO à K plus grand / modèles
  plus forts) — à balayer.
- **Run pleinement réel** = rejouer T1 avec `swarm_slm.rs` (génération) + `swarm_verifier.rs`
  (`CandidateVerifier`) + `EFE_SWARM_LLM=llama` (llama-server), sur tâches vérifiables réelles
  (un vérificateur `efe-gold-test-gate` atteint réellement OR). C'est le dernier gate du
  non-vérifiable.

## Question ouverte (le dernier gate)
**Sur des RÉPONSES réelles : (a) quel est le vrai OR des cheap par famille (F2 passe-t-elle GO en
remontant K / la force des modèles ?) et (b) un juge réel (jury/débat) atteint-il v ≥ celui qu'il
faut pour F3 ?** À mesurer avec un backend réel — l'outillage (`swarm_slm`, `swarm_verifier`,
client Mode C réseau, `lot_s2`) est prêt.

## Références
- `scripts/swarm/lot_s2.py` — harness T1 vs monolithe, 3 familles, bootstrap
- `scripts/swarm/EFE_SWARM_S2_2026-06-08.{md,json}` — résultats
- `crates/efe-blackboard/src/streaming.rs` — `async_agent` (handler async Mode C)
- `crates/efe-blackboard/examples/swarm_async_slm.rs` — agent SLM async dans le streaming swarm
- `crates/efe-blackboard/examples/swarm_verifier.rs` — `CandidateVerifier` (le vérificateur a-posteriori)
