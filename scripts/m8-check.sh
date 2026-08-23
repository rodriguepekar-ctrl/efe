#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
#  M8 — la console opposable, éprouvée sur un vrai processus.
#
#  « auth + TLS + un tenant ne voit jamais l'autre ». M7 a rendu la
#  troisième clause testable. M8 traite les deux premières — et il faut
#  dire ce qu'elles voulaient dire ici.
#
#  Un acte de supervision est scellé dans une chaîne NON RÉPUDIABLE. Il
#  n'est opposable que si l'on peut tenir les conditions dans lesquelles
#  il a été obtenu. Or, avant M8 :
#
#    · un `<form method=post>` HTML n'envoie aucun en-tête Authorization,
#      et le serveur ne posait JAMAIS le cookie qu'il acceptait : le
#      chemin d'écriture était cassé tel que livré, et la seule façon de
#      le faire marcher — poser le cookie à la main — était exactement la
#      configuration exposée au CSRF ;
#    · aucune défense CSRF n'existait (ni Origin, ni Sec-Fetch, ni jeton) ;
#    · le jeton était gardé EN CLAIR en mémoire, sans échéance ;
#    · `assert_local` interdisait tout service distant, donc « TLS » ne
#      pouvait même pas se poser.
#
#  Ce script vérifie ce qu'un processus réel peut montrer.
#
#  ⚠ CE PROCESSUS NE TERMINE PAS TLS, et M8 ne le prétend pas. Ce qu'il
#    refuse, c'est de servir à distance EN SILENCE.
#
#  Usage :  bash scripts/m8-check.sh
#  Sortie 0 si la console tient ; ≥ 1 sinon.
# ─────────────────────────────────────────────────────────────────────
set -euo pipefail
cd "$(dirname "$0")/.."

PORT=18289
TRAVAIL="$(mktemp -d)"
PID=""
RACINE="$(printf '0%.0s' {1..64})"

ok()    { printf '  \033[32m✓\033[0m %s\n' "$*"; }
ko()    { printf '  \033[31m✗\033[0m %s\n' "$*"; exit 1; }
titre() { printf '\n\033[1m%s\033[0m\n' "$*"; }
menage() { [ -n "$PID" ] && { kill "$PID" 2>/dev/null; wait "$PID" 2>/dev/null; } || true; rm -rf "$TRAVAIL"; }
trap menage EXIT

titre "0. Les tests du module"
cargo test -q -p audit-console --test m8_console_opposable >/dev/null 2>&1 \
  || ko "le critère de M8 ne passe pas"
ok "plancher de scellement, CSRF, session, isolation sous chaque barreau"
cargo test -q -p audit-console --test profile_files >/dev/null 2>&1 \
  || ko "les trois profils ne servent pas ce qu'ils annoncent"
ok "les trois profils (console · service · expose) tiennent leur promesse"

titre "1. Servir à distance SANS le déclarer est refusé"
cargo build -q -p audit-console 2>/dev/null || ko "le binaire ne compile pas"
BIN="target/debug/audit-console"
set +e
EFE_CONSOLE_ADDR="0.0.0.0:$PORT" EFE_CONSOLE_ROOT="$RACINE" \
  "$BIN" > /dev/null 2> "$TRAVAIL/refus-bind.log"
CODE=$?
set -e
[ "$CODE" != "0" ] || ko "un bind non-local a été accepté sans déclaration"
grep -q "souveraineté" "$TRAVAIL/refus-bind.log" || ko "le refus n'invoque pas la souveraineté"
grep -q "DÉCLAREZ le terminateur" "$TRAVAIL/refus-bind.log" || ko "le refus ne dit pas le geste qui le lève"
grep -q "en clair" "$TRAVAIL/refus-bind.log" || ko "le refus ne dit pas le danger que le silence ferait courir"
ok "bind 0.0.0.0 refusé · le geste ET le danger sont nommés"

titre "2. Le même bind, DÉCLARÉ, est accepté"
cat > "$TRAVAIL/deploiement.json" <<'JSON'
{
  "note": "vérification M8",
  "tenants": [{"id": "acme"}, {"id": "globex"}],
  "sessions": [
    {"token": "jeton-acme-m8",   "tenant": "acme",   "role": "compliance_officer", "subject": "officer:v"},
    {"token": "jeton-globex-m8", "tenant": "globex", "role": "auditor",            "subject": "auditor:v"}
  ]
}
JSON
EFE_CONSOLE_ADDR="0.0.0.0:$PORT" \
EFE_CONSOLE_ROOT="$RACINE" \
EFE_CONSOLE_TERMINATEUR_CANAL="ingress-de-verification" \
EFE_CONSOLE_DEPLOIEMENT="$TRAVAIL/deploiement.json" \
  "$BIN" > "$TRAVAIL/sortie.log" 2> "$TRAVAIL/demarrage.log" &
PID=$!
for _ in $(seq 1 100); do
  curl -sf "http://127.0.0.1:$PORT/healthz" >/dev/null 2>&1 && break
  sleep 0.1
