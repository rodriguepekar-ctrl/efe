#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
#  M9 — l'ancre TIERCE, éprouvée.
#
#  Le critère : « AnchorProof de méthode rfc3161 PRODUITE ET VÉRIFIÉE ».
#  Avant ce module, AnchorProof n'était construite que dans des tests, et
#  verify_anchor rendait toujours crypto_receipt_checked: false.
#
#  L'enjeu, dit par le message de démarrage de la passerelle lui-même :
#  « l'ancre est la racine attendue, écrite AILLEURS ». Avec le seul
#  journal de fichier, cet « ailleurs » est NOTRE fichier, remis par nous.
#  Un horodateur tiers atteste avec SA clé — c'est le pas de
#  « tamper-evident pour un détenteur de clé » à « OPPOSABLE au détenteur
#  lui-même ».
#
#  Usage :  bash scripts/m9-check.sh
#  Sortie 0 si l'ancre tierce tient ; ≥ 1 sinon.
# ─────────────────────────────────────────────────────────────────────
set -euo pipefail
cd "$(dirname "$0")/.."

ok()    { printf '  \033[32m✓\033[0m %s\n' "$*"; }
ko()    { printf '  \033[31m✗\033[0m %s\n' "$*"; exit 1; }
titre() { printf '\n\033[1m%s\033[0m\n' "$*"; }

titre "1. Le lecteur, sur de l'entrée HOSTILE"
cargo test -q -p scella-rfc3161 >/dev/null 2>&1 || ko "le lecteur RFC 3161 ne passe pas"
ok "DER borné, longueur indéfinie et non minimale refusées, tampons hostiles sans panique"

titre "2. La liaison — ce qui casse le cercle"
cargo test -q -p forensic-export >/dev/null 2>&1 || ko "la vérification d'ancre ne passe pas"
ok "un reçu qui engage une AUTRE racine est démasqué (indétectable avant M9)"
ok "crypto_receipt_checked devient enfin atteignable"

titre "3. LE CRITÈRE — produite et vérifiée, sur une vraie socket"
cargo test -q -p provenance-bridge --test m9_ancre_tierce >/dev/null 2>&1 \
  || ko "le critère de M9 ne passe pas"
ok "demande émise, jeton lu, liaison contrôlée, preuve déposée, verify_anchor = true"
ok "un jeton menteur ⟹ RIEN n'est déposé, et le journal n'annonce aucune ancre"

