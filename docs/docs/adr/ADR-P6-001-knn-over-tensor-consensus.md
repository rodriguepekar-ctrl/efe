# ADR-P6-001 : Routage P6 par centroïde-kNN sur embeddings, pas par consensus tensoriel (Mode 1)

- **Status** : Accepted · **Date** : 2026-06-02
- **Related** : Expérience Lot 1 (`scripts/latent/`) ; architecture P6 (passerelle pré-mâchage à routage latent) ; campagnes H0·a/H1·a (économie du pré-mâchage)

## Contexte

**P6** = la passerelle de pré-mâchage prouvée (P1 : compression de prompt gate-protégée,
démontrée live à −47/−60 % de tokens) dont le routeur heuristique Pareto serait remplacé
par une **classification de prompt prédite**. Dans le code actuel, `RouterClient` ne prédit
pas la `task_class` : il la **reçoit** du DTO (défaut `FactualQa`) et pondère coût/latence/
capacité — l'analyzer SLM est non-gating.

La **thèse de moat** de P6 (et de P2, « vendre le moteur de classification brut ») reposait
ENTIÈREMENT sur une hypothèse : que le **consensus tensoriel de Mode 1** (`efe-core` :
genesis → fusion attention → consensus + entropie) classe **mieux** qu'un simple
**centroïde-kNN** sur embeddings. C'était, au classement nouveauté × concurrence, la **seule**
nouveauté défendable du dossier (P1 est dans un espace encombré et cannibalisé par le prompt
caching des providers ; P3/P4/P5 sont saturés).

**Lot 1** était l'expérience go/no-go la **moins chère possible** pour valider ou tuer cette
hypothèse **avant** d'investir le flywheel (Lots 2-6) sur une prémisse non vérifiée.

## Décision

1. **Le routage de P6 se fait par embedding (bge-m3) + centroïde-kNN.** Simple, déterministe, < 5 ms.
2. **On NE branche PAS `efe-core` (consensus Mode 1) comme cerveau de routage.**
3. **Le moat est relocalisé** : plus dans la *méthode* de classification (elle n'apporte rien),
   mais dans le **système** (passerelle dans le chemin de dépense, sticky) + le **flywheel**
   (active-learning sur les outcomes réels de routage). ⚠️ Ces deux moats **n'existent qu'avec
   du trafic** — ils ne sont pas défendables au repos.
4. **L'architecture P6 reste valide à ~90 %** : seul le classifieur change (kNN au lieu de
   consensus) ; le seam routeur, la politique pilotée par la confiance (le kNN se calibre
   aussi), le flywheel et l'A/B shadow sont identiques.

## Évidence (Lot 1 — `scripts/latent/`)

**Design contrôlé** : kNN = consensus à **1 itération** ; consensus = **K itérations**. Même
embedding, mêmes ancres → tout écart est imputable à l'itération. Seed = **132 prompts
labellisés** (108 clairs + 24 **ambigus**, 6 `TaskClass`). Verdict sur **splits stratifiés
répétés** (moyenne ± σ), pas un tirage unique.

| Mesure | Résultat |
|---|---|
| Embedding réel (ST multilingue), **30 splits** — Δaccuracy **ambigu** (consensus − kNN) | **−0,4 % ± 3,1 %** (centré sur 0) |
| idem, **lenient** (pred ∈ classe primaire ∪ `also`) | −0,3 % ± 1,8 % |
| **Sweep 24 configs** (iters × tau × beta) — meilleur Δambigu | **+0,4 % ± 2,2 %** (chevauche 0) |
| Configs battant le kNN **de façon robuste** | **aucune** (la plupart négatives) |
| Seul effet survivant : **calibration** | ECE kNN 0,282 → consensus 0,183 |
| Plafond : lenient ambigu du **kNN seul** | déjà **93 %** (≈ au plafond, aucune place pour la méthode) |

**Leçon de méthode** : un **split unique** donnait un trompeur +4 à +8 % — du **pur bruit**
révélé par la répétition. La rigueur statistique a inversé une fausse conclusion positive.

→ **Verdict : NO-GO sur le moat-méthode** (robuste, multi-config, multi-split).

## Justification

1. **Coût/risque** : binder `efe-core` (API swarm génératif lourde) pour un gain **nul** = dette.
   Le kNN fait ~30 LOC, est déterministe et < 5 ms (vs un swarm multi-tours).
2. **Preuve cheap-first** : si la math du cœur (consensus itératif sur ancres) n'égale pas le
   kNN, le moteur complet est improbable à le sauver. Pour **~400 LOC + un seed + ~1 h**, on a
   évité de bâtir le flywheel (Lots 2-6) sur du sable.
3. **L'architecture survit** : le pivot **retire** le composant non prouvé sans casser le reste.

**vs Alternative A (durcir le test)** — seed ≥ 300 + bge-m3 + vrai `efe-core`. **Rejetée pour
l'instant** : trois mesures convergentes (single → repeats → sweep) disent la même chose ; le
fardeau de la preuve a basculé ; poursuivre serait du sunk-cost. Réouvrable (cf. Caveats).

## Conséquences

### Positives
- Routeur **plus simple, plus rapide, déterministe**, sans dépendance lourde à `efe-core`.
- P6 **dé-risqué** : on ne construit plus le flywheel sur une hypothèse fausse.
- **Preuve documentée** (rigueur statistique) — pourquoi le consensus n'a pas été poursuivi.

### Négatives
- **Le moat-méthode (la seule nouveauté défendable) disparaît.** P6 devient « bon gateway +
  routage kNN » dans un espace encombré (RouteLLM, Martian, NotDiamond, Portkey, LiteLLM…).
- **Conséquence funding** : P6 = **wedge / produit de trésorerie**, pas licorne-à-moat. La
  différenciation doit venir de **l'exécution + la donnée** (flywheel), qui n'existent qu'avec
  des clients.
- **L'inconnue #1 (mesure du trafic réel) devient le seul levier de valeur défendable.**

## Question ouverte (le vrai test, non encore réalisé)

Lot 1 a répondu « consensus vs kNN ». Reste à trancher : **le routage kNN bat-il le routage
NAÏF** (toujours-premium / aléatoire / défaut-DTO) **sur le composite coût × latence ×
qualité ?** Si non, P6 n'a aucune story de routage (= juste P1). À mesurer — réutilise le
harness + des outcomes LLM réels.

## Caveats (ce qui rouvrirait cette décision)
- **MiniLM ≠ bge-m3** : mais bge-m3 renforcerait probablement le NO-GO (le kNN est déjà au plafond).
- **Consensus minimal ≠ `efe-core` complet** : un GO exigerait de tester le vrai moteur.
- **Tâche peut-être trop facile** (6 classes) : une classification plus dure pourrait favoriser
  le consensus — hors du cas d'usage routage.

## Références
- `scripts/latent/latent_eval.py` — harness contrôlé (kNN = consensus@1iter ; splits répétés ; sweep)
- `scripts/latent/corpus/taskclass_seed_v1.json` — seed labellisé (132 prompts)
- `scripts/latent/EFE_LATENT_LOT1_2026-06-02.{md,json}` + `EFE_LATENT_LOT1_SWEEP_2026-06-02.json` — résultats
