#!/usr/bin/env bash
# SPDX-License-Identifier: UNLICENSED
# ─────────────────────────────────────────────────────────────────────────────
#  LE BANC DU REGISTRE (C1) — une console de conformité qu'on peut CASSER.
#
#  ## Pourquoi ce script existe
#
#  `audit-console` sert une vraie interface web. Elle est éprouvée par M7 et M8,
#  elle a son profil, ses DIX variables déclarées, ses sondes — et elle était
#  INJOIGNABLE : absente de `compose.yaml`, absente de l'image. Un actif payé que
#  personne ne pouvait ouvrir.
#
#  Et une de ces dix variables n'était alimentée par RIEN : `EFE_CONSOLE_RECORDS`,
#  celle qui réhydrate un tenant depuis un journal scellé. Zéro script, zéro test
#  ne l'écrivait (mesuré : `grep -rl` sur `scripts/` et `crates/*/tests/`). Le
#  chemin de réhydratation existait, compilait, et n'avait jamais servi.
#
#  La rendre lançable sur un registre VIDE n'aurait rien réglé : on aurait
#  remplacé « on ne peut pas l'ouvrir » par « on l'ouvre et il n'y a rien ».
#  D'où le semeur (`console-semence`), et d'où ce banc.
#
#  ## Ce n'est pas une vitrine
#
#  Un registre vert ne prouve pas qu'on saurait reconnaître un registre faux. Ce
#  banc monte donc TROIS postures, et la différence entre les deux dernières est
#  la chose la plus utile qu'il enseigne :
#
#    (défaut)    le cas conforme        chaîne intacte  → badge VERT, verdict JUSTE
#    --menteur   le cas MENTEUR scellé  chaîne intacte  → badge VERT, verdict FAUX
#    --altere    le fichier RETOUCHÉ    chaîne cassée   → badge ROUGE, record nommé
#
#  Autrement dit : la chaîne atteste l'INTÉGRITÉ de ce qui a été enregistré,
#  jamais la CORRECTION de la décision. La console l'écrit elle-même en haut de
#  chaque page — ce banc le fait TOUCHER. « Intègre » n'est jamais « correct ».
#
#  ## Pourquoi sur l'hôte, et pas dans Docker
#
#  La console REFUSE de binder hors de la boucle locale sans qu'on ait déclaré le
#  terminateur de canal placé devant elle (`server.rs`, garde I-S3-SOUVERAIN).
#  Or Docker ne publie un port que si le conteneur écoute sur son interface : le
#  loopback d'un conteneur est inatteignable depuis l'hôte. Un service `console`
#  dans `compose.yaml` exigerait donc de DÉCLARER un terminateur qui n'existe
#  pas — et cette déclaration est portante : elle abaisse le plancher de
#  scellement à A2, c'est-à-dire qu'elle autorise un pair distant à sceller des
#  actes humains. Écrire « un terminateur protège ce service » quand la seule
#  chose devant est `docker-proxy` serait exactement le décor que ce dépôt
#  retire partout ailleurs.
#
#  Sur l'hôte, le pair EST la boucle locale, observée par le noyau : A1, le
#  barreau le plus haut, sans rien déclarer et sans rien mentir.
#
#  ## Les trois états
#     0  le banc s'est prouvé (vert vert, rouge rouge) — et il vous attend
#     1  une épreuve a rendu le CONTRAIRE de ce qui était attendu
#     3  NON MESURÉ — l'instrument n'a pas pu regarder (cargo, curl, un port pris)
#
#  Usage :
#     bash scripts/banc-registre.sh                 # monte le banc, s'éprouve, RESTE ouvert
#     bash scripts/banc-registre.sh --menteur       # ... sur le cas menteur (vert, et faux)
#     bash scripts/banc-registre.sh --altere        # ... sur le fichier retouché (rouge)
#     bash scripts/banc-registre.sh --verifier      # joue les trois épreuves et SORT (0/1/3)
#     bash scripts/banc-registre.sh --sans-semence  # NE RESÈME PAS : reprend les fichiers tels
#                                                   # quels, saute les épreuves, ouvre. C'est le
#                                                   # mode qu'il faut pour que VOS retouches
#                                                   # survivent au relancement.
#
#     BANC_PORT=9099 bash scripts/banc-registre.sh  # si 8088 est pris
# ─────────────────────────────────────────────────────────────────────────────
set -uo pipefail
cd "$(dirname "$0")/.."

