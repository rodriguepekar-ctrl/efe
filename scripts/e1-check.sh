#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
#  É1 / É2 — LA PORTE : le verdict de l'exécution en conteneur.
#
#  L'architecture de É1 (docs/architecture/E1-OUVRIR-LA-PORTE-*.tex)
#  décrit cinq couches qui existent et ont tourné — préconditions,
#  contexte de build, compilation, image, orchestration — puis une
#  SIXIÈME qui n'existait pas : « comment sait-on que la porte s'est
#  ouverte ? ». Elle écrivait, et c'est la phrase qui a fait naître ce
#  fichier : « son absence rend les cinq autres NON MESURABLES ».
#
#  C'est le geste 3 de son ordre du travail, et il vient AVANT les
#  portes jamais ouvertes — écrit après, il n'aurait servi qu'à
#  confirmer ce qu'on a déjà cru voir.
#
#  ── CE QU'IL NE FAUT PAS CROIRE ───────────────────────────────────
#
#  1. Le code de retour de `docker compose up`.
#     Il est juste quand un conteneur a démarré et qu'il échoue, et
#     MUET quand l'échec précède tout conteneur : réseau, volume,
#     image absente, port déjà pris. Le 23 août, un tel échec a rendu
#     0 ; le 4 septembre, un port pris a rendu 1. Un témoin dont le
#     verdict dépend de la version de Compose n'est pas un témoin —
#     quelle que soit la valeur qu'il rend ce jour-là.
#
#  2. `ExitCode` seul, dans `docker compose ps -a --format json`.
#     ⚠ MESURÉ le 4 septembre, et c'est le piège le plus vicieux que
#     ce contrôle ait rencontré : 8080 était pris, AUCUN conteneur
#     n'avait démarré, et les deux lignes JSON portaient
#         "State":"created"   "ExitCode":0
#     Zéro est la valeur PAR DÉFAUT d'un conteneur qui n'a jamais
#     tourné. Un contrôle qui lit le code sans lire l'état FABRIQUE le
#     succès qu'il prétend mesurer — il aurait annoncé « les deux
#     conteneurs ont rendu 0 » d'une exécution qui n'a pas eu lieu.
#     ⟹ ici, l'ÉTAT d'abord. Le code n'est lu QUE si State=exited.
#
#  ── LES TROIS ÉTATS ───────────────────────────────────────────────
#
#     0   mesuré, et la porte s'ouvre
#     1   mesuré, et elle ne s'ouvre pas
#     3   NON MESURÉ — l'instrument n'a pas pu regarder
#
#  Le troisième n'est pas un raffinement. C'est la leçon de
#  `r3-check.sh`, payée le 2 septembre : il écrivait « la mesure est
#  caduque » puis comptait le cas comme une régression, rendant un
#  verdict sur ce qu'il venait de déclarer inconnaissable. La règle,
#  écrite une fois pour toutes : un instrument en panne doit dire
#  qu'il est en panne, et ne jamais emprunter le vocabulaire du sujet
#  qu'il mesure.
#
#  ── USAGE ─────────────────────────────────────────────────────────
#
#     bash scripts/e1-check.sh                → É1 · le coin
#     bash scripts/e1-check.sh jonction       → É2 · le coin ET le registre
#     bash scripts/e1-check.sh --construire   → construit l'image d'abord
#     bash scripts/e1-check.sh --relire       → relit le volume SANS rien exécuter
#
#     SCELLA_PORT=18080          bash scripts/e1-check.sh
#     SCELLA_PORT_JONCTION=18081 bash scripts/e1-check.sh jonction
#
#  Un seul instrument pour les deux jalons, et c'est délibéré : É2
#  n'ajoute pas une porte, il ajoute une PIÈCE derrière la même porte.
#  Deux scripts auraient divergé au premier correctif.
# ─────────────────────────────────────────────────────────────────────

# PAS de `-e`. Ce contrôle doit CONCLURE, pas mourir à la première commande qui
# rend 1 — `grep -c` sans correspondance en rend 1, et un contrôle qui s'arrête
# là ne rend aucun des trois états. Chaque échec est nommé, à sa place.
set -uo pipefail
cd "$(dirname "$0")/.."

