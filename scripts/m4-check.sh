#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
#  M4 — ce qui se vérifie SANS Docker.
#
#  Le critère de M4 est « machine vierge, `docker compose up` → un reçu
#  re-prouvé, sans clé ». Il se sépare en deux moitiés :
#
#    · la CHARGE UTILE — la passerelle sert un reçu, le pilote l'exporte,
#      le confronte à l'ancre et démasque une troncature. Cette moitié
#      s'exécute ici, sans conteneur : c'est le même code, les mêmes
#      binaires, la même conversation HTTP.
#
#    · l'EMBALLAGE — l'image, le volume partagé, le réseau de compose.
#      Cette moitié exige un démon Docker.
#
#  Ce script prouve la première et ne prétend rien de la seconde.
#
#  Il ne se contente pas de croire le pilote : il REFAIT ses comptages à
#  la main, avec `grep -c` et `awk`, sur les fichiers qu'il a laissés.
#  C'est précisément ce qu'un auditeur ferait, et c'est ce qui autorise le
#  pilote à ne pas être auditable lui-même.
#
#  Usage :  bash scripts/m4-check.sh
#  Sortie 0 si la charge utile tient ; ≥ 1 sinon.
# ─────────────────────────────────────────────────────────────────────
set -euo pipefail
cd "$(dirname "$0")/.."

PORT=18211
TRAVAIL=$(mktemp -d)
trap 'rm -rf "$TRAVAIL"; kill "${GW:-}" 2>/dev/null || true' EXIT

echo "═══ 1/4 · compilation des deux binaires ═══"
# En debug : ce script éprouve une LOGIQUE, pas une performance. L'image, elle,
# compile en release — le comportement testé ici est le même.
cargo build -q -p efe-swarm-gateway --bin efe-swarm-gateway
cargo build -q -p scella-preuve
echo "  passerelle + pilote — compilés."

echo
echo "═══ 2/4 · la passerelle, avec le profil docker ═══"
# Le profil du dépôt, celui que T2 vérifie — deux valeurs seulement sont
# remplacées, et pour des raisons qui ne touchent pas à ce qu'on éprouve :
# le port (éviter une collision) et le chemin de l'ancre (pas de /preuve ici).
set -a
# shellcheck disable=SC1091
. ./profiles/docker.env
set +a
export EFE_SWARM_GATEWAY_ADDR="127.0.0.1:${PORT}"
export EFE_SWARM_ANCHOR_LOG="${TRAVAIL}/anchors.log"

./target/debug/efe-swarm-gateway > "${TRAVAIL}/passerelle.log" 2>&1 &
GW=$!
echo "  démarrée sur 127.0.0.1:${PORT} (pid ${GW})"

# M0 doit annoncer la dégradation : sans clé, le reçu ne prouve rien contre
# personne, et une démonstration qui le tairait serait une démonstration qui ment.
for _ in $(seq 1 60); do grep -q 'provenance' "${TRAVAIL}/passerelle.log" 2>/dev/null && break; sleep 0.5; done
if ! grep -qE 'provenance +DÉGRADÉE' "${TRAVAIL}/passerelle.log"; then
  echo "ÉCHEC : le rapport de démarrage n'annonce PAS « provenance DÉGRADÉE » alors" >&2
  echo "        qu'aucune clé n'a été fournie — la démonstration se surévaluerait." >&2
  exit 1
fi
echo "  rapport M0 : « provenance DÉGRADÉE » — annoncé, comme il se doit."

echo
echo "═══ 3/4 · le pilote conduit la démonstration ═══"
SCELLA_PASSERELLE="127.0.0.1:${PORT}" \
SCELLA_ANCRE="${TRAVAIL}/anchors.log" \
SCELLA_SORTIE="${TRAVAIL}/preuve" \
  ./target/debug/scella-preuve | sed 's/^/  │ /'

echo
echo "═══ 4/4 · on REFAIT ses comptages à la main ═══"
# Le pilote n'a pas à être cru. Voici la même mesure, avec les outils d'un
# auditeur — si les deux divergeaient, c'est le pilote qui mentirait.
# L'ancre est choisie PAR LE RUN, jamais par position. Ici le journal n'a qu'une ligne
# — une seule passerelle, un seul run — et `END` marchait donc. Mais c'était le
# troisième endroit du dépôt à enseigner « la dernière ligne », après le pilote et la
# page qu'on tend, qui ont tous deux cessé : sous le profil `jonction` deux passerelles
# publient dans le même journal, et « la dernière » devient celle du hasard. Un contrôle
# qui montre un geste faux le propage, même là où son contexte le rend inoffensif.
RUN_ID=$(head -1 "${TRAVAIL}/preuve/bundle.sealed.jsonl" \
  | python3 -c 'import sys,json;print(json.load(sys.stdin)["node_id"].split("::")[0])')
ANNONCE=$(awk -F'\t' -v r="$RUN_ID" '$2==r {print $5}' "${TRAVAIL}/anchors.log")
INTACT=$(grep -c '[^[:space:]]' "${TRAVAIL}/preuve/bundle.sealed.jsonl")
TRONQUE=$(grep -c '[^[:space:]]' "${TRAVAIL}/preuve/bundle.TRONQUE.jsonl")

echo "  awk sur l'ancre                : ${ANNONCE}"
echo "  grep -c sur le bundle intact   : ${INTACT}"
echo "  grep -c sur le bundle tronqué  : ${TRONQUE}"

[ "$INTACT" = "$ANNONCE" ] || { echo "ÉCHEC : l'intact ne concorde pas avec l'ancre." >&2; exit 1; }
[ "$TRONQUE" != "$ANNONCE" ] || { echo "ÉCHEC : la troncature n'est pas détectable." >&2; exit 1; }
[ "$TRONQUE" -lt "$INTACT" ] || { echo "ÉCHEC : le tronqué n'est pas plus court." >&2; exit 1; }

echo
echo "═══ CHARGE UTILE DE M4 : OK ═══"
echo "  · la passerelle démarre sur le profil docker du dépôt et ANNONCE sa dégradation ;"
echo "  · un reçu est servi, exporté, et son ancre publiée hors bande ;"
echo "  · une troncature est démasquée par un comptage — sans clé, sans crypto ;"
echo "  · et les comptages du pilote se refont à la main, avec awk et grep -c."
echo
echo "  NON vérifié ici : l'image, le volume et le réseau de compose — ils exigent"
echo "  un démon Docker. \`docker compose config\` valide la composition ; le build"
echo "  et le \`up\` restent à exécuter sur une machine qui a Docker."
