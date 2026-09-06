#!/usr/bin/env bash
# SPDX-License-Identifier: UNLICENSED
# ─────────────────────────────────────────────────────────────────────────────
#  LE BANC DU SEUIL (C3) — une passerelle souveraine qu'on peut CASSER.
#
#  ## Pourquoi ce script existe
#
#  Trois chemins menaient déjà à C3, et aucun n'était un banc :
#
#    docker compose up          montre le mécanisme, sur la clé de TEST PUBLIQUE
#    mon-premier-recu.sh        vous remet UN reçu opposable, puis vous dit d'arrêter
#    p1-check.sh                mesure dix propriétés et sort
#
#  Aucun ne vous laisse VARIER quoi que ce soit, et surtout aucun ne vous fait voir
#  un garde TOMBER. Or c'est la seule chose qui distingue un produit d'une
#  démonstration : « un garde incapable de tomber ne garde rien. »
#
#  ## Ce que ce banc montre, et qui n'était montré nulle part
#
#  Le cœur du Seuil tient en une distinction que personne ne croit avant de l'avoir
#  vue : **intégrité ≠ complétude**.
#
#      un bundle TRONQUÉ se re-prouve PARFAITEMENT.
#
#  Le préfixe d'une chaîne scellée est lui-même une chaîne scellée valide. Rien, de
#  l'intérieur, ne trahit ce qui a été coupé. C'est l'ANCRE — publiée hors du bundle,
#  remise par un canal distinct — qui l'attrape, et elle l'attrape par un COMPTAGE,
#  sans clé et sans cryptographie. Le banc joue les deux, côte à côte.
#
#  ## Les deux étages
#
#    (défaut)       la passerelle sans features : runs, écrêtages, budgets, codes de
#                   route, reçu portable, re-preuve, troncature, altération, usage.
#                   Aucun démon, aucune base, quelques secondes.
#
#    --production   les quatre features que le garde exige. Trois formes d'`exit 78`
#                   et le discriminant bruyant/silencieux de la frontière. ⚠ Ne
#                   demande NI Docker NI PostgreSQL : le préflight lit des INTENTIONS
#                   d'environnement, jamais une connexion. Il coûte un `cargo build`.
#
#  ## Les trois états
#     0  le banc s'est prouvé — chaque garde a tenu ET est tombé quand il devait
#     1  une épreuve a rendu le CONTRAIRE de ce qui était attendu
#     3  NON MESURÉ — l'instrument n'a pas pu regarder (cargo, curl, python3, un port)
#
#  Usage :
#     bash scripts/banc-seuil.sh                    # monte le banc, s'éprouve, RESTE ouvert
#     bash scripts/banc-seuil.sh --production       # + l'étage production (exit 78)
#     bash scripts/banc-seuil.sh --verifier         # joue les épreuves et SORT (0/1/3)
#     bash scripts/banc-seuil.sh --verifier --production
#
#     BANC_PORT=19099 bash scripts/banc-seuil.sh    # si 18088 est pris
# ─────────────────────────────────────────────────────────────────────────────
set -uo pipefail
cd "$(dirname "$0")/.."

ok()    { printf '  \033[32m✓\033[0m %s\n' "$*"; }
ko()    { printf '  \033[31m✗\033[0m %s\n' "$*"; FAUTE=$((FAUTE+1)); }
mort()  { printf '  \033[31m✗\033[0m %s\n' "$*"; exit 1; }
nm()    { printf '  \033[33m·\033[0m NON MESURÉ — %s\n' "$*"; exit 3; }
note()  { printf '      %s\n' "$*"; }
titre() { printf '\n\033[1;34m%s\033[0m\n' "$*"; }
#  L'épreuve élémentaire : on COMPARE un mesuré à un attendu, et on imprime les deux.
#  Jamais « ✓ le garde marche » — toujours « ✓ <geste> → <mesuré> (attendu <attendu>) ».
#  Un vert qui ne montre pas son chiffre est un vert qu'on ne peut pas contredire.
att()   { # $1 = geste · $2 = mesuré · $3 = attendu
  if [ "$2" = "$3" ]; then ok "$(printf '%-52s → %s' "$1" "$2")"
  else ko "$(printf '%-52s → %s   (attendu %s)' "$1" "$2" "$3")"; fi
}

FAUTE=0
VERIFIER=0
PRODUCTION=0
DOSSIER=""
for a in "$@"; do
  case "$a" in
    --verifier)   VERIFIER=1 ;;
    --production) PRODUCTION=1 ;;
    -*) mort "option inconnue « $a » — voir l'en-tête de ce fichier." ;;
    *)  DOSSIER="$a" ;;
  esac
done
DOSSIER="${DOSSIER:-./banc-seuil}"
PORT="${BANC_PORT:-18088}"
BASE="http://127.0.0.1:$PORT"