PROFIL=""
CONSTRUIRE=0
RELIRE=0
for arg in "$@"; do
  case "$arg" in
    jonction)     PROFIL="jonction" ;;
    --construire) CONSTRUIRE=1 ;;
    --relire)     RELIRE=1 ;;
    -h|--help)    sed -n '2,60p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "argument inconnu : $arg (attendus : jonction, --construire, --relire)" >&2; exit 2 ;;
  esac
done

JALON="É1"; IMAGE="scella-coin:local"; PILOTE="preuve"; PLANCHER=18
if [ -n "$PROFIL" ]; then
  JALON="É1+É2"; IMAGE="scella-jonction:local"; PILOTE="preuve-jonction"; PLANCHER=33
fi
# En relecture, les assertions sur les conteneurs ne sont pas jouées : le plancher
# descend d'autant, et de RIEN d'autre. Un plancher qu'on baisse « pour que ça
# passe » ne garde plus rien ; celui-ci se calcule.
if [ "$RELIRE" = "1" ]; then
  [ -n "$PROFIL" ] && PLANCHER=29 || PLANCHER=16
  JALON="${JALON} (relecture)"
fi

CP=(docker compose)
[ -n "$PROFIL" ] && CP+=(--profile "$PROFIL")

TRAVAIL=$(mktemp -d)
trap 'rm -rf "$TRAVAIL"' EXIT

CONTROLES=0
ECHECS=0
BORNE_CODE=""     # posé par `borne` ; jamais lu sans défaut (`set -u`).

# ── Les quatre voix du contrôle ─────────────────────────────────────────────
exiger()  { CONTROLES=$((CONTROLES+1))
            if [ "$2" = "0" ]; then echo "  ✓ $1"
            else echo "  ✗ $1"; ECHECS=$((ECHECS+1)); fi; }
