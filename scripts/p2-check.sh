#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
#  P2 — la conservation opérable, CONTRÔLÉE.
#
#  Le critère de la feuille de route est littéral : « un store détruit
#  puis restauré, dont `verify-mini` re-prouve la chaîne ». Deux mots y
#  font tout le travail, et ce script refuse de les contourner :
#
#    · DÉTRUIT — le dossier du store est effacé du disque entre la
#      sauvegarde et la restauration. Une relecture sans destruction ne
#      prouverait que la sérialisation.
#
#    · verify-mini — la re-preuve est faite par le binaire à
#      dépendances NULLES, pas par la bibliothèque qui vient d'écrire
#      l'archive. Un module qui se vérifie lui-même ne vérifie rien.
#
#  Et comme un contrôle qui ne sait que dire « vert » ne vaut rien, il
#  exige aussi le ROUGE : une copie altérée d'un seul champ scellé — le
#  verdict de criblage, retourné de PASS à FAIL — doit sortir 1 et
#  NOMMER le record fautif.
#
#  Le script ne croit pas l'exemple sur parole : il recompte les records
#  à la main (`grep -c`) et relit la racine dans le manifeste, comme le
#  ferait un auditeur qui ne dispose que des fichiers.
#
#  Usage :  bash scripts/p2-check.sh
#  Sortie 0 si les quatre étapes tiennent ; ≥ 1 sinon.
# ─────────────────────────────────────────────────────────────────────
set -euo pipefail
cd "$(dirname "$0")/.."

TRAVAIL=$(mktemp -d)
trap 'rm -rf "$TRAVAIL"' EXIT

CLE=2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a
ECHECS=0

titre() { printf '\n\033[1;34m%s\033[0m\n' "$1"; }
ok()    { printf '  \033[0;32m✓\033[0m %s\n' "$1"; }
ko()    { printf '  \033[0;31m✗\033[0m %s\n' "$1"; ECHECS=$((ECHECS + 1)); }

titre "1. Sceller, archiver, DÉTRUIRE le store, restaurer"
if ! cargo run -q -p scella-conservation --example p2_conservation -- "$TRAVAIL" > "$TRAVAIL/journal" 2>&1; then
    ko "l'exemple a échoué"; sed 's/^/      /' "$TRAVAIL/journal"; exit 1
fi
sed 's/^/      /' "$TRAVAIL/journal"
[ -d "$TRAVAIL/store" ] && ko "le store existe encore — il devait être détruit" || ok "le store a bien été détruit"
[ -f "$TRAVAIL/archive/records.jsonl" ] && ok "l'archive porte ses records" || ko "records.jsonl absent"
[ -f "$TRAVAIL/archive/MANIFESTE" ]     && ok "l'archive porte son manifeste" || ko "MANIFESTE absent"

titre "2. Le manifeste, relu à la main"
RACINE=$(awk -F'\t' '$1=="racine"{print $2}' "$TRAVAIL/archive/MANIFESTE")
NOEUDS=$(awk -F'\t' '$1=="noeuds"{print $2}' "$TRAVAIL/archive/MANIFESTE")
REELS=$(grep -c '"seal"' "$TRAVAIL/archive/records.jsonl")
printf '      racine annoncée : %s\n      noeuds annoncés : %s · comptés : %s\n' "$RACINE" "$NOEUDS" "$REELS"
[ "$NOEUDS" = "$REELS" ] && ok "le compte annoncé est le compte réel" || ko "compte annoncé $NOEUDS ≠ réel $REELS"
[ ${#RACINE} -eq 64 ]    && ok "la racine a 64 caractères"            || ko "racine mal formée"

titre "3. La clé n'est PAS dans l'archive"
if grep -qi "$CLE" "$TRAVAIL/archive/records.jsonl" "$TRAVAIL/archive/MANIFESTE" 2>/dev/null; then
    ko "la clé apparaît dans l'archive — l'invariant est rompu"
else
    ok "aucun octet de la clé dans les fichiers remis"
fi

titre "4. La re-preuve, par un AUTRE binaire (verify-mini, zéro dépendance)"
if cargo run -q -p verify-mini -- "$TRAVAIL/archive/records.jsonl" --key-hex "$CLE" --root "$RACINE" > "$TRAVAIL/vm.out" 2>&1; then
    sed 's/^/      /' "$TRAVAIL/vm.out"
    ok "la chaîne RESTAURÉE se re-prouve (sortie 0), racine concordante"
else
    sed 's/^/      /' "$TRAVAIL/vm.out"; ko "la chaîne restaurée ne se re-prouve pas"
fi

titre "5. Le rouge — sans lui, le vert ne dit rien"
set +e
cargo run -q -p verify-mini -- "$TRAVAIL/records.ALTERE.jsonl" --key-hex "$CLE" --root "$RACINE" > "$TRAVAIL/vm.ko" 2>&1
CODE=$?
set -e
sed 's/^/      /' "$TRAVAIL/vm.ko"
[ "$CODE" -eq 1 ] && ok "l'altération d'UN champ scellé sort 1" || ko "attendu 1, obtenu $CODE"
grep -q 'cassé au record' "$TRAVAIL/vm.ko" && ok "et le record fautif est NOMMÉ" || ko "l'échec ne désigne aucun record"

titre "Verdict"
if [ "$ECHECS" -eq 0 ]; then
    printf '  \033[0;32mP2 tient : détruit, restauré, re-prouvé par un tiers, et l'"'"'altération démasquée.\033[0m\n\n'
    exit 0
fi
printf '  \033[0;31m%s contrôle(s) en échec.\033[0m\n\n' "$ECHECS"
exit 1