ok()    { printf '  \033[32m✓\033[0m %s\n' "$*"; }
ko()    { printf '  \033[31m✗\033[0m %s\n' "$*"; FAUTE=1; }
mort()  { printf '  \033[31m✗\033[0m %s\n' "$*"; exit 1; }
nm()    { printf '  \033[33m·\033[0m NON MESURÉ — %s\n' "$*"; exit 3; }
note()  { printf '      %s\n' "$*"; }
titre() { printf '\n\033[1;34m%s\033[0m\n' "$*"; }

FAUTE=0
POSTURE=conforme
VERIFIER=0
SEMER=1
DOSSIER=""
for a in "$@"; do
  case "$a" in
    --menteur)  POSTURE=menteur ;;
    --altere)   POSTURE=altere ;;
    --verifier) VERIFIER=1 ;;
    --sans-semence) SEMER=0 ;;
    -*) mort "option inconnue « $a » — voir l'en-tête de ce fichier." ;;
    *)  DOSSIER="$a" ;;
  esac
done
DOSSIER="${DOSSIER:-./banc-registre}"
PORT="${BANC_PORT:-8088}"
TENANT="11111111-2222-3333-4444-555555555555"
CAS="SUP-2024-0917"
BASE="http://127.0.0.1:$PORT"

# ── Le ménage. Le banc laisse la console DEBOUT quand il rend la main (c'est le
#    but) ; il ne doit rien laisser derrière lui dans tous les AUTRES cas — un
#    échec qui laisse un processus sur le port fait échouer la tentative suivante
#    en accusant la machine, alors que le coupable est l'essai précédent.
PID=""
menage() { [ -n "$PID" ] && kill "$PID" 2>/dev/null; }
trap menage EXIT

# ═══ 0 · les outils ══════════════════════════════════════════════════════════
titre "0 · l'instrument"
command -v cargo >/dev/null || nm "cargo est absent. La console est un crate Rust SANS dépendance externe ; il faut néanmoins un compilateur. https://rustup.rs"
command -v curl  >/dev/null || nm "curl est absent — le banc s'interroge lui-même par HTTP."
if command -v lsof >/dev/null && lsof -nP -iTCP:"$PORT" -sTCP:LISTEN >/dev/null 2>&1; then
  nm "le port $PORT est déjà pris. Relancez avec : BANC_PORT=9099 bash scripts/banc-registre.sh"
fi
ok "cargo, curl, port $PORT libre"

# ═══ 1 · les deux binaires ═══════════════════════════════════════════════════
titre "1 · la console et son semeur"
note "cargo build -p audit-console (les deux binaires) — long la première fois."
if ! cargo build --quiet -p audit-console 2>&1 | sed 's/^/      /'; then
  mort "la compilation a échoué (sortie ci-dessus)."
fi
CONSOLE=target/debug/audit-console
SEMENCE=target/debug/console-semence
[ -x "$CONSOLE" ] || mort "$CONSOLE est absent après un build réussi — anomalie."
[ -x "$SEMENCE" ] || mort "$SEMENCE est absent après un build réussi — anomalie."
ok "audit-console + console-semence"

