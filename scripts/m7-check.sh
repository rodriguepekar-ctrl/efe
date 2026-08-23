#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
#  M7 — le service de lecture, éprouvé sur un vrai processus.
#
#  Le critère de la feuille de route (« audit-api répond en HTTP sur un
#  port ») était DÉJÀ satisfait : audit-console sert les six routes de
#  route() sur /api/tenants/{t}/…. Sa preuve — `ls crates/audit-api/src/bin`
#  — mesurait l'emplacement d'un binaire, pas la propriété.
#
#  Le critère réécrit, et c'est lui que ce script exécute :
#
#      UN SEUL PROCESSUS SERT PLUSIEURS TENANTS, ET AUCUN NE VOIT L'AUTRE.
#
#  Il vérifie aussi les trois choses qui font que ce n'est pas du théâtre :
#    · un jeton partagé entre deux tenants ARRÊTE le démarrage (le port
#      ne s'ouvre pas) — sans quoi HashMap::insert garderait la dernière
#      session en silence et l'isolation tomberait sur une faute de frappe ;
#    · aucun jeton n'apparaît dans le journal de démarrage ;
#    · les sondes répondent sans jeton et ne révèlent rien.
#
#  Usage :  bash scripts/m7-check.sh
#  Sortie 0 si le service tient ; ≥ 1 sinon.
# ─────────────────────────────────────────────────────────────────────
set -euo pipefail
cd "$(dirname "$0")/.."

PORT=18288
TRAVAIL="$(mktemp -d)"
PID=""

ok()    { printf '  \033[32m✓\033[0m %s\n' "$*"; }
ko()    { printf '  \033[31m✗\033[0m %s\n' "$*"; exit 1; }
titre() { printf '\n\033[1m%s\033[0m\n' "$*"; }
menage() { [ -n "$PID" ] && { kill "$PID" 2>/dev/null; wait "$PID" 2>/dev/null; } || true; rm -rf "$TRAVAIL"; }
trap menage EXIT

titre "0. Les tests du module"
cargo test -q -p audit-console --test m7_service_de_lecture >/dev/null 2>&1 \
  || ko "les tests de M7 ne passent pas"
ok "critère, parité sur les six routes, et gardes"
cargo test -q -p audit-console --test profile_files >/dev/null 2>&1 \
  || ko "les profils ne servent pas ce qu'ils annoncent"
ok "les deux profils (console = démonstration, service = N tenants) tiennent leur promesse"

titre "1. Une déclaration à DEUX tenants, sur un vrai processus"
cat > "$TRAVAIL/deploiement.json" <<'JSON'
{
  "note": "vérification M7",
  "tenants": [{"id": "acme"}, {"id": "globex"}],
  "sessions": [
    {"token": "jeton-acme-verif",   "tenant": "acme",   "role": "compliance_officer", "subject": "officer:v"},
    {"token": "jeton-globex-verif", "tenant": "globex", "role": "auditor",            "subject": "auditor:v"}
  ]
}
JSON
cargo build -q -p audit-console 2>/dev/null || ko "le binaire ne compile pas"
BIN="target/debug/audit-console"
[ -x "$BIN" ] || ko "binaire introuvable : $BIN"

EFE_CONSOLE_ADDR="127.0.0.1:$PORT" \
EFE_CONSOLE_ROOT="$(printf '0%.0s' {1..64})" \
EFE_CONSOLE_DEPLOIEMENT="$TRAVAIL/deploiement.json" \
  "$BIN" > "$TRAVAIL/sortie.log" 2> "$TRAVAIL/demarrage.log" &
PID=$!

for _ in $(seq 1 100); do
  curl -sf "http://127.0.0.1:$PORT/healthz" >/dev/null 2>&1 && break
  sleep 0.1
done
curl -sf "http://127.0.0.1:$PORT/healthz" >/dev/null 2>&1 || { cat "$TRAVAIL/demarrage.log"; ko "le service n'a pas démarré"; }
ok "démarré : deux tenants montés dans UN processus"

titre "2. Les sondes : sans jeton, et sans rien révéler"
SANTE=$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORT/healthz")
PRET=$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORT/readyz")
[ "$SANTE" = "200" ] && [ "$PRET" = "200" ] || ko "sondes : healthz=$SANTE readyz=$PRET"
CORPS=$(curl -s "http://127.0.0.1:$PORT/readyz")
printf '%s' "$CORPS" | grep -qE 'acme|globex' && ko "la sonde nomme un tenant : $CORPS"
ok "healthz 200 · readyz 200 · aucun nom de tenant : $CORPS"

titre "3. LE CRITÈRE : chacun chez soi, personne chez l'autre"
CHEZ_SOI=$(curl -s -o /dev/null -w '%{http_code}' \
  -H "Authorization: Bearer jeton-acme-verif" "http://127.0.0.1:$PORT/api/tenants/acme/cases")
