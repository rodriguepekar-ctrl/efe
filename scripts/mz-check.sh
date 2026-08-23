#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
#  MZ — la mesure du Cadrage.
#
#  Le critère : « le plafond du compresseur et le taux de préservation
#  sont CHIFFRÉS ». MZ ne sert rien — il mesure, et son produit attendu
#  est peut-être un refus : la feuille de route dit qu'il « peut tuer le
#  Cadrage sans rien coûter ».
#
#  ⚠ CE SCRIPT NE JUGE PAS LE CADRAGE.
#
#    Il vérifie que la MESURE a eu lieu, pas que le Cadrage a PASSÉ. Si
#    un verdict « TUÉ » faisait sortir en erreur, alors MZ réussissant
#    ferait rougir une CI — et la pression serait d'assouplir les seuils
#    jusqu'au retour du vert. C'est le mécanisme exact qui rend un garde
#    décoratif. Le verdict s'écrit en toutes lettres ; il ne se code pas
#    dans un entier.
#
#  Usage :  bash scripts/mz-check.sh
#  Sortie 0 si la mesure a pu être faite ; ≥ 1 sinon.
# ─────────────────────────────────────────────────────────────────────
set -euo pipefail
cd "$(dirname "$0")/.."

PORT="${MZ_PORT:-8081}"
MODELE="models/compressor.gguf"
PID=""

ok()    { printf '  \033[32m✓\033[0m %s\n' "$*"; }
ko()    { printf '  \033[31m✗\033[0m %s\n' "$*"; exit 1; }
note()  { printf '  \033[33m·\033[0m %s\n' "$*"; }
titre() { printf '\n\033[1m%s\033[0m\n' "$*"; }
menage(){ [ -n "$PID" ] && { kill "$PID" 2>/dev/null; wait "$PID" 2>/dev/null; } || true; }
trap menage EXIT

titre "1. L'appareil de mesure, éprouvé SANS modèle"
#
#  Un verdict qu'on ne peut éprouver qu'en lançant l'expérience est un verdict
#  que personne ne relit — et c'est celui-là qui finit par s'assouplir. Les
#  seuils, le vérificateur et le corpus sont donc purs et testés hors ligne.
#
cargo test -q -p scella-mz --lib >/dev/null 2>&1 || ko "l'appareil de mesure ne passe pas ses tests"
ok "corpus (vérité de terrain), vérificateur indépendant, verdict — tous testés hors ligne"

titre "2. Le corpus dit-il la vérité sur lui-même ?"
cargo test -q -p scella-mz --lib corpus >/dev/null 2>&1 \
  || ko "le corpus ne tient pas ses propres invariants"
ok "chaque entité est réellement dans son document, et l'adverse les place bien en fin"

titre "3. Les seuils viennent-ils du dépôt, ou de nous ?"
grep -q '8:1 à 12:1' crates/efe-nano-app-coordinator/src/types/compressor.rs \
  || ko "le contrat du compresseur ne revendique plus 8:1 à 12:1 — le seuil de MZ doit être rediscuté"
ok "le ratio jugé est celui que le compresseur REVENDIQUE, pas un chiffre choisi ici"

titre "4. La mesure elle-même"
[ -f "$MODELE" ] || ko "modèle absent : $MODELE"
ok "modèle présent ($(du -h "$MODELE" | cut -f1))"

if curl -sf "http://127.0.0.1:$PORT/health" >/dev/null 2>&1; then
  note "un llama-server répond déjà sur $PORT — on l'emploie"
else
  command -v llama-server >/dev/null 2>&1 || ko "llama-server absent et rien n'écoute sur $PORT"
  llama-server -m "$MODELE" --host 127.0.0.1 --port "$PORT" -c 4096 --log-disable \
    > /tmp/mz-llama.log 2>&1 &
  PID=$!
  for _ in $(seq 1 90); do
    curl -sf "http://127.0.0.1:$PORT/health" >/dev/null 2>&1 && break
    sleep 1
  done
  curl -sf "http://127.0.0.1:$PORT/health" >/dev/null 2>&1 \
    || { tail -5 /tmp/mz-llama.log >&2; ko "le modèle n'a pas démarré"; }
  ok "llama-server démarré sur $PORT"
fi

EFE_COMPRESSOR_URL="http://127.0.0.1:$PORT" cargo run -q -p scella-mz 2>&1 | tail -24
[ -f crates/scella-mz/MZ_MESURE_CADRAGE.md ] || ko "aucun rapport produit — la mesure n'a pas eu lieu"
ok "rapport écrit : crates/scella-mz/MZ_MESURE_CADRAGE.md"

titre "5. Le verdict est-il CHIFFRÉ, comme le critère l'exige ?"
grep -qE '[0-9]+[.,][0-9]+:1' crates/scella-mz/MZ_MESURE_CADRAGE.md \
  || ko "le rapport ne porte aucun ratio chiffré — le critère de MZ n'est pas tenu"
grep -q 'préservation \*\*vérifiée\*\*' crates/scella-mz/MZ_MESURE_CADRAGE.md \
  || ko "le rapport ne distingue pas la préservation vérifiée de la déclarée"
ok "plafond et taux de préservation chiffrés, et les deux taux distingués"

titre "Ce que MZ ne mesure pas, et qu'il faut redire"
cat <<'FIN'
  · La QUALITÉ de la réponse finale. MZ mesure le compresseur, pas le
    pipeline — un résumé qui préserve tout peut mal servir le modèle
    frontière, et cette expérience-là coûte des API.
  · La CORRECTION. Ce qui est mesuré est ce qui a été préservé, jamais
    ce qui est vrai.
  · Les AUTRES robots. Analyseur, vérifieur et reformulateur ont leurs
    propres plafonds ; les mesurer ensemble mélangerait les causes.

  Et le fait qui encadre tout : les deux sites de production passent
  `must_preserve: vec![]`. Cette mesure DEMANDE la préservation ; le
  service, jamais. Un verdict favorable désignerait donc le câblage.
FIN

printf '\n\033[32mLa mesure a eu lieu.\033[0m  (le verdict est dans le rapport — il ne se code pas dans un code de sortie)\n'