# ═══ 2 · vos secrets, sur votre machine ══════════════════════════════════════
titre "2 · la racine et le jeton"
mkdir -p "$DOSSIER" || mort "impossible de créer « $DOSSIER »"
RACINE_F="$DOSSIER/racine.secret"
JETON_F="$DOSSIER/jeton.secret"
alea() { openssl rand -hex 32 2>/dev/null || head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n'; }
# Persistés : la clé tenant est DÉRIVÉE de la racine, et des records scellés sous
# une racine ne se re-prouvent pas sous une autre. Une racine tirée à chaque
# lancement rendrait « mauvaise clé tenant » indiscernable d'une altération.
[ -s "$RACINE_F" ] || { alea > "$RACINE_F"; note "racine tirée au sort (32 octets)"; }
[ -s "$JETON_F"  ] || { alea > "$JETON_F";  note "jeton de session tiré au sort"; }
chmod 600 "$RACINE_F" "$JETON_F"
RACINE=$(cat "$RACINE_F"); JETON=$(cat "$JETON_F")
# ⚠ JAMAIS les défauts `demo-root-secret` / `demo-token` : ils sont écrits dans
#   le dépôt, donc publics, donc la clé tenant l'est aussi — et une re-preuve
#   sous une clé publique ne prouve rien à personne. La console le crie d'ailleurs
#   elle-même dans son rapport de démarrage ; ce banc ne la fait pas crier.
[ "$RACINE" = "demo-root-secret" ] && mort "racine restée au défaut public."
ok "racine et jeton, hors du dépôt, en 600, dans $DOSSIER"

export EFE_CONSOLE_ROOT="$RACINE"
export EFE_CONSOLE_TOKEN="$JETON"
export EFE_CONSOLE_TENANT="$TENANT"
export EFE_CONSOLE_SUBJECT="officer:banc"
export EFE_CONSOLE_ADDR="127.0.0.1:$PORT"

# ═══ 3 · les semences ════════════════════════════════════════════════════════
titre "3 · les trois postures"
CONFORME="$DOSSIER/records.conforme.jsonl"
MENTEUR="$DOSSIER/records.menteur.jsonl"
ALTERE="$DOSSIER/records.altere.jsonl"

# ── `--sans-semence` : on reprend les fichiers TELS QUELS ────────────────────
#
#  Sans ce mode, le banc resème à chaque lancement — et resémer ÉCRASE ce que
#  vous venez de retoucher. Les deux gestes les plus instructifs (retoucher un
#  sceau ; changer la racine) étaient donc INJOUABLES : on retouchait, on
#  relançait, et le banc redonnait du vert en ayant silencieusement défait la
#  retouche. Mesuré le 6 septembre 2026 en exécutant mes propres instructions.
#
#  Les épreuves du § 4 sont sautées ici, et il faut le dire : elles supposent des
#  fichiers fraîchement semés. Les jouer sur des fichiers que vous avez modifiés
#  ferait échouer le banc sur VOTRE modification en l'appelant un défaut.
if [ "$SEMER" -eq 0 ]; then
  for f in "$CONFORME" "$MENTEUR" "$ALTERE"; do
    [ -s "$f" ] || nm "« $f » est absent — --sans-semence reprend des fichiers existants, et il n'y en a pas. Lancez le banc une fois SANS ce drapeau."
  done
  ok "fichiers repris tels quels — aucune semence, aucune épreuve"
  note "racine lue dans $RACINE_F : si vous l'avez changée, la clé tenant a changé,"
  note "et un fichier scellé sous l'ANCIENNE racine doit virer au ROUGE."
  VERIFIER=0
else

EFE_CONSOLE_RECORDS="$CONFORME" "$SEMENCE" > "$DOSSIER/semence.conforme.txt" 2>&1 \
  || mort "la semence conforme a échoué : $(cat "$DOSSIER/semence.conforme.txt")"
EFE_CONSOLE_RECORDS="$MENTEUR" "$SEMENCE" --truquer > "$DOSSIER/semence.menteur.txt" 2>&1 \
  || mort "la semence menteuse a échoué : $(cat "$DOSSIER/semence.menteur.txt")"
ok "cas conforme  : $(wc -l < "$CONFORME" | tr -d ' ') records · $(awk '/^racine/{print substr($3,1,16)}' "$DOSSIER/semence.conforme.txt")…"
ok "cas menteur   : $(wc -l < "$MENTEUR" | tr -d ' ') records · verdict de criblage retourné À LA SEMENCE (donc scellé de bonne foi)"

# L'ALTÉRATION : elle ne se sème pas, elle se COMMET. On retouche le fichier
# scellé après coup, exactement comme le ferait quelqu'un qui a obtenu le
# registre et veut faire passer un fournisseur bloqué.
cp "$CONFORME" "$ALTERE"
if command -v perl >/dev/null; then
  perl -pi -e 's/"output":"FAIL"/"output":"PASS"/' "$ALTERE"
else
  sed -i.bak 's/"output":"FAIL"/"output":"PASS"/' "$ALTERE" && rm -f "$ALTERE.bak"
fi
if grep -q '"output":"FAIL"' "$ALTERE" || ! grep -q '"output":"PASS"' "$ALTERE"; then
  mort "l'altération n'a pas eu lieu — le banc ne peut pas prouver qu'un garde tombe s'il ne le pousse pas."
fi
ok "fichier altéré : verdict de criblage retourné APRÈS COUP, sceau non recalculé"
fi

# ═══ le moteur d'épreuve ═════════════════════════════════════════════════════
COOKIES="$DOSSIER/cookies"
demarrer() { # $1 = fichier de records ; $2 = journal
  EFE_CONSOLE_RECORDS="$1" "$CONSOLE" > "$2" 2>&1 &
  PID=$!
  # macOS n'a pas `timeout(1)` : on sonde nous-mêmes. 15 s, c'est vingt fois ce
  # qu'un démarrage sans dépendance externe demande.
  for _ in $(seq 1 60); do
    curl -sf -o /dev/null "$BASE/healthz" 2>/dev/null && return 0
    kill -0 "$PID" 2>/dev/null || return 1
    perl -e 'select(undef,undef,undef,0.25)' 2>/dev/null || sleep 1
  done
  return 1
}
arreter() { [ -n "$PID" ] && kill "$PID" 2>/dev/null; wait "$PID" 2>/dev/null; PID=""; }
ouvrir() { # remplit $PAGE avec la page du dossier, après connexion
  rm -f "$COOKIES"
  local c
  c=$(curl -s -o /dev/null -w '%{http_code}' -c "$COOKIES" -H "origin: $BASE" --data-urlencode "token=$JETON" "$BASE/login")
  [ "$c" = "303" ] || { ko "POST /login a rendu $c (attendu 303)"; return 1; }
  PAGE="$DOSSIER/page.html"
  c=$(curl -s -b "$COOKIES" -o "$PAGE" -w '%{http_code}' "$BASE/cases/$CAS")
  [ "$c" = "200" ] || { ko "GET /cases/$CAS a rendu $c (attendu 200)"; return 1; }
  return 0
}
texte() { sed 's/></>\n</g' "$1" | sed -e 's/<[^>]*>//g' -e '/^[[:space:]]*$/d'; }

epreuve() { # $1 = étiquette ; $2 = fichier ; $3 = badge attendu (vert|rouge)
  local nom="$1" fic="$2" attendu="$3"
  demarrer "$fic" "$DOSSIER/console.$nom.log" || { ko "$nom : la console n'a pas répondu sur /healthz"; note "journal : $DOSSIER/console.$nom.log"; return 1; }
  ouvrir || { arreter; return 1; }
  local vu
  if   grep -q "Altération détectée" "$PAGE"; then vu=rouge
  elif grep -q "record authentique"  "$PAGE"; then vu=vert
  else vu=indéterminé; fi
  if [ "$vu" = "$attendu" ]; then
    ok "$nom : badge $vu — comme attendu"
  else
    ko "$nom : badge $vu, alors que $attendu était attendu"
  fi
  texte "$PAGE" | grep -E "^(Statut|cassé au record|⚠ Altération|🔒 Vérifié)" | sed 's/^/        /'
  texte "$PAGE" | grep -E "sceau invalide" | sed 's/^/        /'
  arreter
}

# ═══ 4 · les trois épreuves ══════════════════════════════════════════════════
if [ "$SEMER" -eq 1 ]; then
titre "4 · les épreuves — un garde incapable de tomber ne garde rien"
epreuve conforme "$CONFORME" vert
epreuve menteur  "$MENTEUR"  vert
epreuve altere   "$ALTERE"   rouge

titre "ce que les trois épreuves viennent de montrer"
note "le fichier RETOUCHÉ tombe : la chaîne détecte l'altération et NOMME le record."
note "le cas MENTEUR passe au vert : sa chaîne est intacte — il a été scellé de"
note "bonne foi avec un verdict faux. Aucune signature au monde ne rattrape cela."
note "C'est la phrase que la console affiche en haut de chaque page, et c'est la"
note "limite honnête du produit : « Intègre » n'est jamais « correct »."
fi

if [ "$FAUTE" -ne 0 ]; then
  printf '\n\033[31m✗ une épreuve a rendu le contraire de ce qui était attendu.\033[0m\n'
  exit 1
fi

if [ "$VERIFIER" -eq 1 ]; then
  printf '\n\033[32m✓ le banc s'"'"'est prouvé : vert vert, rouge rouge.\033[0m\n'
  exit 0
fi

# ═══ 5 · le banc vous attend ═════════════════════════════════════════════════
case "$POSTURE" in
  conforme) FIC="$CONFORME" ;;
  menteur)  FIC="$MENTEUR"  ;;
  altere)   FIC="$ALTERE"   ;;