done
curl -sf "http://127.0.0.1:$PORT/healthz" >/dev/null 2>&1 || { cat "$TRAVAIL/demarrage.log"; ko "le service n'a pas démarré"; }
ok "démarré sur 0.0.0.0 — la déclaration a levé la garde"
grep -q "terminateur DÉCLARÉ" "$TRAVAIL/demarrage.log" || ko "le démarrage ne répète pas la déclaration"
grep -q "ne termine PAS TLS" "$TRAVAIL/demarrage.log" || ko "le démarrage ne dit pas qu'il ne chiffre pas lui-même"
ok "et le démarrage RÉPÈTE la déclaration, en disant qu'il ne chiffre pas lui-même"
for J in jeton-acme-m8 jeton-globex-m8; do
  grep -q "$J" "$TRAVAIL/demarrage.log" && ko "le jeton « $J » est imprimé au démarrage"
done
ok "aucun jeton dans le journal"
grep -q "SANS ÉCHÉANCE" "$TRAVAIL/demarrage.log" || ko "les sessions sans échéance ne sont pas annoncées"
ok "les sessions sans échéance sont COMPTÉES, pas acceptées en silence"

titre "3. La connexion pose un cookie scellé"
CK=$(curl -s -i -X POST "http://127.0.0.1:$PORT/login" \
  -H 'Sec-Fetch-Site: same-origin' --data 'token=jeton-acme-m8' | tr -d '\r' | grep -i '^set-cookie:' || true)
[ -n "$CK" ] || ko "aucun cookie posé à la connexion"
printf '%s' "$CK" | grep -q 'HttpOnly' || ko "cookie sans HttpOnly : un script pourrait le lire"
printf '%s' "$CK" | grep -q 'SameSite=Strict' || ko "cookie sans SameSite=Strict : un site tiers le ferait partir"
printf '%s' "$CK" | grep -q 'Secure' && ko "« Secure » sur http://127.0.0.1 empêcherait le navigateur d'enregistrer le cookie"
ok "HttpOnly · SameSite=Strict · pas de Secure en boucle locale (sinon le cookie ne s'enregistre pas)"

titre "4. CSRF : un cookie part tout seul, un en-tête non"
ACTE='target_step=aggregate&action=APPROVE&justification=forge'
INTER=$(curl -s -o /dev/null -w '%{http_code}' -X POST "http://127.0.0.1:$PORT/cases/X/action" \
  -H "Cookie: session=jeton-acme-m8" -H 'Sec-Fetch-Site: cross-site' --data "$ACTE")
[ "$INTER" = "403" ] || ko "une requête inter-site par cookie a été acceptée (HTTP $INTER)"
ok "POST inter-site par cookie : 403"

NUE=$(curl -s -o /dev/null -w '%{http_code}' -X POST "http://127.0.0.1:$PORT/cases/X/action" \
  -H "Cookie: session=jeton-acme-m8" --data "$ACTE")
[ "$NUE" = "403" ] || ko "un POST par cookie sans preuve d'origine a été accepté (HTTP $NUE)"
ok "POST par cookie sans preuve d'origine : 403"

PORTEUR=$(curl -s -o /dev/null -w '%{http_code}' -X POST "http://127.0.0.1:$PORT/api/tenants/acme/cases/ABSENT/human-action" \
  -H "Authorization: Bearer jeton-acme-m8" -H 'Content-Type: application/json' \
  --data '{"target_step":"aggregate","action":"APPROVE","justification":"x"}')
[ "$PORTEUR" != "403" ] || ko "l'en-tête porteur devrait être dispensé de la preuve d'origine (HTTP $PORTEUR)"
ok "en-tête porteur : dispensé (un site tiers ne peut pas le faire poser) — HTTP $PORTEUR"

titre "5. L'isolation de M7 tient toujours"
CROISE=$(curl -s -o /dev/null -w '%{http_code}' \
  -H "Authorization: Bearer jeton-acme-m8" "http://127.0.0.1:$PORT/api/tenants/globex/cases")
[ "$CROISE" = "403" ] || ko "acme atteint globex (HTTP $CROISE)"
ok "acme ne franchit pas la frontière de globex (403)"

titre "Ce que M8 NE fait PAS, et qu'il faut redire"
cat <<'FIN'
  · Ce processus ne termine PAS TLS. Le dépôt a pourtant une pile complète
    (TLS 1.3, mTLS, rotation) dans efe-meta-swarm-server ; la tirer dans un
    crate feuille triplerait son arbre, et un terminateur mûr fait mieux le
    travail. Ce que M8 refuse, c'est de servir à distance EN SILENCE.
  · La déclaration vaut pour le PROCESSUS, pas pour la connexion : un client
    qui contourne le terminateur et se connecte directement est crédité
    « protégé ». Le seul témoin serait un en-tête, et un en-tête est
    falsifiable par le client — donc inutilisable ici. La parade est un geste
    de déploiement : ne pas exposer le service à côté de son terminateur.
  · Le barreau du canal n'est PAS scellé dans le nœud Human : le faire
    changerait le format de Node::Human, ce qui est un événement de version,
    pas un module. M8 COUPE au lieu d'enregistrer — un acte qui ne franchit
    pas le plancher n'est jamais scellé.
  · Un jeton porteur reste un secret PARTAGÉ : le porteur le présente, donc
    le serveur pourrait le rejouer. Une vraie non-répudiation demanderait que
    l'officier SIGNE le tuple de l'acte avec une clé que le serveur ne détient
    pas — l'analogue exact du palier R3 de RC-1, nommé et non produit.
FIN

printf '\n\033[32mM8 tient.\033[0m\n'
