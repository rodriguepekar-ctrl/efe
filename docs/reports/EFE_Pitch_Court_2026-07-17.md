# EFE — pitch court

## Le problème

Une décision prise par des agents IA n'est pas vérifiable. On l'accepte sur parole, ou on la refuse.
Dans les métiers réglementés — achats, conformité, audit — ça bloque.

## Ce que fait EFE (les trois schémas)

| Étage | Ce qu'il change | La preuve produite |
|---|---|---|
| **Nano-robots** | Une tâche découpée en 4 petits agents spécialisés, en parallèle, au lieu d'un gros modèle appelé 100 fois en file | Chaque étape scellée ; toucher une étape casse les suivantes |
| **Essaim** | Les agents (Claude, Gemini, petit modèle, code) écrivent sur un tableau partagé au lieu de se parler tous à tous | Un reçu : qui a contribué quoi — re-vérifiable hors ligne |
| **Méta-essaim** | Trois organisations décident ensemble sans partager leurs données : seules les positions circulent | Une preuve que chaque partie re-vérifie seule, sans faire confiance aux deux autres |

Un dénominateur commun : **le tiers re-vérifie lui-même, hors ligne. Personne n'a à faire confiance au fabricant.**

## Ce qui est construit (pas des slides — du code)

- **67 crates Rust, ~450 K lignes, ~9 500 tests.**
- Le relais entre modèles frontière (Fil B) est **servi** : un petit modèle (Qwen3-0.6B, local, grammaire GBNF) traduit la sortie de Claude vers la forme que GPT consomme. Chaque passage est estampillé du moteur réel, scellé dans une chaîne HMAC, borné au temps de mission restant.
- **1 016 tests / 45 suites / 0 échec** au dernier passage sur ce fil.
- **Falsifier un seul sceau casse la re-preuve, et la faute est localisée** — épinglé par test, pas revendiqué.

## Les tests qui comptent vraiment

**Ce qui marche, mesuré :**
- Provenance : 45 contrôles d'audit indépendants (6 vérificateurs), **0 trou** contre la spécification.
- Qualité : 4 revues adverses (agents chargés de réfuter le travail) → **43 défauts bruts trouvés, 19 défauts distincts réels corrigés** — dont un chemin où une invocation réelle ne laissait aucune trace d'audit, reproduit en runtime par le vérificateur.
- Fusion latente Φ : déterministe, invariante au seed — un run se rejoue à l'identique.

**Ce que la mesure a réfuté — de nos propres affirmations :**
- **Constante de Lipschitz** : le certificat analytique annonçait β₁ ≈ 0,17. Mesure black-box : **L̂ = 0,691** (topologie dynamique), 0,366 (fixe). Le certificat est **réfuté** ; la borne honnête est la borne mesurée.
- **Vérificateur arithmétique par ré-exécution** : **92 % vs 94 %** pour la self-consistency à n=50. Il ne paie pas. Le déficit est un déficit de modélisation, pas de calcul.
- **Synthèse/agrégation** (médiane, médiane tronquée, pondérée par audit) : **≤ 85 %**, fusion à 70 %. Les erreurs sont biaisées (skew +8,72), pas symétriques — **aucune agrégation déployable ne comble l'écart** sur ce banc.
- **Consensus** : l'apprentissage par consensus crédite la présence et l'accord, **pas la correction**. Un unanime-faux empoisonne la réputation. Corrigeable seulement avec un oracle de correction dans le panel — sinon c'est une limite de conception, dite.

## Ce que ça vaut

Le différenciateur n'est pas « nos essaims de petits modèles sont meilleurs ». Il est :
**tout ce qui est revendiqué est scellé et re-vérifiable, et tout ce qui ne tient pas à la mesure est dit — y compris quand ça réfute ce qu'on espérait.**

C'est exactement ce qu'exige un produit d'IA en environnement réglementé : pas de la confiance, des reçus.

---
*EFE · 2026-07-17 · chiffres vérifiés à HEAD 02eccdb*
