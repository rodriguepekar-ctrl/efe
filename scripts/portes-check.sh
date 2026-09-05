#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
#  LES PORTES — la qualité, sur CHAQUE profil réellement servi.
#
#  ## Pourquoi ce fichier existe
#
#  `cargo clippy --workspace --all-targets -- -D warnings` compile les
#  features PAR DÉFAUT. C'est le constat de É3, et É4 en a mesuré le
#  prix : sous `postgres,oidc,pii,frontier-llm`, **sept défauts**
#  dormaient dans le chemin d'egress payant pendant que la porte du
#  dépôt était verte — dont une paire d'attributs `#[cfg]`/`#[test]`
#  orpheline que le rapport de P1 citait déjà comme la preuve que ce
#  chemin vivait hors de la rigueur du dépôt. Aucune porte ne pouvait
#  la voir, puisque aucune porte ne compilait cette feature.
#
#  É4 a fermé le profil de production, dans `p1-check.sh`. Restait la
#  JONCTION — `reg-evidence`, c'est-à-dire **la moitié Registre du
#  produit** — que rien ne gardait.
#
#  ## Ce que ce script ne fait pas : deviner la liste
#
#  Les profils ne sont pas écrits ici. Ils sont **lus dans
#  `compose.yaml`**, qui est le fichier qui décide de ce qui est
#  CONSTRUIT et SERVI. Ajouter un profil au compose sans le garder
#  devient donc impossible : la porte suit le produit, au lieu d'être
#  une liste parallèle qui diverge en silence — le défaut que ce dépôt
#  a déjà payé plusieurs fois.
#
#  ## Ce qu'il ne mesure PAS, et pourquoi
#
#  · `cargo fmt --check`. Ce dépôt est formaté À LA MAIN : mesuré le
#    4 septembre 2026, `cargo fmt --all -- --check` réécrirait
#    **4 643 fichiers**. C'est aussi la raison pour laquelle `ci.yml`
#    ne pourrait pas passer même s'il se déclenchait : son job
#    `fast-checks` met `rustfmt` AVANT `clippy`, et tomberait sans
#    jamais atteindre la porte que ce script mesure.
#
#  · Les features qu'aucun profil servi n'emploie — `demo`,
#    `tamper-testkit`, `chaos`, `nats-bus`, `local-slm`, `metrics`,
#    `integration-tests`, `test-util`, `fastembed-real`. Les garder
#    reviendrait à exiger la compilation de code que le produit
#    n'embarque pas.
#    ⚠ L'une d'elles est CASSÉE : `fastembed-real` ne compile pas
#    (`E0599` — la variante `BGEM3` n'existe plus dans `fastembed
#    4.9.1`, où les modèles BGE disponibles sont la famille v1.5).
#    Elle n'est activée par personne. La réparer demande de CHOISIR un
#    modèle de remplacement : c'est une décision produit, pas un lint,
#    et la prendre en silence serait un vert de substitution.
#
#  ## Trois états
#     0  mesuré, toutes les portes passent
#     1  mesuré, au moins une tombe
#     3  NON MESURÉ — l'instrument n'a pas pu regarder
#
#  Usage :  bash scripts/portes-check.sh
# ─────────────────────────────────────────────────────────────────────
set -uo pipefail
cd "$(dirname "$0")/.."

ok()   { printf '  \033[32m✓\033[0m %s\n' "$*"; }
ko()   { printf '  \033[31m✗\033[0m %s\n' "$*"; }
note() { printf '      %s\n' "$*"; }
titre(){ printf '\n\033[1;34m%s\033[0m\n' "$*"; }

