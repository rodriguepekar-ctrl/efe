#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
#  M0 — le critère d'acceptation du profil servi.
#
#  « un clone frais + le profil documenté sert un reçu non vide »
#
#  Ce que ce script prouve, dans cet ordre :
#    1. chaque profil produit EXACTEMENT le rapport qu'il annonce — passerelle ET console
#       (`.expected.json`) — sinon l'opérateur croit savoir ce qu'il déploie ;
#    2. aucun profil ne pose de secret — un secret embarqué ferait dire
#       « SERVIE » avec une valeur publiquement connue ;
#    3. il ne manque au profil `souverain` QUE le secret : injecté, la
#       provenance passe de DÉGRADÉE à SERVIE.
#
#  Aucune écoute réseau, aucun appel externe. Sortie 0 si tout tient.
# ─────────────────────────────────────────────────────────────────────
set -euo pipefail
cd "$(dirname "$0")/.."

BIN="cargo run -q -p efe-swarm-gateway --bin efe-swarm-gateway --"

echo "═══ 1/3 · chaque profil produit le rapport qu'il annonce ═══"
# La comparaison de fond est faite par le test d'intégration, qui n'a pas besoin
# de muter l'environnement du process (et ne peut donc pas courir contre un autre).
cargo test -q -p efe-swarm-gateway --test profile_files 2>/dev/null | grep -E '^test result' \
  || { echo "ÉCHEC : un profil de passerelle a dérivé de son attendu." >&2; exit 1; }
# La console est un binaire SÉPARÉ, avec ses propres interrupteurs et ses propres secrets à
# défaut public. Son profil se vérifie exactement pareil, sinon M0 ne couvrirait qu'une moitié
# du produit — et l'autre moitié est celle qui dérive une clé d'un secret versionné.
cargo test -q -p audit-console --test profile_files 2>/dev/null | grep -E '^test result' \
  || { echo "ÉCHEC : le profil de la console a dérivé de son attendu." >&2; exit 1; }
echo "  les quatre profils concordent avec leur .expected.json (3 passerelle + 1 console)"

echo
echo "═══ 2/3 · aucun profil ne pose de secret ═══"
FAUTE=0
for f in profiles/*.env; do
  for k in EFE_NANO_AUDIT_HMAC_MASTER EFE_SWARM_BUNDLE_SIGNING_SEED DATABASE_URL \
           ANTHROPIC_API_KEY OPENAI_API_KEY GOOGLE_AI_KEY EFE_OIDC_PUBLIC_KEY_PEM \
           EFE_CONSOLE_ROOT EFE_CONSOLE_TOKEN; do
    if grep -qE "^[[:space:]]*${k}=" "$f"; then
      echo "  ÉCHEC : $f pose $k" >&2; FAUTE=1
    fi
  done
done
[ "$FAUTE" -eq 0 ] || exit 1
echo "  aucun secret dans les profils livrés — ils s'injectent au déploiement"

echo
echo "═══ 3/3 · il ne manque au profil souverain QUE le secret ═══"
statut() {  # $1 = capacité ; lit le JSON sur stdin
  python3 -c "
import json,sys
d=json.load(sys.stdin)
print(next((c['status'] for c in d['capabilities'] if c['capability']==sys.argv[1]), 'ABSENTE'))
" "$1"
}

set -a; . ./profiles/souverain.env; set +a
unset EFE_NANO_AUDIT_HMAC_MASTER || true
AVANT=$($BIN --profile-report 2>/dev/null | statut provenance)
echo "  sans secret        : provenance = $AVANT"
[ "$AVANT" = "DÉGRADÉE" ] || { echo "ÉCHEC : sans secret, le rapport doit dire DÉGRADÉE — jamais un faux vert." >&2; exit 1; }

export EFE_NANO_AUDIT_HMAC_MASTER=$(python3 -c "import secrets;print(secrets.token_hex(32))")
APRES=$($BIN --profile-report 2>/dev/null | statut provenance)
echo "  secret injecté     : provenance = $APRES"
[ "$APRES" = "SERVIE" ] || { echo "ÉCHEC : le profil souverain manque autre chose que le secret." >&2; exit 1; }

echo
echo "═══ 4/4 · et il SERT réellement un reçu non vide ═══"
# La moitié qui manquait, et c'était la plus importante : les étapes 1 à 3 ne prouvent
# que le DÉCLARÉ. Un module dont le but est de ne pas confondre le déclaré et le servi
# doit prouver les deux. On démarre pour de vrai, on POSTe un run, on lit la racine.
PORT=18099
export EFE_SWARM_GATEWAY_ADDR="127.0.0.1:${PORT}"
$BIN >/tmp/m0-gw.log 2>&1 &
GW=$!
trap 'kill "$GW" 2>/dev/null || true' EXIT

for _ in $(seq 1 60); do
  curl -sf "http://127.0.0.1:${PORT}/healthz" >/dev/null 2>&1 && break
  sleep 0.5
done
curl -sf "http://127.0.0.1:${PORT}/healthz" >/dev/null 2>&1 \
  || { echo "ÉCHEC : la passerelle n'a pas démarré." >&2; tail -20 /tmp/m0-gw.log >&2; exit 1; }

RUN=$(curl -sf -X POST "http://127.0.0.1:${PORT}/v1/swarm/run" \
        -H 'content-type: application/json' --data @examples/run.json) \
  || { echo "ÉCHEC : le run a été refusé." >&2; exit 1; }
ID=$(printf '%s' "$RUN" | python3 -c 'import json,sys; print(json.load(sys.stdin)["run_id"])')
echo "  run servi          : $ID"

V=$(curl -sf "http://127.0.0.1:${PORT}/v1/swarm/verify/${ID}")
read -r ROOT VERIFIED <<EOF2
$(printf '%s' "$V" | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d.get("root") or "", d.get("verified"))')
EOF2
echo "  racine             : ${ROOT:-（vide）}"
echo "  verified           : $VERIFIED"

[ "${#ROOT}" -eq 64 ] || { echo "ÉCHEC : racine de ${#ROOT} caractères, 64 attendus — le reçu est VIDE." >&2; exit 1; }
[ "$VERIFIED" = "True" ] || { echo "ÉCHEC : le reçu servi ne se re-prouve pas." >&2; exit 1; }
kill "$GW" 2>/dev/null || true

echo
echo "═══ M0 OK ═══"
echo "  · les quatre profils disent la vérité sur ce qu'ils servent ;"
echo "  · aucun n'embarque de secret ;"
echo "  · le seul manque de « souverain » est le secret, injecté au déploiement ;"
echo "  · et un clone frais, ce profil appliqué, SERT un reçu de 64 caractères qui se re-prouve."