note()    { echo "    $1"; }
titre()   { echo; echo "═══ $1 ═══"; }
nonmesure() {
  echo
  echo "═══ NON MESURÉ ═══"
  echo "  $1"
  [ $# -gt 1 ] && echo "  → $2"
  echo
  echo "  Ce n'est ni un succès ni un échec de $JALON : l'instrument n'a pas pu"
  echo "  regarder. Il le dit, et s'arrête là — code de sortie 3."
  exit 3
}

# macOS n'a pas `timeout`. Une commande Docker qui pend est le mode de panne le
# plus commun de cette machine (`docker version` a pendu deux fois le 2 sept.),
# et un contrôle qui pend est un contrôle qui ne rend aucun état.
borne() {
  local n="$1"; shift
  local o; o=$(mktemp)
  ( "$@" >"$o" 2>&1; echo "__CODE__:$?" >>"$o" ) & local p=$!
  local i=0
  while [ "$i" -lt "$n" ]; do
    grep -q '__CODE__:' "$o" 2>/dev/null && break
    sleep 1; i=$((i+1))
  done
  kill -9 "$p" 2>/dev/null; wait "$p" 2>/dev/null
  if grep -q '__CODE__:' "$o"; then
    grep -v '__CODE__:' "$o"
    BORNE_CODE=$(grep '__CODE__:' "$o" | tail -1 | cut -d: -f2)
  else
    BORNE_CODE="__PEND__"
  fi
  rm -f "$o"
  [ "$BORNE_CODE" = "0" ]
}

# `ps -a --format json` rend du NDJSON (une ligne par conteneur) sous Compose v5 ;
# d'autres versions rendent un tableau. On accepte les deux plutôt que de parier.
etats() {
  "${CP[@]}" ps -a --format json 2>/dev/null | python3 -c '
import sys, json
brut = sys.stdin.read().strip()
objets = []
if brut:
    try:
        d = json.loads(brut)
        objets = d if isinstance(d, list) else [d]
    except Exception:
        for l in brut.splitlines():
            l = l.strip()
            if l:
                try: objets.append(json.loads(l))
                except Exception: pass
for o in objets:
    print("%s\t%s\t%s" % (o.get("Service",""), o.get("State",""), o.get("ExitCode","")))
'
}
etat_de()  { etats | awk -F'\t' -v s="$1" '$1==s {print $2}' | head -1; }
code_de()  { etats | awk -F'\t' -v s="$1" '$1==s {print $3}' | head -1; }

# ════════════════════════════════════════════════════════════════════════════
titre "0/6 · les préconditions — elles échouent en secondes, ou en quarante minutes"
# ════════════════════════════════════════════════════════════════════════════
command -v docker >/dev/null 2>&1 || nonmesure "aucun \`docker\` sur le PATH."

# ⚠ PAS `VERSION=$(borne …)` : la substitution ouvre un sous-shell, `borne` y
# poserait BORNE_CODE, et la lecture qui suit trouverait une variable non liée.
borne 25 docker version --format '{{.Server.Version}}' > "$TRAVAIL/version.txt" || \
  nonmesure "le démon Docker ne répond pas en 25 s (code : ${BORNE_CODE:-?})." \
            "ouvrir Docker Desktop, puis relancer."
VERSION=$(cat "$TRAVAIL/version.txt" 2>/dev/null)
note "démon        : Docker ${VERSION}"
note "compose      : $(docker compose version --short 2>/dev/null || echo '?')"

if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
  if [ "$CONSTRUIRE" = "1" ]; then
    note "image ${IMAGE} absente — construction (longue : le premier build compile tout)…"
    borne 3600 "${CP[@]}" build || \
      nonmesure "le build a échoué ou pendu (code : ${BORNE_CODE:-?})." \
                "il télécharge rust:1.88-bookworm (~1,5 Go) et les caisses : c'est le CHEMIN DE PULL du démon qui décide, pas le réseau de l'hôte."
  else
    nonmesure "l'image ${IMAGE} n'existe pas." \
              "ce contrôle MESURE, il ne construit pas : \`bash scripts/e1-check.sh ${PROFIL} --construire\`"
  fi
fi
note "image        : ${IMAGE} — présente"

# Le port. C'est la précondition qui est réellement tombée le 4 septembre : 8080
# était tenu par une autre publication Docker, et `up` s'est arrêté à la mise en
# réseau — avant qu'un seul conteneur ne démarre.
port_pris() { lsof -nP -iTCP:"$1" -sTCP:LISTEN >/dev/null 2>&1; }
if [ "$RELIRE" = "1" ]; then
  # Une relecture ne PUBLIE rien : exiger un port libre pour lire des fichiers
  # déjà écrits, c'est refuser de mesurer sur une condition qui ne s'applique
  # pas. L'image, elle, reste exigée — les deux vérifieurs vivent dedans.
  note "ports        : non exigés — la relecture ne lie aucun port"
else
P_COIN="${SCELLA_PORT:-8080}"
if port_pris "$P_COIN"; then
  nonmesure "le port hôte ${P_COIN} est déjà pris." \
            "SCELLA_PORT=18080 bash scripts/e1-check.sh ${PROFIL}"
fi
note "port hôte    : ${P_COIN} — libre"
if [ -n "$PROFIL" ]; then
  P_JONC="${SCELLA_PORT_JONCTION:-8081}"
  if port_pris "$P_JONC"; then
    nonmesure "le port hôte ${P_JONC} (jonction) est déjà pris." \
              "SCELLA_PORT_JONCTION=18081 bash scripts/e1-check.sh jonction"
  fi
  note "port jonction: ${P_JONC} — libre"
fi
fi

if [ "$RELIRE" = "1" ]; then
titre "1-3/6 · SAUTÉS — relecture du volume TEL QU'IL EST"
# `--relire` existe pour deux raisons, et la seconde vaut d'être écrite.
#
#   · Après une démonstration qui a mal tourné, on veut REGARDER. Or le geste 1
#     de ce contrôle commence par `down -v` : relancer, c'est effacer la preuve
#     qu'on venait chercher.
#
#   · Et c'est ce qui rend cet instrument RÉFUTABLE. Un contrôle qu'on ne peut
#     faire tomber ne garde rien : sans ce mode, la seule façon de vérifier que
#     ses assertions savent dire « non » aurait été de casser le produit et de
#     reconstruire l'image — quarante minutes pour éprouver un `if`. Ici il
#     suffit de pousser un fichier abîmé dans le volume et de relire.
note "aucun \`down -v\`, aucun \`up\` : ce qui suit décrit l'état laissé par la dernière exécution."
note "les codes de sortie des conteneurs ne sont donc PAS relus — ils appartiennent à cette exécution-là."
else
# ════════════════════════════════════════════════════════════════════════════
titre "1/6 · repartir d'un volume VIDE"
# ════════════════════════════════════════════════════════════════════════════
# Le volume nommé porte les artefacts des exécutions précédentes. Un contrôle qui
# vérifierait leur PRÉSENCE lirait un succès sans qu'une ligne neuve ait été
# écrite — il mesurerait le passé. Une démonstration non idempotente finit par
# prouver l'avant-dernière.
# ⚠ Le NETTOYAGE doit être TOTAL là où la mesure est SCOPÉE, et ce contrôle a
# appris la différence en tombant dessus. `down -v` sous le seul profil courant
# laisse debout les conteneurs des AUTRES profils — qui référencent le même
# volume, donc l'empêchent d'être supprimé. Enchaîner une mesure du coin après
# une mesure de la jonction rendait alors « non mesuré », avec un diagnostic qui
# accusait « un conteneur hors projet » : faux, ils étaient DANS le projet, sous
# un autre profil. On démonte donc tous les profils, toujours.
borne 180 docker compose --profile jonction --profile production down -v --remove-orphans >/dev/null 2>&1
if docker volume inspect scella_preuve >/dev/null 2>&1; then
  nonmesure "le volume scella_preuve survit à \`down -v\` sur TOUS les profils." \
            "voir qui le retient : \`docker ps -a --filter volume=scella_preuve\`"
fi
note "scella_preuve : supprimé — ce qui suit sera écrit à l'instant, ou pas du tout"

# ════════════════════════════════════════════════════════════════════════════
titre "2/6 · le geste — et son code de retour, NOTÉ sans être cru"
# ════════════════════════════════════════════════════════════════════════════
# Le critère dit `docker compose up`. Ce contrôle lance `up -d` puis ATTEND les
# pilotes, et ce choix a une raison mesurable : sous le profil `jonction` il y a
# DEUX pilotes, et `--abort-on-container-exit` tue le plus lent dès que le
# premier sort. Le verdict dépendrait alors de l'ordonnancement — c'est-à-dire
# du hasard. Rien d'autre ne change : mêmes images, même réseau, même volume,
# mêmes variables.
borne 900 "${CP[@]}" up -d >/dev/null 2>&1
CODE_UP="$BORNE_CODE"
note "\`up -d\` a rendu : ${CODE_UP}   ← noté, PAS cru (voir l'en-tête de ce fichier)"

# Le cas le plus fréquent d'un premier essai — port pris, image absente, réseau —
# se lit à l'instant : les conteneurs sont là, en « created », et rien ne les fera
# bouger. Attendre 300 s par pilote pour le découvrir, c'est payer cinq minutes le
# fait qu'on tient déjà.
VIVANTS=$(etats | awk -F'\t' '$2!="created" && $2!="" {n++} END {print n+0}')
if [ "$VIVANTS" = "0" ]; then
  nonmesure "aucun conteneur n'a démarré — tous en « created »." \
            "l'échec précède les conteneurs : réseau, volume, image ou port. \`up\` a rendu ${CODE_UP} ; ce chiffre ne dit RIEN, c'est l'état qui parle."
fi

attendre_sortie() {
  local svc="$1" max="$2" i=0 e
  while [ "$i" -lt "$max" ]; do
    e=$(etat_de "$svc")
    [ "$e" = "exited" ] && return 0
    [ -z "$e" ] && return 2      # le conteneur n'existe pas : rien à attendre
    sleep 3; i=$((i+3))
  done
  return 1
}

PILOTES="preuve"
[ -n "$PROFIL" ] && PILOTES="preuve preuve-jonction"
for pl in $PILOTES; do
  attendre_sortie "$pl" 300
  case "$?" in
    0) note "pilote ${pl} : sorti" ;;
    2) nonmesure "le conteneur du pilote ${pl} n'existe pas." \
                 "\`up\` s'est arrêté avant de le créer — regarder \`docker compose logs\`." ;;
    *) nonmesure "le pilote ${pl} n'est pas sorti en 300 s." \
                 "il attend la passerelle jusqu'à 60 s ; au-delà, c'est la passerelle qu'il faut regarder." ;;
  esac
