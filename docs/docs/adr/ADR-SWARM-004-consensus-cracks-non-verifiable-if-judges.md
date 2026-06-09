# ADR-SWARM-004 : Le consensus N-juges crack le non-vérifiable — SI les juges ajoutent du signal

- **Status** : Accepted · **Date** : 2026-06-09
- **Phase** : 3 (Qualité émergente) · **Gate** : Lot S3
- **Related** : R0 (aucun modèle ne domine) ; S1 (le vote échoue) ; ADR-SWARM-002 (S2/F3 INCONCLUS non-vérifiable) ; module `consensus`

## Contexte

ADR-SWARM-002 a laissé **F3** ouvert : sur les tâches **non-vérifiables** (pas de vérificateur
déterministe), T1 Generate-Verify reste INCONCLUS — le juge approximatif (v=0,85) ne suffisait
pas. **Lot S3** tranche : un **consensus N-juges** (Borda/Condorcet sur les classements de
plusieurs juges) bat-il le **vote** (S1) ET le **meilleur seul** (R0) sur le non-vérifiable ?

La math (Vote/Borda/Condorcet + fallback Copeland) a été codée PURE et **prouvée par oracles**
(paradoxe de Condorcet, exemples de manuel, Condorcet≠Borda). Lot S3 l'EXÉCUTE sur des candidats
+ ballots de juges simulés (modèle honnête : juges = détecteurs de correction de compétence v,
décorrélés — l'hypothèse §0 « les juges ajoutent un signal indépendant des producteurs »).

## Décision

1. **GO CONDITIONNEL : le consensus crack le non-vérifiable — à condition que les juges ajoutent
   du signal (compétence v au-dessus d'un seuil ~0,6).**
2. **Le pari de Phase 3 se précise : la QUALITÉ DES JUGES (v).** Pas la math (prouvée), pas les
   producteurs (R0 : aucun ne domine) — le levier est désormais **un juge qui distingue le bon du
   mauvais candidat**.
3. **Construire `ConsensusTopology` + sélection adaptative** (incrément 3) — conditionné à ce GO.
4. **Le dernier gate** : des juges RÉELS (Mode C / LLM) atteignent-ils v ≥ ~0,7 ? (incrément 4).
5. **Debate reste gaté** (ADR-SWARM-003) : Borda/Condorcet one-shot suffisent ici.

## Évidence (Lot S3 — `lot_s3.rs`, math réelle, K=7, J=5, 3000 tâches, 200 bootstraps, ±σ)

| juges v | best-single | vote | **Borda** | **Condorcet** | oracle |
|---|---|---|---|---|---|
| 0,55 | 0,352 | 0,325 | 0,398 | 0,399 | 0,896 |
| **0,70** | 0,352 | 0,325 | **0,716** | **0,706** | 0,896 |
| 0,85 | 0,352 | 0,325 | **0,871** | 0,865 | 0,896 |

**Lecture chirurgicale** :
- **Le vote (0,325) tombe SOUS best-single (0,352)** : les leurres corrélés en font une majorité
  trompeuse → confirme S1, le vote est un anti-pattern ici.
- **À v=0,70 : Borda 0,716 écrase vote (+0,39) ET best-single (+0,36)** → **GO robuste** (bornes
  basses ≫ baselines). À v=0,85, Borda **0,871 ≈ oracle 0,896** (extraction quasi-parfaite).
- **À v=0,55 (juges faibles), Borda ≈ 0,40** : le consensus n'aide presque pas. **Seuil ~0,6.**
- C'est, encore, le schéma R0/AGG : *le prix est réel (oracle 0,90), le levier est l'EXTRACTEUR
  (ici la qualité des juges v)*.

→ **Verdict : GO sur le levier consensus, conditionné à des juges de v ≥ ~0,7.**

## Justification

1. **Cohérence avec toute la chaîne** : Lot1 (méthode ⊘), R0 (routage ⊘, *aucun ne domine*), S1
   (vote ⊘), AGG (a-priori ⊘, spec v*), S2 (vérifiable GO / non-vérifiable INCONCLUS), **S3
   (consensus GO sur non-vérifiable SI juges)**. Chaque gate a déplacé le levier d'un cran ;
   ici il atterrit sur **la qualité du juge**.
2. **Math sans risque** : la correction de Borda/Condorcet est prouvée par oracles, indépendamment
   de toute qualité de modèle. Le GO ne dépend QUE de v (mesurable).
3. **Coût justifié par R0** : best-single = 1 génération ; consensus = 7 générations + **juges Mode
   A gratuits**. +0,36 de qualité pour 7× le coût de génération — le levier R0 (« aucun ne domine »)
   est décisivement réalisé.

## Conséquences

### Positives
- **Le territoire non-vérifiable s'ouvre** (S2 l'avait laissé fermé) — *si* on a de bons juges.
- Borda et Condorcet se valent (0,716 vs 0,706) ; Borda légèrement devant, plus simple → défaut.
- Les juges Mode A (gratuits) sont la première ligne ; le coût reste minime.

### Négatives
- **Tout repose sur v** : des juges faibles/corrélés → consensus ≈ vote (v=0,55). La diversité
  des juges est CRITIQUE (juges corrélés = redondants).
- **Non encore mesuré en réel** : un juge LLM (Mode C) atteint-il v ≥ 0,7 ? C'est le dernier gate.

## Portée & Caveats
- **Candidats + juges SIMULÉS** (modèle honnête : leurres corrélés S1, juges décorrélés de
  compétence v). **La MATH est réelle** (module `consensus`, prouvé). Seul v (réel) reste à mesurer.
- **Corrélation des juges** : Lot S3 suppose des erreurs de juges décorrélées. Des juges corrélés
  abaissent le gain effectif (à mesurer/exploiter : critères orthogonaux, personas variés).

## Question ouverte (le dernier gate)
**Des juges RÉELS (Mode C / LLM-juge, ou les critères Mode A sur de vraies sorties) atteignent-ils
v ≥ ~0,7 sur des tâches non-vérifiables ?** À mesurer avec un backend réel (juges `LlmClient` +
llama) — l'outillage (`consensus`, `lot_s3`, juges Mode A) est prêt.

## Références
- `crates/efe-swarm-topology/examples/lot_s3.rs` — gate (math réelle + candidats/juges simulés)
- `scripts/swarm/EFE_SWARM_S3_2026-06-09.{md,json}` — résultats
- `crates/efe-swarm-topology/src/consensus/` — Vote/Borda/Condorcet (math prouvée par oracles)
- ADR-SWARM-002 (F3 ouvert) · ADR-SWARM-003 (Debate gaté)
