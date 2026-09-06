#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
#  D3 — la licence dit ce que les manifestes déclarent, et aucun secret
#       ne dort dans un fichier suivi.
#
#  ## Pourquoi ce contrôle existe
#
#  Le dépôt portait une contradiction que personne ne pouvait voir parce
#  que rien ne la cherchait : `crates/efe-nano-sdk/Cargo.toml` déclarait
#  `license = "MIT"` depuis M50, son `README.md` renvoyait à un
#  « workspace LICENSE-MIT » — et **ce fichier n'existait pas**. Pendant
#  ce temps le `LICENSE` racine posait une règle qui rendait propriétaire
#  tout ce qui ne vit pas sous `/clients/<lang>/`. Deux documents, deux
#  réponses, sur le même crate.
#
#  Fournir le texte est la SEULE condition que MIT impose. Elle n'était
#  pas remplie, et aucune porte ne le disait.
#
#  ## Les trois états
#     0  mesuré, la licence est cohérente et rien ne fuit
#     1  mesuré, l'un des deux tombe
#     3  NON MESURÉ — l'instrument n'a pas pu regarder
#
#  Usage :  bash scripts/d3-check.sh
# ─────────────────────────────────────────────────────────────────────
set -uo pipefail
cd "$(dirname "$0")/.."

ok()    { printf '  \033[32m✓\033[0m %s\n' "$*"; }
ko()    { printf '  \033[31m✗\033[0m %s\n' "$*"; ECHECS=$((ECHECS+1)); }
note()  { printf '      %s\n' "$*"; }
titre() { printf '\n\033[1;34m%s\033[0m\n' "$*"; }
nonmesure() {
  printf '\n\033[1mNON MESURÉ\033[0m\n  %s\n' "$1"
  [ $# -gt 1 ] && printf '  → %s\n' "$2"
  printf '\n  Ni un succès ni un échec : l'\''instrument n'\''a pas pu regarder.\n\n'
  exit 3
}

ECHECS=0
CONTROLES=0
# Le compte réel : MIT (2 assertions) + UNLICENSED (1) + le renvoi du README (1)
# + les deux modes de gitleaks (2) = 6. Un plancher au-dessus du compte réel fait
# rendre NON MESURÉ à un contrôle parfaitement exécuté — c'est arrivé à la première
# version de ce fichier, et c'était le garde qui avait raison contre son auteur.
PLANCHER=7

exiger() { CONTROLES=$((CONTROLES+1)); if [ "$2" = "0" ]; then ok "$1"; else ko "$1"; fi; }

printf '\033[1mD3 · la licence, et les secrets\033[0m\n'

# ═════════════════════════════════════════════════════════════════════
titre "1. La licence dit-elle ce que les manifestes déclarent ?"
# ═════════════════════════════════════════════════════════════════════
[ -f Cargo.toml ] || nonmesure "Cargo.toml introuvable — on n'est pas à la racine."

# La licence du workspace, à laquelle se résout `{ workspace = true }`.
# ⚠ `sed 's/.*= *//'` est GLOUTON : sur `license = { workspace = true }` il mange
# jusqu'au DERNIER `=` et rend « true } ». La première version de ce contrôle a donc
# vu une licence nommée « true } » et l'a signalée comme inconnue — ce qui était le
# bon réflexe sur une mauvaise lecture. On extrait la valeur CITÉE, ou rien.
licence_de() {  # $1 = chemin d'un Cargo.toml → la licence déclarée, ou « HERITEE »
  local l
  l=$(grep -m1 '^license' "$1" 2>/dev/null) || return 1
  case "$l" in
    *workspace*=*true*) printf 'HERITEE' ;;
    *) printf '%s' "$l" | sed -E 's/^license[[:space:]]*=[[:space:]]*"([^"]*)".*/\1/' ;;
  esac
}
WS=$(licence_de Cargo.toml)
[ -n "$WS" ] || nonmesure "aucune ligne \`license\` dans [workspace.package]."
note "workspace : $WS"