GW=""
menage() { [ -n "$GW" ] && kill "$GW" 2>/dev/null; }
trap menage EXIT

# ═══ 0 · l'instrument ════════════════════════════════════════════════════════
titre "0 · l'instrument"
for outil in cargo curl python3 openssl; do
  command -v "$outil" >/dev/null || nm "« $outil » est absent — le banc s'en sert pour $(
    case $outil in cargo) echo "compiler la passerelle";; curl) echo "l'interroger";;
      python3) echo "lire le JSON qu'elle rend, sans jq";; openssl) echo "tirer votre master au sort";; esac)."
done
if command -v lsof >/dev/null && lsof -nP -iTCP:"$PORT" -sTCP:LISTEN >/dev/null 2>&1; then
  nm "le port $PORT est déjà pris. Relancez avec : BANC_PORT=19099 bash scripts/banc-seuil.sh"
fi
ok "cargo, curl, python3, openssl · port $PORT libre"

# ═══ 1 · les binaires ════════════════════════════════════════════════════════
titre "1 · les quatre binaires du Seuil"
note "cargo build (passerelle + pilote + les DEUX vérifieurs) — long la première fois."
cargo build --quiet -p efe-swarm-gateway -p scella-preuve -p verify-mini -p provenance-verify 2>&1 | sed 's/^/      /'
for b in efe-swarm-gateway scella-preuve verify-mini provenance-verify; do
  [ -x "target/debug/$b" ] || mort "target/debug/$b absent après un build réussi — anomalie."
done
ok "efe-swarm-gateway · scella-preuve · verify-mini · provenance-verify"

mkdir -p "$DOSSIER" || mort "impossible de créer « $DOSSIER »"
#  ⚠ On COPIE la passerelle dans le dossier de travail. `target/debug/` est un chemin
#  PARTAGÉ : n'importe quel `cargo build` lancé pendant que le banc tourne (un éditeur,
#  une autre session) y réinstalle un binaire, et le processus du banc n'aurait plus
#  aucun rapport avec ce que le banc affirme. La leçon est celle de `p1-check.sh`,
#  payée une première fois par un contrôle qui invalidait lui-même sa propre mesure.
cp target/debug/efe-swarm-gateway "$DOSSIER/passerelle"
cp target/debug/verify-mini "$DOSSIER/verify-mini"
cp target/debug/provenance-verify "$DOSSIER/provenance-verify"
PASSERELLE="$DOSSIER/passerelle"; VM="$DOSSIER/verify-mini"; PV="$DOSSIER/provenance-verify"
ok "copiés dans « $DOSSIER » — un build concurrent ne peut plus les remplacer sous nos pieds"

# ═══ 2 · vos clés ════════════════════════════════════════════════════════════
titre "2 · votre master — tiré ici, il ne part nulle part"
MASTER_F="$DOSSIER/master.hex"
[ -s "$MASTER_F" ] || openssl rand -hex 32 > "$MASTER_F"
chmod 600 "$MASTER_F"
MASTER="$(tr -d '\n' < "$MASTER_F")"
[ "${#MASTER}" -ge 64 ] || mort "le master tiré ne fait pas 64 caractères hexadécimaux."
ok "master 32 octets, en 600 : $DOSSIER/master.hex"
note "sans lui, la passerelle replie sur la CLÉ DE TEST PUBLIQUE et le dit :"
note "« la provenance serait FORGEABLE (verify non fiable) ». Un reçu scellé sous"
note "une clé que tout le monde connaît n'oppose rien à personne."

# ═══ 3 · la passerelle démarre, et déclare ce qu'elle sert ════════════════════
titre "3 · le rapport de démarrage — ce qui est SERVI, ce qui est DÉGRADÉ"
export EFE_SWARM_GATEWAY_ADDR="127.0.0.1:$PORT"
export EFE_SWARM_ANCHOR_LOG="$DOSSIER/anchors.log"
export EFE_NANO_AUDIT_HMAC_MASTER="$MASTER"
export EFE_SWARM_BUNDLE_ENABLED=true
rm -f "$DOSSIER/anchors.log"
"$PASSERELLE" > "$DOSSIER/passerelle.log" 2>&1 &
GW=$!
for _ in $(seq 1 60); do
  curl -sf -o /dev/null "$BASE/healthz" 2>/dev/null && break
  kill -0 "$GW" 2>/dev/null || mort "la passerelle est morte au démarrage : $(tail -3 "$DOSSIER/passerelle.log")"
  perl -e 'select(undef,undef,undef,0.25)' 2>/dev/null || sleep 1
