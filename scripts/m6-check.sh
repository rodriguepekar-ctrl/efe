#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
#  M6 — la conservation limitée, éprouvée.
#
#  Le critère : « un cas expiré disparaît, et tout le reste se re-prouve
#  encore ». Il a l'air contradictoire — une chaîne scellée se replie sur
#  les OCTETS ENTIERS de chaque nœud, donc retirer un record casse tous
#  les sceaux qui suivent.
#
#  Il ne l'est pas, parce qu'on ne retire rien : les champs effaçables
#  sont scellés CHIFFRÉS, et broyer détruit la clé du cas. Pas un octet
#  du DAG ne bouge.
#
#  Ce script vérifie les quatre choses qui font que ce n'est pas du
#  théâtre :
#    1. le critère lui-même, sur le manifest-root AVANT et APRÈS ;
#    2. le VÉRIFIEUR TIERS zéro-dépendance re-prouve encore — sans quoi
#       la promesse du produit tomberait le jour où l'on exerce un droit ;
#    3. le clair n'est pas dans les octets publiés ;
#    4. le garde : UN seul point de scellement, aucun joker de `match`.
#
#  Usage :  bash scripts/m6-check.sh
#  Sortie 0 si la conservation tient ; ≥ 1 sinon.
# ─────────────────────────────────────────────────────────────────────
set -euo pipefail
cd "$(dirname "$0")/.."

ok()   { printf '  \033[32m✓\033[0m %s\n' "$*"; }
ko()   { printf '  \033[31m✗\033[0m %s\n' "$*"; exit 1; }
titre(){ printf '\n\033[1m%s\033[0m\n' "$*"; }

titre "1. Le critère : « un cas expiré disparaît, et tout le reste se re-prouve encore »"
cargo test -q -p provenance-capture --test m6_conservation_limitee >/dev/null 2>&1 \
  || ko "le critère de M6 ne passe pas"
ok "le DAG est octet pour octet identique après le broyage, et le contenu est parti"

titre "2. Le vérifieur TIERS zéro-dépendance re-prouve une chaîne broyée"
cargo test -q -p verify-mini --test m6_broyage_invisible_au_verifieur >/dev/null 2>&1 \
  || ko "verify-mini ne re-prouve plus après broyage — la promesse du produit tomberait"
ok "même verdict, même manifest-root : le broyage lui est invisible"

titre "3. Le garde : un seul point de scellement, aucun joker"
cargo test -q -p provenance-capture --test m6_un_seul_scellement >/dev/null 2>&1 \
  || ko "le garde de M6 est tombé"
ok "un seul \`self.dag.append(\` dans le store — aucun tap ne peut oublier d'enrober"
ok "aucun joker de \`match\` : une variante de \`Node\` ajoutée casse la compilation"

titre "4. La primitive et sa règle la plus fragile"
cargo test -q -p provenance-dag --lib shred >/dev/null 2>&1 || ko "la primitive de broyage est cassée"
ok "aucune clé de cas n'est dérivée — une clé dérivée se re-dérive, la broyer ne détruirait rien"

# NOTE — ce script EXÉCUTE les gardes, il ne les RÉ-IMPLÉMENTE pas.
#
# La première version faisait ici son propre `grep -q 'fn derive'`. Il tombait sur la LISTE
# D'INTERDITS du garde lui-même (`for interdit in ["fn derive", …]`) : un second analyseur, plus
# faible que le premier, qui refusait du code correct. C'est le défaut que M1 a passé un module
# entier à supprimer — cinq analyseurs d'autorité dont un seul était juste. On n'en rouvre pas un
# ici pour le confort d'un script.

titre "5. La démonstration, à l'œil"
SORTIE=$(cargo run -q -p provenance-capture --example m6_conservation 2>&1)
AVANT=$(printf '%s' "$SORTIE" | grep -m1 -A1 'AVANT le broyage' | grep -oE '[0-9a-f]{64}')
APRES=$(printf '%s' "$SORTIE" | grep -m1 -A1 'APRÈS le broyage' | grep -oE '[0-9a-f]{64}')
[ -n "$AVANT" ] && [ "$AVANT" = "$APRES" ] \
  || ko "les racines diffèrent : avant=$AVANT après=$APRES"
ok "manifest-root avant = après = ${AVANT:0:16}…"
printf '%s' "$SORTIE" | grep -q 'détruit à t=' || ko "aucun contenu n'a été détruit — la démo serait vide"
ok "et le contenu des dossiers expirés est détruit"

titre "Ce que M6 NE fait PAS, et qu'il faut redire"
cat <<'FIN'
  · Le NŒUD reste. Un auditeur voit toujours qu'un acte a eu lieu, son rôle,
    sa date, ses arêtes — il ne peut plus lire ce qu'il disait. C'est la
    limitation de conservation, pas l'effacement d'une trace.
  · Rien ne protège le PASSÉ : un record scellé avant l'activation d'un
    trousseau porte son contenu en clair pour toujours.
  · Ce n'est pas une PREUVE de destruction : sauvegardes et instantanés
    peuvent conserver une clé ailleurs. La discipline des copies reste une
    affaire d'exploitation.
  · Le trousseau ne se recharge pas au redémarrage — sa persistance est le
    geste du déployeur. Un trousseau perdu se lit « clé absente », jamais
    « broyé » : on n'affirme pas une destruction qu'on n'a pas faite.
FIN

printf '\n\033[32mM6 tient.\033[0m\n'