done

# Les passerelles tournent encore : on les ARRÊTE pour observer leur code. C'est
# plus exigeant que le critère, qui se contentait de « les deux ont rendu 0 » :
# ici l'arrêt gracieux est lui aussi mesuré.
borne 120 "${CP[@]}" stop >/dev/null 2>&1

# ════════════════════════════════════════════════════════════════════════════
titre "3/6 · l'observation — l'ÉTAT d'abord, le code ensuite"
# ════════════════════════════════════════════════════════════════════════════
SERVICES="passerelle preuve"
[ -n "$PROFIL" ] && SERVICES="passerelle preuve jonction preuve-jonction"

DEMARRES=0
for svc in $SERVICES; do
  E=$(etat_de "$svc"); C=$(code_de "$svc")
  [ -z "$E" ] && E="(aucun conteneur)"
  case "$E" in
    exited) DEMARRES=$((DEMARRES+1)) ;;
    created|"(aucun conteneur)") : ;;
    *)      DEMARRES=$((DEMARRES+1)) ;;
  esac
  printf '    %-16s état=%-10s code=%s\n' "$svc" "$E" "${C:-—}"
done

# Le troisième état, à sa place la plus utile : si RIEN n'a démarré, tout ce qui
# suit lirait des fichiers absents et conclurait « échec » sur une exécution qui
# n'a pas eu lieu.
[ "$DEMARRES" = "0" ] && \
  nonmesure "aucun conteneur n'a démarré (tous en « created » ou absents)." \
            "l'échec précède les conteneurs : réseau, volume, image ou port. \`docker compose logs\` ne dira rien — c'est \`up\` qu'il faut relire."