nonmesure() {
  printf '\n\033[1mNON MESURÉ\033[0m\n  %s\n' "$1"
  [ $# -gt 1 ] && printf '  → %s\n' "$2"
  printf '\n  Ni un succès ni un échec : l'\''instrument n'\''a pas pu regarder.\n\n'
  exit 3
}

JOURNAL="$(mktemp)"
trap 'rm -f "$JOURNAL"' EXIT

PAQUET="efe-swarm-gateway"
ECHECS=0
PORTES=0

printf '\033[1mLES PORTES — la qualité sur chaque profil servi\033[0m\n'

# ═════════════════════════════════════════════════════════════════════
titre "0. Les profils, LUS dans compose.yaml"
# ═════════════════════════════════════════════════════════════════════
command -v cargo >/dev/null 2>&1 || nonmesure "aucun \`cargo\` sur le PATH."
[ -f compose.yaml ] || nonmesure "compose.yaml introuvable — impossible de savoir ce qui est servi."

# `CARGO_FEATURES: <liste>` sous `build.args`. Le profil par défaut (le coin) n'en
# porte pas : c'est précisément ce qui le définit, et il est ajouté à la main ci-dessous.
JEUX="$(grep -E '^\s+CARGO_FEATURES:\s*\S' compose.yaml | sed -E 's/.*CARGO_FEATURES:\s*//' | tr -d '"' | sort -u)"
N_JEUX="$(printf '%s\n' "$JEUX" | grep -c '[^[:space:]]')"

# Plancher anti-vacuité : si l'extraction rendait zéro, ce script mesurerait le seul
# profil par défaut et se dirait vert — en ayant perdu exactement ce pour quoi il existe.
[ "$N_JEUX" -ge 2 ] || nonmesure \
  "seulement $N_JEUX jeu(x) de features lu(s) dans compose.yaml, 2 attendus au minimum." \
  "la forme de \`build.args.CARGO_FEATURES\` a changé — la porte suivrait le produit de moins près qu'elle ne le croit."

note "profil par défaut (le coin) : aucune feature"
printf '%s\n' "$JEUX" | while read -r j; do [ -n "$j" ] && note "profil servi : $j"; done
ok "$((N_JEUX + 1)) profil(s) à garder — la liste vient de compose.yaml, pas d'ici"

# ═════════════════════════════════════════════════════════════════════
titre "1. Le COIN — le workspace entier, features par défaut"
# ═════════════════════════════════════════════════════════════════════
# Sur le coin on garde TOUT le workspace : c'est le critère de É3, et c'est aussi le
# seul profil où l'ensemble des crates est atteignable.
PORTES=$((PORTES + 1))
if cargo clippy --locked --workspace --all-targets --message-format short \
     -- -D warnings > "$JOURNAL" 2>&1; then
  ok "clippy -D warnings — workspace, features par défaut"
else
  ko "clippy TOMBE sur le profil du coin"
  grep -E 'error|warning' "$JOURNAL" | grep -vE '^(error|warning): (could not|build failed)' \
    | head -12 | sed 's/^/      /'
  ECHECS=$((ECHECS + 1))
fi

# ═════════════════════════════════════════════════════════════════════
titre "2. Les profils SERVIS — un par jeu de features"
# ═════════════════════════════════════════════════════════════════════
# Ici on cible le seul paquet `efe-swarm-gateway` : ce sont SES features que
# `compose.yaml` allume, et `--workspace --features` n'a pas de sens (cargo refuse
# des features qui n'appartiennent pas à un membre nommé).
while read -r jeu; do
  [ -z "$jeu" ] && continue
  PORTES=$((PORTES + 1))
  if cargo clippy --locked -p "$PAQUET" --features "$jeu" --all-targets \
       --message-format short -- -D warnings > "$JOURNAL" 2>&1; then
    ok "clippy -D warnings — $jeu"
  else
    ko "clippy TOMBE sur « $jeu »"
    note "ce jeu de features est SERVI par compose.yaml : sa dette part en production."
    grep -E 'error|warning' "$JOURNAL" | grep -vE '^(error|warning): (could not|build failed)' \
      | head -12 | sed 's/^/      /'
    ECHECS=$((ECHECS + 1))
  fi
done <<< "$JEUX"

# ═════════════════════════════════════════════════════════════════════
titre "3. Le verdict"
# ═════════════════════════════════════════════════════════════════════
# Un contrôle qui n'exécute plus ses portes rend 0 en silence. Le compte est donc
# lui-même une assertion, et son échec est un NON MESURÉ — pas un succès.
[ "$PORTES" -ge 3 ] || nonmesure \
  "seulement $PORTES porte(s) jouée(s), 3 attendues au minimum." \
  "une branche a été sautée : le verdict porterait sur moins que le produit."

printf '  %s porte(s) · %s échec(s)\n\n' "$PORTES" "$ECHECS"
if [ "$ECHECS" = "0" ]; then
  printf '\033[32mLES PORTES TIENNENT.\033[0m  Chaque profil que compose.yaml construit passe\n'
  printf '`clippy -D warnings`, y compris ceux que la porte par défaut ne compile pas.\n\n'
  printf '  ⚠ CE QUE CECI NE PROUVE PAS :\n'
  printf '  · que les tests passent — c'\''est `cargo test --workspace`, et `r3-check.sh`\n'
  printf '    le joue trois fois d'\''affilée ;\n'
  printf '  · que le profil de production DÉMARRE — c'\''est `p1-check.sh`, qui exige un\n'
  printf '    vrai Postgres et tue le processus pour de bon ;\n'
  printf '  · le PROFIL de production, ni l'\''emballage : `p1-check.sh` et `e1-check.sh`\n'
  printf '    exigent un démon Docker, et la CI ne les joue pas.\n\n'
  printf '  Cette ligne disait « aucun des 90 workflows du dépôt ne lance un *-check.sh,\n'
  printf '  et ci.yml ne se déclenche que sur main/release/** ». C'\''était vrai jusqu'\''au\n'
  printf '  5 septembre 2026 ; D4 l'\''a rendu faux : `.github/workflows/portes.yml` joue ce\n'
  printf '  script et onze contrôles à chaque push sur `c1-c3`. Corrigé ici plutôt que\n'
  printf '  laissé vieillir — un contrôle dont le texte survit à ce qu'\''il décrit est le\n'
  printf '  défaut que ce dépôt attrape le plus souvent.\n\n'
  exit 0
fi
printf '\033[31mUNE PORTE AU MOINS TOMBE — %s sur %s.\033[0m\n\n' "$ECHECS" "$PORTES"
exit 1
