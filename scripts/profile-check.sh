#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
#  M0 — le critère d'acceptation du profil servi.
#
#  « un clone frais + le profil documenté sert un reçu non vide »
#
#  Ce que ce script prouve, dans cet ordre :
#    1. chaque profil produit EXACTEMENT le rapport qu'il annonce
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
  || { echo "ÉCHEC : un profil a dérivé de son attendu." >&2; exit 1; }
echo "  les trois profils concordent avec leur .expected.json"

echo
echo "═══ 2/3 · aucun profil ne pose de secret ═══"
FAUTE=0
for f in profiles/*.env; do
  for k in EFE_NANO_AUDIT_HMAC_MASTER EFE_SWARM_BUNDLE_SIGNING_SEED DATABASE_URL \
           ANTHROPIC_API_KEY OPENAI_API_KEY GOOGLE_AI_KEY EFE_OIDC_PUBLIC_KEY_PEM; do
    if grep -qE "^[[:space:]]*${k}=" "$f"; then
      echo "  ÉCHEC : $f pose $k" >&2; FAUTE=1
    fi
  done
done
[ "$FAUTE" -eq 0 ] || exit 1
echo "  aucun secret dans les trois profils — ils s'injectent au déploiement"

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
echo "═══ M0 OK ═══"
echo "  · les trois profils disent la vérité sur ce qu'ils servent ;"
echo "  · aucun n'embarque de secret ;"
echo "  · et le seul manque de « souverain » est le secret, injecté au déploiement."