for svc in $SERVICES; do
  E=$(etat_de "$svc"); C=$(code_de "$svc")
  if [ "$E" != "exited" ]; then
    exiger "${svc} : sorti proprement (état lu : ${E:-absent})" 1
    note "l'état n'est pas « exited » : son ExitCode ne veut RIEN dire et n'est pas lu."
  elif [ "$C" = "0" ]; then
    exiger "${svc} : exited, code 0 — observé, non déduit" 0
  else
    exiger "${svc} : exited, code ${C}" 1
  fi
done

fi   # fin des gestes 1-3

# ════════════════════════════════════════════════════════════════════════════
titre "4/6 · les artefacts, sortis du volume et RECOMPTÉS à la main"
# ════════════════════════════════════════════════════════════════════════════
# `docker compose cp` est exécuté par le DÉMON : il n'a besoin d'aucun shell dans
# l'image, et il marche sur un conteneur arrêté. C'est la seule façon de lire le
# volume d'une image distroless — et c'est pourquoi le geste que `compose.yaml`
# documentait (`--entrypoint ls`) ne pouvait pas fonctionner.
borne 120 "${CP[@]}" cp preuve:/preuve "$TRAVAIL/vol" >/dev/null 2>&1 || \
  nonmesure "\`compose cp\` n'a pas pu sortir /preuve du conteneur (code : ${BORNE_CODE:-?})."
VOL="$TRAVAIL/vol"
note "volume copié : $(find "$VOL" -type f 2>/dev/null | wc -l | tr -d ' ') fichier(s)"

# Le comptage, refait avec les outils d'un auditeur. Si le pilote et ce script
# divergeaient, c'est le pilote qui mentirait — c'est tout l'intérêt de recompter.
lignes() { local n; n=$(grep -c '[^[:space:]]' "$1" 2>/dev/null); echo "${n:-0}"; }