done
curl -sf -o /dev/null "$BASE/healthz" || mort "la passerelle ne répond pas sur /healthz (journal : $DOSSIER/passerelle.log)"
SERVIES=$(grep -c "SERVIE" "$DOSSIER/passerelle.log")
DEGRADEES=$(grep -c "DÉGRADÉE" "$DOSSIER/passerelle.log")
ok "en écoute · $SERVIES capacité(s) SERVIE(s), $DEGRADEES DÉGRADÉE(s) — chacune nommée, avec son coût"
grep -E "provenance |ancre  |export de bundle" "$DOSSIER/passerelle.log" | sed 's/^/      /'
att "provenance NON-FORGEABLE (master posé)" \
    "$(grep -c 'provenance *SERVIE' "$DOSSIER/passerelle.log")" "1"
att "ancre publiée (la troncature devient détectable)" \
    "$(grep -c 'ancre *SERVIE' "$DOSSIER/passerelle.log")" "1"

# ═══ 4 · des runs VARIÉS — les plafonds sont-ils des plafonds ? ═══════════════
titre "4 · des runs variés — ce qu'un client demande, ce que le serveur accorde"
poste() { # $1 = corps json → imprime le code HTTP, écrit la réponse dans $DOSSIER/rep.json
  curl -s -o "$DOSSIER/rep.json" -w '%{http_code}' -X POST "$BASE/v1/swarm/run" \
       -H 'content-type: application/json' -d "$1"
}
champ() { python3 -c "
import json,io,sys
try: d=json.load(io.open('$DOSSIER/rep.json'))
except Exception: print('illisible'); sys.exit()
for k in '$1'.split('.'):
    d = d.get(k) if isinstance(d,dict) else None
    if d is None: print('absent'); sys.exit()
print(d)"; }

RUN='{"task":{"text":"Résumer la clause de résiliation."},"topology":{"kind":"auto"}}'
att "un run nominal" "$(poste "$RUN")" "200"
att "  · records scellés" "$(champ run.provenance.records)" "3"
att "  · provenance vérifiée par la passerelle" "$(champ run.provenance.verified)" "True"

#  L'ÉCRÊTAGE. Le mot du code est « plafonnée côté serveur », pas « refusée » :
#  « un client ne dimensionne pas la machine d'autrui ». Un refus (400) aurait été un
#  autre produit — celui-ci sert la requête, en la ramenant à ce qu'il accepte, et il
#  l'écrit dans le reçu. C'est vérifiable par le client, ce qu'une promesse n'est pas.
poste '{"task":{"text":"x"},"topology":{"kind":"auto"},"width":4}' >/dev/null
att "width demandé 4 (sous le plafond)" "$(champ run.accounting.width)" "4"
poste '{"task":{"text":"x"},"topology":{"kind":"auto"},"width":100000}' >/dev/null
att "width demandé 100000 → ÉCRÊTÉ, et le reçu le dit" "$(champ run.accounting.width)" "32"

#  LE BUDGET, dans ses deux politiques. `stop` termine proprement le run entamé ;
#  `error` refuse AVANT de dépenser, en 402, et nomme les deux chiffres.
poste '{"task":{"text":"x"},"topology":{"kind":"auto"},"budget":{"max_cents":1}}' >/dev/null
att "budget 1¢, politique « stop » → arrêt propre" "$(champ run.termination)" "budget_stopped"
att "  · et il n'a pas dépassé" "$(champ run.budget.spent_cents)" "1"
att "budget 1¢, politique « error » → refus AVANT dépense" \
    "$(poste '{"task":{"text":"x"},"topology":{"kind":"auto"},"budget":{"max_cents":1,"on_exceed":"error"}}')" "402"
note "$(champ message)"

#  LES REQUÊTES MALFORMÉES. Le message NOMME le champ manquant : un 400 muet coûte
#  une demi-heure à qui l'reçoit.
att "corps sans « topology »" "$(poste '{"task":{"text":"x"}}')" "400"
note "$(champ message)"
att "corps qui n'est pas du JSON" "$(poste 'ceci n_est pas du json')" "400"

# ═══ 5 · le reçu portable ════════════════════════════════════════════════════
titre "5 · le reçu portable — le bundle, et l'ancre publiée HORS de lui"
poste "$RUN" >/dev/null
ID="$(champ run_id)"
[ "${#ID}" = "36" ] || mort "run_id inattendu : « $ID »"
att "GET /v1/swarm/verify/<le run>" "$(curl -s -o "$DOSSIER/verdict.json" -w '%{http_code}' "$BASE/v1/swarm/verify/$ID")" "200"
att "GET /v1/swarm/verify/<uuid inconnu>" "$(curl -s -o /dev/null -w '%{http_code}' "$BASE/v1/swarm/verify/00000000-0000-0000-0000-000000000000")" "404"
att "GET /v1/swarm/verify/<pas un uuid>" "$(curl -s -o /dev/null -w '%{http_code}' "$BASE/v1/swarm/verify/pas-un-uuid")" "400"
note "404 et 400 ne disent pas la même chose : « je ne l'ai pas » n'est pas « ce n'est pas un identifiant »."

att "GET /v1/swarm/bundle/<le run>" "$(curl -s -o "$DOSSIER/bundle.json" -w '%{http_code}' "$BASE/v1/swarm/bundle/$ID")" "200"
python3 - "$DOSSIER" <<'PY' || mort "le bundle rendu n'a pas la forme attendue"
import json, sys, pathlib
d = pathlib.Path(sys.argv[1]); b = json.loads((d/"bundle.json").read_text())
(d/"recu.jsonl").write_text(b["bundle_jsonl"])
(d/"racine.txt").write_text(b["manifest_root"])
print(f"      {b['node_count']} nœud(s) · racine {b['manifest_root'][:16]}… · ancré : {b['anchored']}")
PY
RACINE="$(tr -d '\n' < "$DOSSIER/racine.txt")"
NREC=$(python3 -c "import io;print(sum(1 for l in io.open('$DOSSIER/recu.jsonl') if l.strip()))")
att "records dans le reçu" "$NREC" "3"
#  L'ancre est publiée à l'EXPORT, pas au run — mesuré : une ligne par bundle servi.
#  Elle porte : rang · run · tenant · racine · NOMBRE DE NŒUDS · horodatage.
#  ⚠ La première version de cette section comparait le compte de lignes À LUI-MÊME.
#  Elle passait toujours, et ne mesurait rien. On asserte donc la PROPRIÉTÉ dont
#  dépend tout le § 7 : l'ancre engage le run, sa racine, et son nombre de nœuds.
ANCRE="$(grep "$ID" "$DOSSIER/anchors.log" | tail -1)"
att "l'ancre nomme CE run" "$(printf '%s' "$ANCRE" | cut -f2)" "$ID"
att "l'ancre engage la racine publiée" "$(printf '%s' "$ANCRE" | cut -f4)" "$RACINE"
att "l'ancre engage le NOMBRE DE NŒUDS" "$(printf '%s' "$ANCRE" | cut -f5)" "$NREC"
note "c'est ce dernier chiffre, et lui seul, qui rendra une troncature visible au § 7."

# ═══ 6 · la re-preuve — et LES DEUX PILES DE CLÉS ════════════════════════════
titre "6 · re-prouver le reçu — et le piège des deux piles"
#  Le RUN est scellé sous la clé de la pile A ; le BUNDLE exporté sous celle de la
#  pile B, dérivée de la première. Les confondre fait échouer la re-preuve « au
#  record 0 » — un message qui accuse la CHAÎNE alors que le fautif est la CLÉ.
EFE_NANO_AUDIT_HMAC_MASTER="$MASTER" "$PASSERELLE" --cle-tenant --bundle > "$DOSSIER/cle-B.hex" 2>/dev/null
EFE_NANO_AUDIT_HMAC_MASTER="$MASTER" "$PASSERELLE" --cle-tenant          > "$DOSSIER/cle-A.hex" 2>/dev/null
chmod 600 "$DOSSIER/cle-A.hex" "$DOSSIER/cle-B.hex"
att "les deux clés dérivées sont DIFFÉRENTES" \
    "$([ "$(cat "$DOSSIER/cle-A.hex")" != "$(cat "$DOSSIER/cle-B.hex")" ] && echo oui || echo non)" "oui"

"$VM" "$DOSSIER/recu.jsonl" --key-file "$DOSSIER/cle-B.hex" --root "$RACINE" > "$DOSSIER/vB.txt" 2>&1
att "verify-mini, pile B (la bonne), racine attendue" "$?" "0"
note "$(head -1 "$DOSSIER/vB.txt")"

"$VM" "$DOSSIER/recu.jsonl" --key-file "$DOSSIER/cle-A.hex" --root "$RACINE" > "$DOSSIER/vA.txt" 2>&1
att "verify-mini, pile A (la mauvaise) → doit TOMBER" "$?" "1"
note "$(grep -E 'cassé|motif' "$DOSSIER/vA.txt" | tr '\n' ' ')"
note "⚠ « cassé au record 0 » ne veut PAS dire « le premier record est altéré » : une"
note "mauvaise clé échoue dès le premier sceau. Un numéro > 0 prouve que la clé est bonne."

"$VM" "$DOSSIER/recu.jsonl" --key-file "$DOSSIER/cle-B.hex" --root "$(printf 'a%.0s' $(seq 64))" > "$DOSSIER/vR.txt" 2>&1
att "verify-mini, bonne clé mais racine FAUSSE → tombe" "$?" "1"
note "et le motif est DISTINCT du précédent : $(grep -o 'intégrité OK mais [^(]*' "$DOSSIER/vR.txt" | head -1)"

# ═══ 7 · LA TRONCATURE — la démonstration qui vaut le produit ════════════════
titre "7 · la troncature — pourquoi l'ancre existe"
python3 -c "
import io
ls=[l for l in io.open('$DOSSIER/recu.jsonl') if l.strip()]
io.open('$DOSSIER/recu.TRONQUE.jsonl','w').writelines(ls[:-1])"
att "records après coupe du dernier" \
    "$(python3 -c "import io;print(sum(1 for l in io.open('$DOSSIER/recu.TRONQUE.jsonl') if l.strip()))")" "2"

#  ⚠ CE RÉSULTAT EST LE PLUS IMPORTANT DU BANC, ET C'EST UN ZÉRO.
"$VM" "$DOSSIER/recu.TRONQUE.jsonl" --key-file "$DOSSIER/cle-B.hex" > "$DOSSIER/t1.txt" 2>&1
att "verify-mini SEUL sur le tronqué → il PASSE" "$?" "0"
note "$(head -1 "$DOSSIER/t1.txt")"
note "Le préfixe d'une chaîne scellée EST une chaîne scellée valide. Rien, de"
note "l'intérieur, ne trahit ce qui a été coupé. Un vérifieur sans ancre atteste"
note "l'INTÉGRITÉ ; il ne peut rien dire de la COMPLÉTUDE."

"$VM" "$DOSSIER/recu.TRONQUE.jsonl" --key-file "$DOSSIER/cle-B.hex" --root "$RACINE" >/dev/null 2>&1
att "  + la racine publiée → tombe" "$?" "1"
"$PV" "$DOSSIER/recu.TRONQUE.jsonl" --key-file "$DOSSIER/cle-B.hex" --anchor "$DOSSIER/anchors.log" --run "$ID" > "$DOSSIER/t3.txt" 2>&1
att "  + l'ANCRE (par un simple comptage) → tombe" "$?" "1"
note "$(grep -o 'compte de nœuds.*' "$DOSSIER/t3.txt" | head -1)"
#  La contre-épreuve : le même instrument sur le reçu INTACT doit rendre 0, sans quoi
#  il refuserait tout et son « ✗ » ne vaudrait rien.
"$PV" "$DOSSIER/recu.jsonl" --key-file "$DOSSIER/cle-B.hex" --anchor "$DOSSIER/anchors.log" --run "$ID" > "$DOSSIER/t4.txt" 2>&1
att "  contre-épreuve : le reçu INTACT contre l'ancre" "$?" "0"
note "$(head -1 "$DOSSIER/t4.txt")"

# ═══ 8 · L'ALTÉRATION ════════════════════════════════════════════════════════
titre "8 · l'altération — un octet retourné, et le record est NOMMÉ"
python3 -c "
import io
ls=[l for l in io.open('$DOSSIER/recu.jsonl') if l.strip()]
ls[1]=ls[1].replace('\"agent_role\":\"','\"agent_role\":\"X',1)
io.open('$DOSSIER/recu.ALTERE.jsonl','w').writelines(ls)"
"$VM" "$DOSSIER/recu.ALTERE.jsonl" --key-file "$DOSSIER/cle-B.hex" > "$DOSSIER/a1.txt" 2>&1
att "verify-mini sur le record 1 retouché → tombe" "$?" "1"
att "  · et il nomme LEQUEL" "$(grep -o 'cassé au record : [0-9]*' "$DOSSIER/a1.txt" | grep -o '[0-9]*$')" "1"

# ═══ 9 · LE TROISIÈME ÉTAT — l'usage n'est pas un verdict ════════════════════
titre "9 · le troisième état — « je n'ai pas pu regarder » ≠ « c'est faux »"
#  Un vérifieur qui rendrait 1 sur une erreur d'usage ferait passer sa propre panne
#  pour une altération du reçu. C'est la faute que les trois états existent pour
#  interdire, et elle se vérifie ici en quatre gestes.
"$VM" "$DOSSIER/recu.jsonl"                                            >/dev/null 2>&1; att "sans clé du tout" "$?" "2"
"$VM" "$DOSSIER/recu.jsonl" --key-hex aa --key-file "$DOSSIER/cle-B.hex" >/dev/null 2>&1; att "deux clés à la fois" "$?" "2"
"$VM" "$DOSSIER/absent.jsonl" --key-file "$DOSSIER/cle-B.hex"          >/dev/null 2>&1; att "fichier inexistant" "$?" "2"
"$VM" "$DOSSIER/recu.jsonl" --key-file "$DOSSIER/cle-B.hex" --root --json >/dev/null 2>&1; att "« --root --json » : option volée comme valeur" "$?" "2"
"$PV" "$DOSSIER/recu.jsonl" --key-file "$DOSSIER/cle-B.hex" --anchor "$DOSSIER/anchors.log" >/dev/null 2>&1; att "--anchor sans --run" "$?" "2"

# ═══ 10 · L'INTERRUPTEUR QUI NE FAIT RIEN — et qui le crie ═══════════════════
titre "10 · un interrupteur posé qui n'a aucun effet doit le DIRE"
kill "$GW" 2>/dev/null; wait "$GW" 2>/dev/null; GW=""
EFE_SWARM_BUNDLE_ENABLED=1 "$PASSERELLE" > "$DOSSIER/switch.log" 2>&1 &
GW=$!
for _ in $(seq 1 60); do curl -sf -o /dev/null "$BASE/healthz" 2>/dev/null && break; perl -e 'select(undef,undef,undef,0.25)' 2>/dev/null || sleep 1; done
att "EFE_SWARM_BUNDLE_ENABLED=1 → REFUSÉ, et annoncé" "$(grep -c "REFUSÉ (attendu : exactement .true." "$DOSSIER/switch.log")" "1"
note "$(grep -A1 'BUNDLE_ENABLED' "$DOSSIER/switch.log" | tail -1 | sed 's/^ *//')"
att "  · et la route d'export n'est PAS montée : 404, pas 501" \
    "$(curl -s -o /dev/null -w '%{http_code}' "$BASE/v1/swarm/bundle/$ID")" "404"
note "404 et non 501 : une surface désactivée ne doit pas trahir son existence."
kill "$GW" 2>/dev/null; wait "$GW" 2>/dev/null; GW=""

# ═══ 11 · L'ÉTAGE PRODUCTION — le garde qui REFUSE DE DÉMARRER ═══════════════
if [ "$PRODUCTION" -eq 1 ]; then
titre "11 · le garde de configuration — « exit 78 » (EX_CONFIG)"
note "cargo build --features postgres,oidc,pii,frontier-llm — long la première fois."
cargo build --quiet -p efe-swarm-gateway --features postgres,oidc,pii,frontier-llm 2>&1 | sed 's/^/      /'
#  ⚠ On demande au binaire ce qu'il PORTE. « le fichier existe » est un PROXY, pas la
#  propriété : un `cargo build` sans features y réinstalle un binaire NU, et le garde
#  disparaîtrait avec sa feature — le banc mesurerait alors un démarrage réussi et
#  l'appellerait un succès. Puis on le COPIE, pour que ce ne soit plus possible.
PORTEES="$(./target/debug/efe-swarm-gateway --profile-report 2>/dev/null \
  | python3 -c "import json,sys;print(','.join(sorted(json.load(sys.stdin).get('features',[]))))" 2>/dev/null)"
att "les quatre features sont PORTÉES par le binaire" "$PORTEES" "frontier-llm,oidc,pii,postgres"
[ "$PORTEES" = "frontier-llm,oidc,pii,postgres" ] || nm "le binaire ne porte pas les quatre features (un build sans features l'a remplacé). Relancez, ou isolez avec CARGO_TARGET_DIR."
cp target/debug/efe-swarm-gateway "$DOSSIER/passerelle-prod"
PROD="$DOSSIER/passerelle-prod"

#  Les tests unitaires du module `frontier` n'existent QUE sous `frontier-llm`, donc
#  `cargo test --workspace` — ce que joue la CI — les compte pour ZÉRO. Les jouer ici
#  est la seule façon qu'ils tournent ailleurs que sous `p1-check.sh`, qui exige Docker.
FT=$(cargo test -q --locked -p efe-swarm-gateway --features frontier-llm --lib frontier::tests 2>&1 | grep -E '^test result' | head -1)
att "les tests du module « frontier » tournent (invisibles à --workspace)" \
    "$(echo "$FT" | grep -o '[0-9]* passed' | grep -o '^[0-9]*')" "6"
att "  · et aucun n'échoue" "$(echo "$FT" | grep -o '[0-9]* failed' | grep -o '^[0-9]*')" "0"

OIDC_PUB="$DOSSIER/oidc.pub"
[ -s "$OIDC_PUB" ] || { openssl ecparam -name prime256v1 -genkey -noout -out "$DOSSIER/oidc.pem" 2>/dev/null
                        openssl ec -in "$DOSSIER/oidc.pem" -pubout -out "$OIDC_PUB" 2>/dev/null
                        chmod 600 "$DOSSIER/oidc.pem"; }
PUB="$(cat "$OIDC_PUB")"; FAUSSE_DB="postgres://x:y@127.0.0.1:1/aucune"

#  Chaque essai DOIT sortir : ce sont des refus de démarrage. `env -i` coupe tout
#  l'environnement hérité, sans quoi une variable posée plus haut dans ce script
#  ferait passer un garde pour l'autre — un banc qui se pollue lui-même.
essai78() { # $1 = étiquette ; $2 = fragment attendu dans le FATAL ; reste = env
  local nom="$1" attendu="$2"; shift 2
  env -i PATH="$PATH" EFE_SWARM_GATEWAY_ADDR="127.0.0.1:$((PORT+7))" "$@" "$PROD" > "$DOSSIER/g78.log" 2>&1
  local code=$?
  att "$nom" "$code" "78"
  if grep -q "$attendu" "$DOSSIER/g78.log"; then ok "  · et le message NOMME ce qui manque"
  else ko "  · le message ne contient pas « $attendu »"; fi
  grep -m1 "^FATAL" "$DOSSIER/g78.log" | cut -c1-150 | sed 's/^/        /'
}
essai78 "master FOURNI mais < 32 octets" "≥ 32 octets requis" \
        EFE_NANO_AUDIT_HMAC_MASTER=trop-court
essai78 "frontière ON, sans auth OIDC" "sans authentification" \
        EFE_LLM_FRONTIER_ENABLED=true EFE_NANO_AUDIT_HMAC_MASTER="$MASTER"
essai78 "frontière ON + OIDC, sans store durable" "perdus à chaque rollout" \
        EFE_LLM_FRONTIER_ENABLED=true EFE_NANO_AUDIT_HMAC_MASTER="$MASTER" \
        EFE_OIDC_PUBLIC_KEY_PEM="$PUB" EFE_OIDC_LLM_AUDIENCE=efe-llm

#  LA CONTRE-ÉPREUVE DU GARDE : avec les quatre invariants réunis, il doit LAISSER
#  PASSER. Un garde qui refuse tout ne garde rien non plus — il empêche seulement.
env -i PATH="$PATH" EFE_SWARM_GATEWAY_ADDR="127.0.0.1:$((PORT+7))" \
  EFE_LLM_FRONTIER_ENABLED=true EFE_NANO_AUDIT_HMAC_MASTER="$MASTER" \
  EFE_OIDC_PUBLIC_KEY_PEM="$PUB" EFE_OIDC_LLM_AUDIENCE=efe-llm DATABASE_URL="$FAUSSE_DB" \
  "$PROD" > "$DOSSIER/g78ok.log" 2>&1
att "contre-épreuve : les 4 réunis → le préflight LAISSE PASSER" \
    "$(grep -c 'préflight frontière OK' "$DOSSIER/g78ok.log")" "1"
note "il échoue ensuite sur la base injoignable — et c'est un AUTRE code, pas 78 :"
note "« config invalide » et « la base ne répond pas » ne se corrigent pas pareil."

titre "11 bis · crier sur un acte, se taire sur une absence"
#  Le discriminant de `frontier::enregistrer`. Jusqu'au 6 septembre 2026 il était juste
#  et pourtant faux : `ApiKey::from_env` typait « variable absente » comme « valeur
#  rejetée », si bien que la passerelle affirmait « ce n'est pas une clé manquante »
#  TROIS FOIS, sur la posture la plus fréquente qui soit — frontière activée sans clé.
front() { env -i PATH="$PATH" EFE_SWARM_GATEWAY_ADDR="127.0.0.1:$((PORT+8))" \
  EFE_LLM_FRONTIER_ENABLED=true EFE_NANO_AUDIT_HMAC_MASTER="$MASTER" \
  EFE_OIDC_PUBLIC_KEY_PEM="$PUB" EFE_OIDC_LLM_AUDIENCE=efe-llm DATABASE_URL="$FAUSSE_DB" \
  "$@" "$PROD" > "$DOSSIER/front.log" 2>&1
  grep -c "NON enregistré" "$DOSSIER/front.log"; }
att "aucune clé fournisseur → SILENCE (0 cri)" "$(front)" "0"
note "et l'avertissement global, lui, reste — il est vrai : « aucune clé provider »."
att "ANTHROPIC_API_KEY au mauvais préfixe → 1 cri" "$(front ANTHROPIC_API_KEY=pas-le-bon-prefixe)" "1"
note "$(grep -m1 'NON enregistré' "$DOSSIER/front.log" | cut -c1-140)"
att "EFE_ANTHROPIC_URL en http distant → 1 cri" \
    "$(front ANTHROPIC_API_KEY=sk-ant-xxxx EFE_ANTHROPIC_URL=http://evil.example/v1/messages)" "1"
note "$(grep -m1 -o "n'est permis que vers la machine locale.*" "$DOSSIER/front.log" | cut -c1-120)"
fi

# ═══ le verdict ══════════════════════════════════════════════════════════════
titre "ce que le banc vient d'établir"
note "· un bundle TRONQUÉ se re-prouve parfaitement — l'intégrité ne dit rien de la"
note "  complétude, et c'est l'ancre, par un COMPTAGE, qui referme l'écart."
note "· les plafonds ÉCRÊTENT et l'écrivent dans le reçu, au lieu de promettre."
note "· les trois états sont tenus : un défaut d'usage rend 2, jamais 1 — une panne"
note "  d'instrument ne doit jamais emprunter le vocabulaire de ce qu'il mesure."
[ "$PRODUCTION" -eq 1 ] && note "· en posture production, la mauvaise configuration REFUSE de démarrer (78) et nomme l'invariant manquant."

if [ "$FAUTE" -ne 0 ]; then
  printf '\n\033[31m✗ %s épreuve(s) ont rendu le contraire de ce qui était attendu.\033[0m\n' "$FAUTE"
  exit 1
fi
if [ "$VERIFIER" -eq 1 ]; then
  printf '\n\033[32m✓ le banc du Seuil s'"'"'est prouvé — chaque garde a tenu, et chacun est tombé quand il devait.\033[0m\n'
  exit 0
fi

# ═══ le banc vous attend ═════════════════════════════════════════════════════
titre "le banc — la passerelle vous attend"
"$PASSERELLE" > "$DOSSIER/passerelle.log" 2>&1 &
GW=$!
for _ in $(seq 1 60); do curl -sf -o /dev/null "$BASE/healthz" 2>/dev/null && break; perl -e 'select(undef,undef,undef,0.25)' 2>/dev/null || sleep 1; done
cat <<TEXTE

    passerelle    $BASE
    dossier       $DOSSIER
    dernier run   $ID

  ── ce que vous pouvez casser, et ce que ça doit répondre ──────────────────

  1. Demandez une machine qui n'est pas la vôtre :
       curl -s -X POST $BASE/v1/swarm/run -H 'content-type: application/json' \\
         -d '{"task":{"text":"x"},"topology":{"kind":"auto"},"width":100000}' \\
         | python3 -c 'import json,sys;print(json.load(sys.stdin)["run"]["accounting"]["width"])'
     → 32. Écrêté, et écrit dans VOTRE reçu — pas dans une promesse.

  2. Re-prouvez un reçu TRONQUÉ, sans l'ancre. Le banc en a déjà fabriqué un :
       $VM $DOSSIER/recu.TRONQUE.jsonl --key-file $DOSSIER/cle-B.hex ; echo \$?
     → 0. Il PASSE. C'est le résultat le plus important de ce produit.
     Puis le même fichier, contre l'ancre reçue par un canal distinct :
       $PV $DOSSIER/recu.TRONQUE.jsonl --key-file $DOSSIER/cle-B.hex \\
         --anchor $DOSSIER/anchors.log --run $ID ; echo \$?
     → 1, « compte de nœuds NON concordant — attendu 3, obtenu 2 ».
     Pour en couper un vous-même : « sed '\$d' » (POSIX). ⚠ PAS « head -n -1 »,
     qui est une extension GNU : le head de macOS répond « illegal line count ».

  3. Re-prouvez avec la MAUVAISE pile de clés :
       $VM $DOSSIER/recu.jsonl --key-file $DOSSIER/cle-A.hex
     → « cassé au record 0 ». Le run est scellé sous la pile A, le bundle sous la
       pile B. Un 0 accuse la clé ; un numéro > 0 accuse le record.

  4. Retirez votre master et regardez la passerelle se dénoncer :
       env -i PATH=\$PATH $PASSERELLE --profile-report \\
         | python3 -c 'import json,sys;print([c["detail"] for c in json.load(sys.stdin)["capabilities"] if c["capability"]=="provenance"][0])'
     → « repli sur la clé de TEST PUBLIQUE — le reçu devient FORGEABLE ».
     ⚠ « --profile-report » rend le profil et SORT. Lancer la passerelle sans ce
       drapeau pour lire son rapport ferait pendre votre terminal : elle sert.

  5. Posez un interrupteur à une valeur qui n'existe pas :
       EFE_SWARM_BUNDLE_ENABLED=1 $PASSERELLE --profile-report \\
         | python3 -c 'import json,sys;print(json.load(sys.stdin)["invalides"])'
     → le switch, ce que vous avez FOURNI, et ce qui était ATTENDU. Avec « true »,
       la liste est vide : c'est la contre-épreuve, et elle tient en un mot changé.

  6. L'étage production — trois refus de démarrer, sans base ni Docker :
       bash scripts/banc-seuil.sh --verifier --production

  ── Ctrl-C pour arrêter ────────────────────────────────────────────────────

TEXTE
trap 'printf "\n  banc arrêté.\n"; kill "$GW" 2>/dev/null' EXIT INT TERM
wait "$GW"
