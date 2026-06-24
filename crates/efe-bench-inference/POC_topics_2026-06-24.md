# PoC thèse — premier run sur modèles réels (2026-06-24)

Premier passage du harnais B1 sur de la **vraie inférence** (et non le seam scripté). Modèles servis
localement via Ollama (GGUF importés `ollama create`, aucun téléchargement — registre injoignable).

- **Tâche** : classification de sujet en un mot, corpus `fixtures/topics.jsonl` (8 cas, 4 classes :
  finance/sport/health/science). Scorer **`lenient`** (casse/ponctuation ignorées, le label compte
  s'il apparaît comme mot — éval permissive de vrais modèles).
- **Référence (gros)** : `gemma3:12b`.
- **Essaim (petit)** : `llama3.2-1b` (~1B), puis `phi4-mini` (~3.8B). Consensus Borda, balayage de K.
- **Commande** : `cargo run --example run_bench -p efe-bench-inference -- --endpoint
  http://127.0.0.1:11434 --model <petit> --baseline-model gemma3:12b --ks 1,3,5[,7] --lenient`.

## Résultats (exactitude)

| Petit modèle      | K=1   | K=3  | K=5  | K=7  | réf. gemma3:12b |
|-------------------|-------|------|------|------|-----------------|
| llama3.2-1b       | 25 %  | 25 % | 25 % | 25 % | 87.5 %          |
| phi4-mini ~3.8B   | 87.5 %| 75 % | 75 % | —    | 87.5 %          |

`présent = 100 %` et `R0 = 100 %` partout ; re-preuve indépendante 40/40 puis 32/32 runs (barreau R2
loopback). Coût-jetons estimé (proxy) ; sous un prix petit≈0.1 / gros≈1.2 (¢/1k), `phi4-mini` K=1
**domine** la référence (≥ exactitude, < coût).

## Lecture honnête

1. **Cœur de la thèse — SOUTENU.** Un `phi4-mini` (3.8B) **seul** égale `gemma3:12b` (87.5 %) pour
   ~3× moins de paramètres et un coût moindre. Un petit modèle décent peut matcher le gros, moins cher.
2. **« En grande quantité » / consensus naïf — PAS ENCORE.** Le consensus Borda **n'aide pas, voire
   nuit** (phi4-mini 87.5 %→75 % ; 1B scotché à 25 % ≈ hasard). Le vote sur les échantillons d'un
   petit modèle n'ajoute pas de valeur ici — il faudrait le **consensus vérifié par oracle** (la v2
   déjà codée) ou de la diversité forcée pour faire payer la quantité.

## Réserves (ce n'est pas un benchmark publiable)

- Corpus **minuscule** (8 cas → swings d'1 cas, faible puissance) ; une seule tâche.
- Scorer `lenient` **permissif** (appartenance de mot) — peut sur-compter sur des sorties bavardes.
- Modèle de prix **supposé** (12× ; réaliste ~3× pour 3.8B vs 12B — la domination K=1 tient quand même).
- 1B trop faible/peu divers pour que le consensus récupère quoi que ce soit.

## Valeur

Le harnais **mesure du réel, honnêtement et de façon re-prouvable** (il a même attrapé sa propre
panne d'infra : 1B introuvable → 0 % exactitude **mais 0 % présent**). Le premier signal oriente la
suite : le cœur cheap-parallèle est crédible ; la mécanique de « quantité » à valider est
l'agrégation **vérifiée**, pas le vote naïf.