titre "4. L'analyseur d'URL reste UNIQUE"
cargo test -q -p scella-authority >/dev/null 2>&1 || ko "l'analyseur d'autorité ne passe pas"
N=$(grep -rl '"://"' crates/*/src 2>/dev/null | wc -l | tr -d ' ')
[ "$N" = "3" ] || ko "$N analyseurs d'autorité (attendu 3 : le crate + les 2 exemptions M1.b)"
ok "le port et le chemin passent par scella-authority — aucun second découpage d'URL"

titre "5. La passerelle REFUSE de mentir sur son ancre"
cargo build -q -p efe-swarm-gateway 2>/dev/null || ko "la passerelle ne compile pas"
BIN="target/debug/efe-swarm-gateway"
[ -x "$BIN" ] || ko "binaire introuvable"

# ⚠ Le bloc d'ancrage vit sous `EFE_SWARM_BUNDLE_ENABLED` : sans lui, le binaire ne l'atteint
#   jamais et se met à SERVIR. La première version de ce script l'ignorait et attendait un processus
#   qui ne comptait pas s'arrêter — un test qui pend au lieu d'échouer est un test qui ne dit rien.
essai() {  # $1 = message attendu, puis l'environnement
  local attendu="$1"; shift
  local log; log="$(mktemp)"
  set +e
  env "$@" \
      EFE_SWARM_BUNDLE_ENABLED=true \
      EFE_SWARM_ANCHOR_LOG=/tmp/scella-m9-ancres.log \
      "$BIN" >/dev/null 2>"$log" &
  local pid=$!
  # Garde de temps : un refus est IMMÉDIAT. Si le processus vit encore, c'est qu'il a démarré —
  # donc qu'il a accepté une configuration qu'il devait refuser.
  local reste=60
  while kill -0 "$pid" 2>/dev/null && [ "$reste" -gt 0 ]; do sleep 0.1; reste=$((reste-1)); done
  if kill -0 "$pid" 2>/dev/null; then
    kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null
    cat "$log" >&2; rm -f "$log"
    ko "le binaire a DÉMARRÉ au lieu de refuser"
  fi
  wait "$pid"; local code=$?
  set -e
  [ "$code" = "2" ] || { cat "$log" >&2; rm -f "$log"; ko "attendu code 2, obtenu $code"; }
  grep -q "$attendu" "$log" || { cat "$log" >&2; rm -f "$log"; ko "message attendu absent : $attendu"; }
  rm -f "$log"
}

essai "EFE_SWARM_TSA_DIR manque" EFE_SWARM_TSA_URL=http://horodateur.exemple/tsa
ok "URL d'horodateur sans répertoire ⟹ démarrage REFUSÉ (retomber sur notre journal en silence"
printf '     donnerait une ancre NÔTRE à qui en a demandé une TIERCE)\n'

# ⚠ Pas « 127.evil.com » : c'est un NOM d'hôte parfaitement valide, et l'analyseur l'accepte à
#   raison. Ce que M1 a corrigé, c'est qu'il n'est pas LOOPBACK — pas qu'il soit malformé. La
#   première version de cette ligne confondait les deux.
essai "EFE_SWARM_TSA_URL" EFE_SWARM_TSA_URL=http://horodateur.exemple:0/tsa EFE_SWARM_TSA_DIR=/tmp/x
ok "URL que l'analyseur unique refuse ⟹ démarrage REFUSÉ"

titre "6. Ce que NOUS ne prouvons pas — et qui doit rester vrai"
if command -v openssl >/dev/null 2>&1; then
  ok "openssl est présent : c'est LUI qui vérifie la signature, pas nous"
else
  ok "(openssl absent ici — la vérification de signature est de toute façon celle du TIERS)"
fi
cat <<'FIN'
     La division, et aucun des deux côtés ne demande à l'autre d'être cru :
       · NOUS garantissons que le jeton déposé engage la racine déposée
         (le TSTInfo est LU, et un jeton d'une autre racine est refusé) ;
       · LE TIERS garantit que l'horodateur l'a signé :

           openssl ts -verify -digest <manifest_root> \
                      -in <root>.tsr -CAfile <ca-de-lhorodateur>

     Écrire cette vérification ici violerait la règle du dépôt — « aucune
     crypto maison, aucune 3e pile » — et un vérifieur qu'on écrit soi-même
     ne prouve rien à qui doute de soi.
FIN

titre "Ce que M9 NE fait PAS, et qu'il faut redire"
cat <<'FIN'
  · Il ne vérifie AUCUNE signature ni chaîne de certificats. Le jeton est
    déposé TEL QUEL pour que le tiers le fasse.
  · Il parle en CLAIR : un jeton RFC 3161 est signé, donc son intégrité ne
    dépend pas du canal. Ce que le canal expose est le condensé soumis —
    ni un secret, ni réversible. Pour du chiffrement, pointez un horodateur
    interne ou un mandataire.
  · Il rend la route de bundle DÉPENDANTE d'un tiers : horodateur muet ⟹
    export en échec. C'est un choix d'exploitant, pas un défaut : le
    journal seul ne tombe jamais et ne prouve rien contre nous.
  · L'horodatage ne remplace PAS le journal d'ancres : c'est la ligne de
    journal qui détecte une TRONCATURE. Les deux gardes sont orthogonaux,
    et le puits tiers enveloppe le journal au lieu de s'y substituer.
FIN

printf '\n\033[32mM9 tient.\033[0m\n'