[ "$CHEZ_SOI" = "200" ] || ko "acme ne lit pas chez lui (HTTP $CHEZ_SOI)"
ok "acme lit son registre (200)"

CROISE=$(curl -s -o /dev/null -w '%{http_code}' \
  -H "Authorization: Bearer jeton-acme-verif" "http://127.0.0.1:$PORT/api/tenants/globex/cases")
[ "$CROISE" = "403" ] || ko "acme atteint globex (HTTP $CROISE) — l'isolation est rompue"
ok "acme ne franchit PAS la frontière de globex (403)"

FANTOME=$(curl -s -H "Authorization: Bearer jeton-acme-verif" "http://127.0.0.1:$PORT/api/tenants/nexiste-pas/cases")
REEL=$(curl -s -H "Authorization: Bearer jeton-acme-verif" "http://127.0.0.1:$PORT/api/tenants/globex/cases")
[ "$FANTOME" = "$REEL" ] || ko "un tenant étranger se distingue d'un tenant inexistant — la route est un oracle d'existence"
ok "« étranger » et « inexistant » sont INDISTINGUABLES"

NU=$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORT/api/tenants/acme/cases")
[ "$NU" = "401" ] || ko "une requête sans jeton passe (HTTP $NU)"
ok "sans jeton : 401"

titre "4. Aucun jeton dans le journal de démarrage"
for J in jeton-acme-verif jeton-globex-verif; do
  grep -q "$J" "$TRAVAIL/demarrage.log" && ko "le jeton « $J » est imprimé au démarrage"
done
grep -q "non imprimé" "$TRAVAIL/demarrage.log" || ko "le journal ne dit pas qu'il tait les jetons"
ok "les jetons posés ne sont pas imprimés — et le journal le DIT"
grep -q "2 tenant(s) monté(s)" "$TRAVAIL/demarrage.log" || ko "le résumé ne compte pas les tenants"
ok "le résumé compte : $(grep -m1 'tenant(s) monté' "$TRAVAIL/demarrage.log" | sed 's/^ *//')"

kill "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true   # sans quoi bash annonce « Terminated », ce qui a l'air d'une panne
PID=""

titre "5. Un jeton PARTAGÉ arrête le démarrage — le port ne s'ouvre pas"
cat > "$TRAVAIL/casse.json" <<'JSON'
{
  "tenants": [{"id": "acme"}, {"id": "globex"}],
  "sessions": [
    {"token": "meme-jeton", "tenant": "acme",   "role": "auditor", "subject": "a"},
    {"token": "meme-jeton", "tenant": "globex", "role": "auditor", "subject": "b"}
  ]
}
JSON
set +e
EFE_CONSOLE_ADDR="127.0.0.1:$PORT" \
EFE_CONSOLE_ROOT="$(printf '0%.0s' {1..64})" \
EFE_CONSOLE_DEPLOIEMENT="$TRAVAIL/casse.json" \
  "$BIN" > /dev/null 2> "$TRAVAIL/refus.log"
CODE=$?
set -e
[ "$CODE" = "2" ] || ko "un jeton partagé n'a pas arrêté le démarrage (code $CODE)"
grep -q "JETON PARTAGÉ ENTRE DEUX TENANTS" "$TRAVAIL/refus.log" || ko "le refus ne nomme pas le danger"
grep -q "le port n'a pas été ouvert" "$TRAVAIL/refus.log" || ko "le refus ne dit pas que rien n'a été ouvert"
grep -q "meme-jeton" "$TRAVAIL/refus.log" && ko "le refus imprime le jeton — il devient la fuite qu'il empêche"
curl -sf --max-time 2 "http://127.0.0.1:$PORT/healthz" >/dev/null 2>&1 && ko "le port est ouvert malgré le refus"
ok "code 2 · le danger est nommé · le jeton n'est pas imprimé · le port est fermé"

titre "Ce que M7 ne fait PAS, et qu'il faut redire"
cat <<'FIN'
  · Le bind reste LOCAL (assert_local). Servir un tenant depuis une autre
    machine demande TLS et une authentification réelle : c'est M8.
  · Les sessions sont des jetons porteurs résolus en mémoire. Un déploiement
    réel branche son IdP derrière le même contrat resolve(token) → AuthContext.
  · Un seul verrou protège l'état : toutes les lectures se sérialisent. C'est
    correct, et ce n'est pas un service à fort débit.
  · La déclaration ne porte AUCUN secret racine : les N clés tenant sont
    dérivées de EFE_CONSOLE_ROOT, qui reste pleinement en vigueur — et le
    rapport de démarrage crie encore tant qu'il vaut son défaut public.
FIN

printf '\n\033[32mM7 tient.\033[0m\n'