# Chaque crate, licence résolue. Un crate qui hérite compte pour la licence du
# workspace — c'est ce que cargo publie, et donc ce qu'un tiers lit.
LU=0
DECLAREES="$WS"
for f in crates/*/Cargo.toml; do
  L=$(licence_de "$f") || continue
  [ -z "$L" ] && continue
  LU=$((LU+1))
  [ "$L" = "HERITEE" ] && continue     # se résout à la licence du workspace
  DECLAREES="$DECLAREES
$L"
done

# Plancher anti-vacuité : un glob qui ne trouve rien rendrait « aucune licence
# incohérente » — un vert parfait sur une mesure qui n'a pas eu lieu.
[ "$LU" -ge 25 ] || nonmesure \
  "seulement $LU manifeste(s) de crate lu(s), 25 attendus au minimum." \
  "le glob \`crates/*/Cargo.toml\` ne trouve plus le périmètre."
note "$LU manifestes lus"

DISTINCTES=$(printf '%s\n' "$DECLAREES" | sort -u | grep -v '^$')
note "licences distinctes : $(printf '%s' "$DISTINCTES" | tr '\n' ' ')"

# Pour chacune, le TEXTE doit exister et le LICENSE racine doit la nommer.
while IFS= read -r lic; do
  [ -z "$lic" ] && continue
  case "$lic" in
    UNLICENSED)
      { [ -f LICENSE ] && grep -q "PROPRIETARY LICENSE" LICENSE; }
      exiger "« $lic » → ./LICENSE porte le texte propriétaire" $? ;;
    MIT)
      { [ -f LICENSE-MIT ] && grep -q "MIT License" LICENSE-MIT; }
      exiger "« $lic » → ./LICENSE-MIT existe et porte le texte MIT" $?
      grep -q "LICENSE-MIT" LICENSE
      exiger "« $lic » → ./LICENSE la NOMME (sinon deux documents, deux vérités)" $? ;;
    *)
      exiger "« $lic » → aucune règle : licence déclarée que ce contrôle ne sait pas couvrir" 1
      note "ajouter sa branche ici plutôt que la laisser passer en silence." ;;
  esac
done <<< "$DISTINCTES"

# Le renvoi du crate doit résoudre. C'est LUI qui était cassé : le README disait
# « see workspace LICENSE-MIT » et le fichier n'existait pas.
if [ -f crates/efe-nano-sdk/README.md ]; then
  if grep -q "LICENSE-MIT" crates/efe-nano-sdk/README.md; then
    [ -f LICENSE-MIT ]
    exiger "le renvoi de crates/efe-nano-sdk/README.md résout" $?
  fi
fi

# ═════════════════════════════════════════════════════════════════════
titre "2. Aucun secret ne dort dans un fichier suivi"
# ═════════════════════════════════════════════════════════════════════
command -v gitleaks >/dev/null 2>&1 || nonmesure \
  "gitleaks n'est pas installé." "brew install gitleaks — la moitié « licence » ci-dessus a bien été mesurée."
[ -f .gitleaks.toml ] || nonmesure ".gitleaks.toml absent : la mesure porterait sur d'autres règles que celles du dépôt."

# DEUX modes, et il faut les deux : `--no-git` regarde ce qui est LIVRÉ,
# `detect` regarde l'HISTORIQUE — c'est celui que la CI et le hook pre-commit
# lancent. Une allowlist peut être juste pour l'un et rouvrir l'autre : retirer
# trois chemins parce que leurs fichiers avaient disparu a rouvert quatre
# signalements dans des commits.
gitleaks detect --no-git --no-banner >/dev/null 2>&1
exiger "arbre de travail : aucun secret" $?
gitleaks detect --no-banner >/dev/null 2>&1
exiger "historique git : aucun secret" $?

# ═════════════════════════════════════════════════════════════════════
titre "3. Les portes d'entrée citent-elles des chemins qui existent ?"
# ═════════════════════════════════════════════════════════════════════
# D3 lisait les MANIFESTES, pas la prose — et c'est par la prose que la
# contradiction est revenue : `SECURITY.md` accordait « open source under the
# Apache License 2.0 » quand `LICENSE` dit `All rights reserved`, et citait
# SEPT documents de conformité dont aucun n'existait. `CONTRIBUTING.md`
# renvoyait à un ADR, un CHANGELOG et un CODEOWNERS tous absents.
#
# Ce garde ne relit pas la prose (aucune règle syntaxique ne sépare un chemin
# d'un identifiant de code entre accents graves — sa propre documentation le
# dit). Il tient la moitié qui est mécanisable : tout lien markdown relatif des
# quatre portes d'entrée doit résoudre. On l'EXÉCUTE plutôt que d'en réécrire
# ici une version affaiblie — la leçon de `m6-check.sh`.
JOURNAL=$(mktemp)
cargo test -q -p audit-console --test d1_portes_dentree >"$JOURNAL" 2>&1
PORTES=$?
exiger "les 4 portes d'entrée : chemins existants ET structure intacte (7 tests, contre-épreuves comprises)" "$PORTES"
# Le journal ne sort que si CE contrôle-CI a échoué. La première écriture testait `$ECHECS`, le
# compteur GLOBAL : elle recrachait donc le journal d'une section verte dès qu'une AUTRE section
# était tombée — un instrument qui montre la mauvaise preuve envoie chercher au mauvais endroit.
[ "$PORTES" = "0" ] || sed 's/^/      /' "$JOURNAL" | tail -12
rm -f "$JOURNAL"

# ═════════════════════════════════════════════════════════════════════
titre "4. Le verdict"
# ═════════════════════════════════════════════════════════════════════
[ "$CONTROLES" -ge "$PLANCHER" ] || nonmesure \
  "seulement $CONTROLES contrôle(s) exécutés, $PLANCHER attendus au minimum." \
  "une branche a été sautée : le verdict porterait sur moins que le critère."

printf '  %s contrôle(s) · %s échec(s)\n\n' "$CONTROLES" "$ECHECS"
if [ "$ECHECS" = "0" ]; then
  printf '\033[32mD3 TIENT.\033[0m  Chaque licence déclarée a son texte, le LICENSE la nomme,\n'
  printf 'ni l'\''arbre ni l'\''historique ne portent de secret, et les quatre portes\n'
  printf 'd'\''entrée ne renvoient qu'\''à des chemins qui existent.\n\n'
  printf '  ⚠ CE QUE CECI NE PROUVE PAS : que les exemptions de `.gitleaks.toml` soient\n'
  printf '  JUSTES. Elles portent chacune leur raison écrite ; les relire est un travail\n'
  printf '  humain, qu'\''aucun compteur ne remplace.\n\n'
  exit 0
fi
printf '\033[31mD3 TOMBE — %s échec(s) sur %s.\033[0m\n\n' "$ECHECS" "$CONTROLES"
exit 1
