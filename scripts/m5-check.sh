#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
#  M5 — la jonction, éprouvée de bout en bout.
#
#  Le critère : « un run de la passerelle produit un dossier reg-evidence
#  SANS GESTE MANUEL ». Ce script le vérifie comme un exploitant le
#  ferait : il démarre la passerelle avec le profil « jonction », POSTe
#  UN run, et va voir sur le disque.
#
#  Il vérifie aussi les deux propriétés qui font la valeur du dossier :
#    · il ne prononce PAS la conformité (aucun « compliant ») ;
#    · il se RE-DÉRIVE à l'identique, ce qui est la contrepartie de la
#      décision de ne jamais faire tomber la requête d'un client pour un
#      artefact qu'on saura reconstituer.
#
#  Usage :  bash scripts/m5-check.sh
#  Sortie 0 si la jonction tient ; ≥ 1 sinon.
# ─────────────────────────────────────────────────────────────────────
set -euo pipefail
cd "$(dirname "$0")/.."

PORT=18233
TRAVAIL=$(mktemp -d)
trap 'rm -rf "$TRAVAIL"; kill "${GW:-}" 2>/dev/null || true' EXIT

echo "═══ 1/5 · compilation AVEC la feature ═══"
# La feature est le point : sans elle, le binaire du coin ne contient pas une ligne du registre.
cargo build -q -p efe-swarm-gateway --bin efe-swarm-gateway --features reg-evidence
echo "  passerelle + registre — compilés."

echo
echo "═══ 2/5 · démarrage avec le profil « jonction » ═══"
set -a
# shellcheck disable=SC1091
. ./profiles/jonction.env
set +a
export EFE_SWARM_GATEWAY_ADDR="127.0.0.1:${PORT}"
export EFE_SWARM_ANCHOR_LOG="${TRAVAIL}/anchors.log"
export EFE_SWARM_REG_EVIDENCE_DIR="${TRAVAIL}/reg"

./target/debug/efe-swarm-gateway > "${TRAVAIL}/passerelle.log" 2>&1 &
GW=$!
for _ in $(seq 1 60); do curl -sf "http://127.0.0.1:${PORT}/healthz" >/dev/null 2>&1 && break; sleep 0.5; done
curl -sf "http://127.0.0.1:${PORT}/healthz" >/dev/null 2>&1 \
  || { echo "ÉCHEC : la passerelle n'a pas démarré." >&2; tail -20 "${TRAVAIL}/passerelle.log" >&2; exit 1; }

# Le rapport M0 doit annoncer le pont SERVI : c'est la seule façon pour un exploitant de savoir
# que son binaire embarque bien le registre — la feature ne se voit pas autrement.
grep -qE 'pont réglementaire +SERVIE' "${TRAVAIL}/passerelle.log" \
  || { echo "ÉCHEC : le rapport n'annonce pas « pont réglementaire SERVIE »." >&2; exit 1; }
echo "  rapport M0 : « pont réglementaire SERVIE »."

echo
echo "═══ 3/5 · UN run — et rien d'autre ═══"
RUN=$(curl -sf -X POST "http://127.0.0.1:${PORT}/v1/swarm/run" \
        -H 'content-type: application/json' --data @examples/run.json)
ID=$(printf '%s' "$RUN" | python3 -c 'import json,sys; print(json.load(sys.stdin)["run_id"])')
RACINE=$(printf '%s' "$RUN" | python3 -c 'import json,sys; print(json.load(sys.stdin)["reg"]["manifest_root"])')
CADRE=$(printf '%s' "$RUN" | python3 -c 'import json,sys; print(json.load(sys.stdin)["reg"]["framework"])')
echo "  run              : ${ID}"
echo "  cadre            : ${CADRE}"
echo "  manifest-root    : ${RACINE:0:32}…"

echo
echo "═══ 4/5 · le dossier est SUR LE DISQUE, sans qu'on ait rappelé la passerelle ═══"
REG="${TRAVAIL}/reg/${ID}/reg"
[ -d "$REG" ] || { echo "ÉCHEC : aucun dossier en ${REG}" >&2; exit 1; }
ls -1 "$REG" | sed 's/^/  · /'
N=$(ls -1 "$REG" | wc -l | tr -d ' ')
[ "$N" -eq 3 ] || { echo "ÉCHEC : ${N} fichiers, 3 attendus (artefact + lecture + mode d'emploi)." >&2; exit 1; }

# La promesse la plus importante, vérifiée sur le FICHIER que lira un régulateur.
if grep -qi 'compliant' "${REG}/${CADRE}.md"; then
  echo "ÉCHEC : l'artefact prononce un verdict de conformité." >&2
  exit 1
fi
grep -q 'outillé ≠ conforme' "${REG}/${CADRE}.md" \
  || { echo "ÉCHEC : l'avertissement ne voyage pas AVEC le dossier." >&2; exit 1; }
echo "  ↑ aucun verdict de conformité ; l'avertissement voyage avec le dossier."

echo
echo "═══ 5/5 · un dossier manqué se rattrape À L'IDENTIQUE ═══"
# C'est ce qui rend défendable de ne JAMAIS faire tomber la requête d'un client pour un dossier.
rm -rf "${TRAVAIL}/reg/${ID}"
RE=$(curl -sf -X POST "http://127.0.0.1:${PORT}/v1/swarm/reg/${ID}" -H 'x-efe-roles: auditor')
RACINE2=$(printf '%s' "$RE" | python3 -c 'import json,sys; print(json.load(sys.stdin)["manifest_root"])')
[ "$RACINE" = "$RACINE2" ] || { echo "ÉCHEC : re-dérivation divergente ($RACINE ≠ $RACINE2)." >&2; exit 1; }
[ -f "${REG}/${CADRE}.md" ] || { echo "ÉCHEC : le dossier n'est pas revenu." >&2; exit 1; }
echo "  effacé, puis re-dérivé : manifest-root identique."

echo
echo "═══ M5 OK ═══"
echo "  · un SEUL run a produit son dossier, sans geste manuel ;"
echo "  · l'artefact ne prononce pas la conformité, et le dit lui-même ;"
echo "  · un dossier manqué se re-dérive à l'identique — ce qui autorise à ne jamais couper."
