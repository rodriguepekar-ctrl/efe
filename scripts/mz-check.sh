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
AVANT=""
JSON="crates/scella-mz/MZ_MESURE_CADRAGE.json"

ok()    { printf '  \033[32m✓\033[0m %s\n' "$*"; }
ko()    { printf '  \033[31m✗\033[0m %s\n' "$*"; exit 1; }
note()  { printf '  \033[33m·\033[0m %s\n' "$*"; }
titre() { printf '\n\033[1m%s\033[0m\n' "$*"; }
menage(){ [ -n "$PID" ] && { kill "$PID" 2>/dev/null; wait "$PID" 2>/dev/null; } || true;
          [ -n "$AVANT" ] && rm -f "$AVANT" || true; }
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

titre "5. Les DEUX bras sont-ils mesurés ?"
#
#  La v1 ne mesurait que le bras « exigé » — une configuration que PERSONNE
#  n'exécute : les deux sites de production passent `must_preserve: vec![]`.
#  Et sur ce bras, le filtre du client écarte les réponses où le modèle avoue
#  une omission : seules passaient les réponses déclarant tout avoir gardé. Le bras « servi »
#  n'a aucun
#  filtre : c'est le chiffre à opposer à qui décrit le Cadrage tel qu'il tourne.
#
grep -q 'Bras `exigé`' crates/scella-mz/MZ_MESURE_CADRAGE.md \
  || ko "le rapport ne porte pas le bras exigé"
grep -q 'Bras `servi`' crates/scella-mz/MZ_MESURE_CADRAGE.md \
  || ko "le rapport ne porte pas le bras SERVI — il décrirait une configuration que personne n'exécute"
ok "les deux bras figurent : la promesse ET ce que le pipeline obtient"

grep -q 'biaisé vers les réponses qui déclarent tout préserver' crates/scella-mz/MZ_MESURE_CADRAGE.md \
  || ko "le rapport ne déclare pas le biais du bras exigé"
ok "et le biais du filtre est DÉCLARÉ, pas tu"

titre "6. Le verdict est-il CHIFFRÉ, comme le critère l'exige ?"
grep -qE '[0-9]+[.,][0-9]+:1' crates/scella-mz/MZ_MESURE_CADRAGE.md \
  || ko "le rapport ne porte aucun ratio chiffré — le critère de MZ n'est pas tenu"
grep -q 'préservation \*\*vérifiée\*\*' crates/scella-mz/MZ_MESURE_CADRAGE.md \
  || ko "le rapport ne distingue pas la préservation vérifiée de la déclarée"
ok "plafond et taux de préservation chiffrés, et les deux taux distingués"

titre "7. Le rapport DÉRIVE-T-IL des mesures, ou a-t-il dérivé d'elles ?"
#
#  Un rapport publié se corrige : une formulation, un mot mal choisi. Re-lancer le balayage pour
#  cela changerait les CHIFFRES — l'artefact ne prouverait plus rien du changement de mots. D'où
#  `--rendre`, qui refabrique le `.md` depuis le `.json` sans un appel au modèle.
#
#  Ce qui rend ce mode honnête : il RECALCULE paliers et verdict depuis les points bruts au lieu de
#  recopier ceux qu'il trouve, puis confronte. Recopier aurait fait un re-rendu incapable d'échouer.
#
AVANT="$(mktemp)"
if [ -f "$JSON" ]; then
  #  Sortie non nulle de `--rendre` = le recalcul depuis les points bruts ne redonne PAS ce que le
  #  balayage avait enregistré. C'est la vérification qui porte le fond.
  cp "$JSON" "$AVANT"
  cargo run -q -p scella-mz -- --rendre >/dev/null 2>&1 \
    || ko "\`--rendre\` a divergé : le rapport ne se recalcule plus depuis les mesures"
  ok "le rapport se refabrique depuis les mesures, sans modèle, et le recalcul CONCORDE"

  #  ⭐ Et la mesure doit ressortir INTACTE, à l'octet. C'est ce contrôle qui a trouvé le défaut :
  #  une première version de `--rendre` rafraîchissait les champs dérivés du JSON, donc relisait et
  #  ré-émettait `points` — et les flottants en revenaient changés d'1 ulp (`0.45454545454545453`
  #  → `0.4545454545454546`), la lecture de `serde_json` n'étant pas correctement arrondie. Un
  #  outil qui prétend conserver un enregistrement en le modifiant : le défaut que ce dépôt traque.
  if cmp -s "$AVANT" "$JSON"; then
    ok "et la mesure ressort INTACTE — le re-rendu lit le \`.json\`, il n'y écrit pas"
  else
    ko "le re-rendu a MODIFIÉ la mesure qu'il relit :$(printf '\n'; diff "$AVANT" "$JSON" | head -6 | sed 's/^/      /')"
  fi
else
  note "aucun \`$JSON\` : rien à re-rendre tant qu'aucun balayage n'a eu lieu"
fi

titre "8. L'A/B de température — optionnel, et pourquoi"
#
#  La campagne H1.1 avait mesuré ≈ 33 % de no-op à `temperature 0.2` (sans seed,
#  llama-server tire un aléa par appel) ; le passage au greedy 0.0 était censé
#  la supprimer. Le rejouer double le temps de mesure, donc il n'est pas
#  systématique — mais la commande est écrite ici pour que personne n'ait à la
#  redécouvrir.
#
cat <<'FIN'
  · Le bras de comparaison H1.1 se rejoue ainsi :

      EFE_COMPRESSOR_TEMPERATURE=0.2 cargo run -p scella-mz

    et se compare au rapport courant (dégénérescence attendue ≈ 33 % contre
    0 % au greedy). Non lancé ici : il double le temps de mesure.
FIN
ok "la commande de comparaison est écrite, pas à redécouvrir"

titre "Ce que MZ ne mesure pas, et qu'il faut redire"
cat <<'FIN'
  · La QUALITÉ de la réponse finale. MZ mesure le compresseur, pas le
    pipeline — un résumé qui préserve tout peut mal servir le modèle
    frontière, et cette expérience-là coûte des API.
  · La CORRECTION. Ce qui est mesuré est ce qui a été préservé, jamais
    ce qui est vrai.
  · Les AUTRES robots. Analyseur, vérifieur et reformulateur ont leurs
    propres plafonds ; les mesurer ensemble mélangerait les causes.

  Et une limite de la mesure ELLE-MÊME, qu'il faut dire : sur le bras
  « exigé », le filtre du client écarte les réponses où le modèle avoue
  une omission. Ce bras est donc, par construction, biaisé vers celles
  qui déclarent tout préserver. C'est précisément pourquoi le bras
  « servi » existe — il n'a aucun filtre, et il reproduit la
  configuration que le pipeline exécute réellement.
FIN

printf '\n\033[32mLa mesure a eu lieu.\033[0m  (le verdict est dans le rapport — il ne se code pas dans un code de sortie)\n'