mesurer_bundle() {   # $1 = dossier de sortie d'un pilote, $2 = son nom
  local d="$1" nom="$2" intact tronque run annonce racine dans_i dans_t
  intact="$d/bundle.sealed.jsonl"
  tronque="$d/bundle.TRONQUE.jsonl"

  [ -s "$intact" ];  exiger "[$nom] bundle.sealed.jsonl existe et n'est pas vide" $?
  [ -s "$tronque" ]; exiger "[$nom] bundle.TRONQUE.jsonl existe et n'est pas vide" $?
  [ -s "$intact" ] && [ -s "$tronque" ] || return

  ! cmp -s "$intact" "$tronque"; exiger "[$nom] la copie tronquée DIFFÈRE de l'intacte" $?

  local ni nt
  ni=$(lignes "$intact"); nt=$(lignes "$tronque")
  [ "$nt" -lt "$ni" ]; exiger "[$nom] le tronqué est strictement plus court ($nt < $ni)" $?

  # Le run se DÉDUIT du bundle : `node_id` = "<run>::<seq>-<rôle>". L'ancre est
  # ensuite choisie par ce champ, jamais par position — sous le profil jonction,
  # deux passerelles publient dans le même journal et « la dernière » est celle
  # du hasard. C'est le défaut que É2 a corrigé dans le pilote ; ce contrôle
  # aurait le même s'il lisait la dernière ligne.
  run=$(head -1 "$intact" | python3 -c 'import sys,json; print(json.load(sys.stdin).get("node_id","").split("::")[0])' 2>/dev/null)
  [ -n "$run" ]; exiger "[$nom] le run se déduit du bundle lui-même (${run:-illisible})" $?
  [ -n "$run" ] || return

  local anc="$VOL/anchors.log"
  [ -s "$anc" ]; exiger "[$nom] le journal d'ancres existe HORS du bundle" $?
  [ -s "$anc" ] || return

  annonce=$(awk -F'\t' -v r="$run" '$2==r {print $5}' "$anc" | head -1)
  racine=$(awk  -F'\t' -v r="$run" '$2==r {print $4}' "$anc" | head -1)
  [ -n "$annonce" ]; exiger "[$nom] l'ancre de CE run est trouvée par son identifiant" $?
  [ -n "$annonce" ] || return

  [ "$ni" = "$annonce" ]; exiger "[$nom] l'intact ($ni) CONCORDE avec l'ancre ($annonce)" $?
  [ "$nt" != "$annonce" ]; exiger "[$nom] le tronqué ($nt) CONTREDIT l'ancre ($annonce) — sans clé" $?

  # ── Le fait qui explique tout le reste, et que l'architecture n'avait pas vu.
  # La racine annoncée par l'ancre est le SCEAU DU DERNIER MAILLON : elle est
  # dans le bundle intact, et elle DISPARAÎT avec la troncature. Le témoin part
  # avec le crime. C'est pour cela qu'un bundle tronqué « se re-prouve
  # parfaitement » — il est un préfixe de chaîne valide, cohérent avec SA propre
  # racine — et pour cela que seule une racine reçue par un canal DISTINCT peut
  # le contredire.
  dans_i=$(grep -c "$racine" "$intact"  2>/dev/null); dans_i="${dans_i:-0}"
  dans_t=$(grep -c "$racine" "$tronque" 2>/dev/null); dans_t="${dans_t:-0}"
  [ "$dans_i" -ge 1 ]; exiger "[$nom] la racine de l'ancre EST dans le bundle intact (le sceau du dernier maillon)" $?
  [ "$dans_t" = "0" ]; exiger "[$nom] et elle DISPARAÎT du tronqué — le bundle seul ne peut pas témoigner de sa complétude" $?
}

mesurer_bundle "$VOL" "coin"
[ -n "$PROFIL" ] && mesurer_bundle "$VOL/jonction" "jonction"

# ── É5 · LA PAGE QU'ON TEND enseigne-t-elle encore des gestes qui MARCHENT ? ──────
#
# La page (`docs/reports/Scella_Le_Seuil_En_Une_Commande_*.tex`) est ce qu'un inconnu
# lit AVANT de lancer la commande, et elle lui apprend à recompter lui-même. Ses
# chiffres sont datés — l'en-tête le dit — mais ses GESTES doivent rester vrais : un
# nom de fichier qui change, un champ d'ancre qui se déplace, et la page enseigne une
# manipulation qui échoue chez le lecteur, au pire moment.
#
# Ce garde ne relit pas la prose. Il vérifie exactement trois choses, celles qu'un
# lecteur va TAPER : les noms des fichiers que le pilote écrit, et la forme de la
# sélection d'ancre. Il refuse en particulier `END{print $5}` — « la dernière ligne » —
# qui a été enseignée à trois endroits du dépôt avant d'être réfutée par É2.
PAGE=$(ls docs/reports/Scella_Le_Seuil_En_Une_Commande_*.tex 2>/dev/null | head -1)
[ -n "$PAGE" ] && [ -s "$PAGE" ]
exiger "[page] la page qu'on tend existe (${PAGE:-absente})" $?
if [ -n "$PAGE" ] && [ -s "$PAGE" ]; then
  MANQUE=""
  for f in anchors.log bundle.sealed.jsonl bundle.TRONQUE.jsonl; do
    grep -qF "$f" "$PAGE" || MANQUE="$MANQUE $f"
  done
  [ -z "$MANQUE" ]
  exiger "[page] elle nomme les trois fichiers que le pilote écrit${MANQUE:+ — manquant :$MANQUE}" $?

  # La sélection par le run, et l'absence de la forme réfutée.
  grep -qF '$2==r' "$PAGE" && ! grep -qE "END *\\?\{print" "$PAGE"
  exiger "[page] elle enseigne la sélection PAR LE RUN, pas « la dernière ligne »" $?
