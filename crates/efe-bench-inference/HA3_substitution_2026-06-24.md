# HA.3 — première carte des écarts de substitution, MESURÉE (2026-06-24)

Premier passage du module `substitution` (HA.1) sur le corpus pilote (HA.2), vraie inférence Ollama.
Petits (phi4-mini ~3.8B, llama-1b ~1B) vs gros de référence **gemma3:12b**, **K=1**, n=99/classe,
T-MATH (scorer numeric) + T-EXTRACT (scorer set), δ ∈ {2,5,10} pts, α=0.05, non-infériorité appariée
(bootstrap cluster déterministe). Commande : `cargo run --release --example ha_substitution`.

## La carte (exactitude · écart Δ=gros−petit · verdict)

| petit \ tâche | T-MATH | T-EXTRACT |
|---|---|---|
| **phi4-mini 3.8B** | 40.4 % · Δ=+8.1 · **indéterminé** | 82.8 % · Δ=+17.2 · **inférieur** |
| **llama 1B** | 24.2 % · Δ=+24.2 · **inférieur** | 31.3 % · Δ=+68.7 · **inférieur** |
| réf. gemma3:12b | 48.5 % | 100 % |

présent = 100 % partout (les écarts sont de **capacité**, pas de format) · R0 = 100 % ·
**re-preuve indépendante 594/594 runs** (barreau R2 loopback) — la mesure est dans le moat.

## Lecture honnête

1. **H-A (substitution par un petit SEUL, K=1) : NON soutenue ici.** Aucune cellule « non-inférieure »
   après Holm. Les petits, seuls, n'égalent pas gemma3:12b. C'est attendu — et ce n'est pas la thèse
   (qui parie sur l'**agrégation**, H-B). H-A fait son travail : il **dimensionne** H-B.
2. **La carte DIRIGE H-B, chiffré.** L'écart résiduel le plus petit est **phi4-mini × T-MATH (Δ≈8 pts,
   indéterminé)** → la cellule la plus prometteuse pour l'expérience « la quantité paie » : si le
   consensus vérifié récupère ~8 pts là, phi4-math **substitue**. Les cellules à 24–69 pts (le 1B) sont
   hors de portée d'une agrégation — on n'y envoie pas le GPU.
3. **I-HA-POWER, en action.** À n=99, le verdict de la cellule serrée (phi4-math) est honnêtement
   **indéterminé** ; `required_n ≈ 2943` à δ=10. Le harnais REFUSE de déguiser un manque de données en
   parité → HA.4 doit élargir le corpus pour un verdict ferme. (Les cellules « inférieur » ont
   `n_req=∞` : on ne peut pas prouver une non-infériorité qui est fausse — correct.)

## Caveat méthodologique (vérifié, important)

Les 48.5 % de gemma3:12b sur l'arithmétique sont un **artefact de suppression du raisonnement** :
l'instruction « reply with ONLY the number » interdit le chain-of-thought. Probe : sur un problème
3-op, gemma3:12b répond **122 (faux)** sans raisonnement, mais calcule **70 + 72 = 142 (juste)** avec
« step by step ». Le multi-étapes en un coup est dur même pour un gros modèle. L'écart Δ reste valide
(les deux modèles sont contraints à l'identique — I-HA-FAIR), mais les exactitudes ABSOLUES de T-MATH
sont déprimées : la tâche, telle que configurée, mesure « arithmétique en un coup », pas
« arithmétique ». **HA.4 doit autoriser le CoT + un vérificateur** — ce qui est aussi, précisément, le
mécanisme que la thèse parie (best-of-N vérifié = échantillonner des CoT, garder le vérifié-correct).

## Valeur

- L'**instrument fonctionne** : première carte des écarts réelle, re-prouvable, avec puissance honnête.
- Premier verdict de substitution : **négatif au K=1**, mais il **chiffre la cible de H-B** (≈8 pts sur
  phi4×math) — exactement la fonction de H-A.
- Deux directives nettes pour la suite : **(a)** HA.4 à l'échelle (n≫99) + CoT + vérificateur sur
  T-MATH ; **(b)** la cellule phi4×math est l'entrée de l'expérience pivot H-B (consensus vérifié).