esac
titre "5 · le banc — posture « $POSTURE »"
demarrer "$FIC" "$DOSSIER/console.banc.log" || mort "la console n'a pas démarré (journal : $DOSSIER/console.banc.log)"
cat <<TEXTE

    ouvrez        $BASE/login
    jeton         $JETON
    records       $FIC

  ── ce que vous pouvez casser, et ce que ça doit répondre ──────────────────

  1. Retouchez le fichier scellé, puis relancez SANS RESEMER :
        perl -pi -e 's/BLOQUÉ/CONFORME/' $FIC
        bash scripts/banc-registre.sh --sans-semence
     → badge ROUGE, et le NUMÉRO du record cassé.
     ⚠ « --sans-semence » n'est pas décoratif : sans lui le banc resème et
       défait votre retouche, puis affiche du vert. Le geste sans le drapeau
       était écrit ici et il était FAUX — trouvé en l'exécutant.

  2. Changez la racine (donc la clé tenant dérivée), sans resemer :
        echo une-autre-racine > $RACINE_F
        bash scripts/banc-registre.sh --sans-semence
     → ROUGE aussi, et le message dit les trois causes possibles : « record
       altéré, réordonné, ou MAUVAISE CLÉ TENANT ». Mais le NUMÉRO les sépare,
       et c'est le geste 1 mis en regard du geste 2 qui le montre :

           mauvaise clé  → cassé au record 0   (rien ne vérifie, dès le premier)
           record retouché → cassé à SON numéro (ici 4, l'agrégat)

       Un numéro > 0 PROUVE donc que la clé est bonne : les records précédents
       ont vérifié sous elle. Mesuré, pas déduit — les deux gestes ci-dessus
       rendent bien 0 et 4.
     Pour revenir : effacez $RACINE_F et relancez sans le drapeau (nouvelle
     racine, nouvelle semence, tout redevient cohérent).

  3. Scellez une revue (bouton « Sceller la revue » sur la page du dossier),
     puis redémarrez le banc (avec ou sans --sans-semence, peu importe).
     → elle a DISPARU. La console scelle en mémoire et ne réécrit jamais
       $(basename "$FIC"). Mesuré, pas supposé : le fichier reste identique à
       l'octet près après un scellement réussi. Voir SECURITY.md § 4.

  4. Exportez le dossier (les quatre boutons oranges) : c'est le seul geste qui
     produise une preuve OPPOSABLE. Le badge vert n'est qu'une vue.

  5. Montez la posture menteuse, et regardez-la passer au vert :
        bash scripts/banc-registre.sh --menteur

  ── Ctrl-C pour arrêter ────────────────────────────────────────────────────

TEXTE
trap 'printf "\n  banc arrêté.\n"; kill "$PID" 2>/dev/null' EXIT INT TERM
wait "$PID"