fi

# ── É2 · le dossier réglementaire, recompté lui aussi ───────────────────────
if [ -n "$PROFIL" ]; then
  RUNJ=$(head -1 "$VOL/jonction/bundle.sealed.jsonl" 2>/dev/null | \
         python3 -c 'import sys,json; print(json.load(sys.stdin).get("node_id","").split("::")[0])' 2>/dev/null)
  DOSSIER="$VOL/reg/$RUNJ"
  [ -d "$DOSSIER" ]; exiger "[C1] le dossier existe et porte le NOM du run ($RUNJ)" $?
  if [ -d "$DOSSIER" ]; then
    NF=$(find "$DOSSIER" -type f | wc -l | tr -d ' ')
    [ "$NF" -ge 3 ]; exiger "[C1] il contient $NF fichier(s), comptés EN PROFONDEUR" $?
    # Par RÔLE, jamais par nom exact : épingler `mapping.2026-06.json` ferait
    # échouer ce contrôle au prochain millésime, et un contrôle qui tombe quand le
    # produit évolue correctement ne garde rien.
    find "$DOSSIER" -type f -name '*.md'   | grep -qv REDERIVE; exiger "[C1] la VUE réglementaire (un .md de cadre)" $?
    find "$DOSSIER" -type f -name 'mapping.*.json' | grep -q .;  exiger "[C1] le mapping, VERSIONNÉ dans son nom" $?
    find "$DOSSIER" -type f -name 'REDERIVE*'      | grep -q .;  exiger "[C1] le mode d'emploi de re-dérivation" $?
  fi
fi

# ════════════════════════════════════════════════════════════════════════════
titre "5/6 · les deux vérifieurs — DANS l'image, pas sur la machine de l'auteur"
# ════════════════════════════════════════════════════════════════════════════
# ⚠ CE QUE L'ARCHITECTURE DEMANDAIT ICI EST INEXÉCUTABLE, et c'est le contrôle
# qui l'établit. Son point 3 disait : « verify-mini re-prouve l'intacte et
# CONTREDIT la tronquée, sans clé ». Deux choses fausses, l'une et l'autre
# vérifiées par le code lui-même :
#
#   · sans clé, `verify-mini` ne rend aucun verdict — il sort en 2 (usage) ;
#   · AVEC la clé, il ne contredirait toujours pas la tronquée : il la
#     VALIDERAIT. `provenance-verify` le dit dans sa propre aide — « un préfixe
#     de chaîne valide reste valide ».
#
# Ce qui contredit la troncature sans clé, c'est le COMPTAGE (étape 4). Ce qui
# la contredit avec la clé, c'est `provenance-verify --anchor --run` : le second
# vérifieur, celui que É1 a ajouté à l'image. Ce contrôle mesure donc les deux
# propriétés qui sont vraies et exécutables : ils sont LÀ, et ils ne rendent
# jamais un verdict quand on ne leur a pas donné de quoi conclure.
# Le code de sortie est RENDU, jamais posé dans une variable : appelée en
# substitution, cette fonction s'exécuterait dans un sous-shell et la variable
# n'en sortirait pas. L'aide part dans un fichier, le code dans `$?`.
usage_de() {   # $1 = binaire, $2 = fichier où déposer l'aide
  "${CP[@]}" run --rm --no-deps --entrypoint "/usr/local/bin/$1" "$PILOTE" >"$2" 2>&1
}

