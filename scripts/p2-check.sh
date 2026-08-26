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

titre "6. La commande d'exploitation — un cycle complet, sans jamais nommer la génération"
CONS=(cargo run -q -p scella-conservation --bin scella-conserver --)
TM="$TRAVAIL/generations"
set +e
"${CONS[@]}" poser --trousseau-maitre "$TM" --numero 1 --cle-hex "$CLE" --a 100 >/dev/null 2>&1
[ $? -eq 0 ] && ok "poser : la génération 1 entre en service" || ko "poser a échoué"
"${CONS[@]}" sauvegarder --records "$TRAVAIL/archive/records.jsonl" --vers "$TRAVAIL/arch-cli" \
    --tenant acme --trousseau-maitre "$TM" --a 200 >/dev/null 2>&1
[ $? -eq 0 ] && ok "sauvegarder : la génération vient du TROUSSEAU, pas de la mémoire de l'exploitant" \
             || ko "sauvegarder a échoué"
grep -q 'generation	1' "$TRAVAIL/arch-cli/MANIFESTE" 2>/dev/null \
  && ok "le manifeste porte la génération que le trousseau connaissait" \
  || ko "le manifeste n'annonce pas la génération 1"
"${CONS[@]}" tourner --trousseau-maitre "$TM" --numero 2 --cle-hex "$(printf '07%.0s' {1..32})" --a 300 >/dev/null 2>&1
[ $? -eq 0 ] && ok "tourner : la 2 scelle, la 1 est retirée" || ko "tourner a échoué"
"${CONS[@]}" restaurer --archive "$TRAVAIL/arch-cli" --trousseau-maitre "$TM" >/dev/null 2>&1
[ $? -eq 0 ] && ok "APRÈS ROTATION, l'archive ancienne remonte encore (rien n'a été détruit)" \
             || ko "la rotation a cassé une archive ancienne"
"${CONS[@]}" detruire --trousseau-maitre "$TM" --numero 1 --a 400 >/dev/null 2>&1
[ $? -eq 1 ] && ok "détruire SANS --je-confirme est refusé (1), et ne touche à rien" \
             || ko "une destruction non confirmée aurait dû être refusée"
"${CONS[@]}" detruire --trousseau-maitre "$TM" --numero 1 --a 400 --je-confirme >/dev/null 2>&1
[ $? -eq 0 ] && ok "détruire --je-confirme : la clé quitte le fichier" || ko "la destruction a échoué"
grep -q 'DETRUITE' "$TM" && ok "et la pierre tombale DATÉE reste (« détruite » ≠ « jamais vue »)" \
                         || ko "la date de destruction a disparu"
SORTIE=$("${CONS[@]}" restaurer --archive "$TRAVAIL/arch-cli" --trousseau-maitre "$TM" 2>&1); CODE=$?
set -e
[ "$CODE" -eq 1 ] && ok "restaurer sous une génération détruite : refus MOTIVÉ (1), pas erreur (2)" \
                  || ko "attendu 1, obtenu $CODE"
echo "$SORTIE" | grep -q 'DÉTRUITE le 400' \
  && ok "et le message porte la DATE, sans jamais dire « invalide »" \
  || ko "le refus n'annonce pas la date de destruction"

titre "7. Ce que l'ANCRE achète — la démonstration qui justifie tout le reste"
echo "      L'adversaire a re-scellé la chaîne ET réécrit le manifeste pour concorder."
set +e
"${CONS[@]}" restaurer --archive "$TRAVAIL/archive.RESCELLEE" --cle-hex "$CLE" >/dev/null 2>&1
CODE_SANS=$?
SORTIE_AVEC=$("${CONS[@]}" restaurer --archive "$TRAVAIL/archive.RESCELLEE" --cle-hex "$CLE" \
    --journal "$TRAVAIL/ancres.journal" 2>&1); CODE_AVEC=$?
set -e
[ "$CODE_SANS" -eq 0 ] \
  && ok "SANS ancre : l'archive falsifiée passe (0) — c'est la limite, énoncée et démontrée" \
  || ko "attendu 0 sans ancre, obtenu $CODE_SANS"
[ "$CODE_AVEC" -eq 1 ] \
  && ok "AVEC l'ancre publiée : elle est attrapée (1)" \
  || ko "attendu 1 avec ancre, obtenu $CODE_AVEC"
echo "$SORTIE_AVEC" | grep -q "L'ANCRE PUBLIÉE CONTREDIT" \
  && ok "et le verdict nomme la raison : le manifeste voyage avec l'archive, l'ancre non" \
  || ko "le message ne désigne pas l'ancre"

titre "Verdict"
if [ "$ECHECS" -eq 0 ]; then
    printf '  \033[0;32mP2 tient : détruit, restauré, re-prouvé par un tiers, altération démasquée,\n  cycle d'"'"'exploitation complet, et le re-scellement attrapé par l'"'"'ancre.\033[0m\n\n'
    exit 0
fi
printf '  \033[0;31m%s contrôle(s) en échec.\033[0m\n\n' "$ECHECS"
exit 1