usage_de verify-mini       "$TRAVAIL/u_vm.txt"; C_VM=$?
usage_de provenance-verify "$TRAVAIL/u_pv.txt"; C_PV=$?

[ "$C_VM" = "2" ]; exiger "verify-mini est dans l'image et sort en 2 (usage), jamais en 0 ni en 1" $?
[ "$C_PV" = "2" ]; exiger "provenance-verify est dans l'image et sort en 2 (usage)" $?
note "2 ≠ 1 : « je n'ai pas de quoi conclure » n'emprunte pas le mot de « c'est altéré »."

grep -q -- '--anchor' "$TRAVAIL/u_pv.txt"; exiger "provenance-verify SAIT lire une ancre (--anchor dans son usage) — P.4" $?
! grep -q -- '--anchor' "$TRAVAIL/u_vm.txt"
exiger "verify-mini ne le sait PAS : les deux ne sont pas interchangeables" $?
note "sans ce contrôle, retirer le COPY de provenance-verify laisserait tous les tests verts."

# ════════════════════════════════════════════════════════════════════════════
titre "6/6 · le verdict"
# ════════════════════════════════════════════════════════════════════════════
# Plancher anti-vacuité : un contrôle qui n'exécute plus ses assertions rend 0
# en silence. Le compte est donc lui-même une assertion — et son échec est un
# NON MESURÉ, pas un succès.
if [ "$CONTROLES" -lt "$PLANCHER" ]; then
  nonmesure "seulement ${CONTROLES} contrôle(s) exécutés, ${PLANCHER} attendus au minimum." \
            "une branche a été sautée : le verdict porterait sur moins que le critère."
fi

echo "  ${CONTROLES} contrôle(s) · ${ECHECS} échec(s) · plancher ${PLANCHER}"
echo
if [ "$ECHECS" = "0" ]; then
  echo "═══ ${JALON} : LA PORTE S'OUVRE ═══"
  echo "  · un reçu servi, exporté, ancré hors bande, et une troncature démasquée"
  echo "    par un COMPTAGE que ce script a refait lui-même sur les fichiers du volume ;"
  # ⚠ Cette ligne ne doit PAS être imprimée en relecture : les gestes 1-3 y sont
  # sautés, donc aucun état de conteneur n'a été lu. Un verdict qui revendique
  # une observation qu'il n'a pas faite est exactement la faute que cet
  # instrument existe pour empêcher — et je l'ai écrite ici avant de la voir.
  if [ "$RELIRE" = "0" ]; then
    echo "  · les codes de sortie des conteneurs ont été OBSERVÉS après lecture de"
    echo "    leur état — jamais déduits d'un entier rendu par l'orchestrateur ;"
  else
    echo "  · MAIS les conteneurs n'ont pas été observés : cette relecture porte sur"
    echo "    les fichiers laissés par la dernière exécution, pas sur une exécution."
  fi
  [ -n "$PROFIL" ] && echo "  · et le Registre a dérivé un dossier réglementaire DU MÊME run."
  echo
  echo "  CE QUE CECI NE PROUVE PAS :"
  echo "  · la substance — la démonstration tourne sur la clé de TEST PUBLIQUE ;"
  echo "    elle montre le mécanisme, jamais l'opposabilité ;"
  echo "  · l'absence d'egress au BUILD — la promesse porte sur l'exécution ;"
  echo "  · « machine vierge » — ici la machine porte déjà un état Docker."
  exit 0
else
  echo "═══ ${JALON} : LA PORTE NE S'OUVRE PAS — ${ECHECS} échec(s) ═══"
  # Même règle que du côté vert : ne revendiquer que ce qui a été regardé. En
  # relecture, l'affirmation « les conteneurs ont démarré » n'a pas été vérifiée.
  if [ "$RELIRE" = "0" ]; then
    echo "  Mesuré, et négatif. Les conteneurs ont démarré ; ce qui est en cause"
    echo "  est le produit, pas l'instrument."
  else
    echo "  Mesuré sur les FICHIERS, et négatif. Cette relecture ne dit pas si les"
    echo "  conteneurs ont démarré — relancer sans \`--relire\` pour le savoir."
  fi
  exit 1
fi
