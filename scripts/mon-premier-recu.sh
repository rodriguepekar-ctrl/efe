#!/usr/bin/env bash
# SPDX-License-Identifier: UNLICENSED
# ─────────────────────────────────────────────────────────────────────────────
#  VOTRE PREMIER REÇU OPPOSABLE — en une commande.
#
#  ## Pourquoi ce script existe
#
#  `docker compose up` donne un reçu qui dit, DE LUI-MÊME : « il ne prouve rien
#  contre personne ». C'est exact, et c'est honnête : la démonstration tourne sur
#  la clé de TEST PUBLIQUE, connue de quiconque a lu le dépôt. Elle montre le
#  MÉCANISME. Elle ne peut rien opposer à qui que ce soit.
#
#  L'AUTRE reçu — celui qui oppose quelque chose — existait déjà : le profil
#  `production` le produit, et `scripts/p1-check.sh` le prouve depuis toujours.
#  Mais pour l'obtenir, il fallait savoir six choses qu'aucun document ouvert par
#  un nouvel arrivant ne disait : fabriquer un master, fabriquer une paire OIDC,
#  connaître `p1-jeton.sh`, poser une allowlist, appeler `/v1/swarm/bundle/:id`,
#  et rejouer `verify-mini` avec la clé DÉRIVÉE — pas le master.
#
#  Tout était construit. Rien n'était assemblé. Ce script est l'assemblage.
#
#  ## Ce qu'il fait, et ce qu'il ne fait pas
#
#  Il fabrique VOS clés (elles ne quittent jamais cette machine), démarre le
#  profil de production, sert un run, vous remet le bundle et l'ancre, dérive la
#  clé qui les re-prouve — puis **la re-prouve immédiatement devant vous**. Ce
#  dépôt n'affirme rien qu'il n'ait mesuré ; ce script ne fait pas exception.
#
#  Il ne configure PAS de fournisseur frontière (cela demande une clé qui coûte
#  de l'argent) et n'installe rien hors de Docker.
#
#  ## Les trois états
#     0  votre reçu existe, et il a été re-prouvé sous VOTRE clé
#     1  quelque chose a échoué, et le script dit quoi
#     3  NON MESURÉ — l'instrument n'a pas pu regarder (Docker, un port, un outil)
#
#  Usage :  bash scripts/mon-premier-recu.sh [dossier]        (défaut : ./mon-premier-recu)
# ─────────────────────────────────────────────────────────────────────────────
set -uo pipefail
cd "$(dirname "$0")/.."

ok()    { printf '  \033[32m✓\033[0m %s\n' "$*"; }
ko()    { printf '  \033[31m✗\033[0m %s\n' "$*"; exit 1; }
note()  { printf '      %s\n' "$*"; }
titre() { printf '\n\033[1;34m%s\033[0m\n' "$*"; }
nonmesure() {
  printf '\n\033[1mNON MESURÉ\033[0m\n  %s\n' "$1"
  [ $# -gt 1 ] && printf '  → %s\n' "$2"
  printf '\n  Ni un succès ni un échec : l'\''instrument n'\''a pas pu regarder.\n\n'
  exit 3
}

DOSSIER="${1:-mon-premier-recu}"
PORT="${SCELLA_PORT_PROD:-8082}"
IMAGE="scella-production:local"

printf '\033[1mVotre premier reçu opposable\033[0m\n'

# ═════════════════════════════════════════════════════════════════════════════
titre "1. De quoi ce script a besoin"
# ═════════════════════════════════════════════════════════════════════════════
for outil in docker openssl curl python3; do
  command -v "$outil" >/dev/null 2>&1 || nonmesure "« $outil » est introuvable." \
    "Ce script n'installe rien : il vous dit ce qui manque plutôt que de le deviner."
done
docker info >/dev/null 2>&1 || nonmesure "le démon Docker ne répond pas." \
  "Démarrez Docker Desktop, puis relancez cette commande."
if lsof -nP -iTCP:"$PORT" -sTCP:LISTEN >/dev/null 2>&1; then
  nonmesure "le port $PORT est déjà pris sur cette machine." \
    "SCELLA_PORT_PROD=18082 bash scripts/mon-premier-recu.sh"
fi
ok "Docker répond, le port $PORT est libre, les outils sont là"

# ═════════════════════════════════════════════════════════════════════════════
titre "2. VOS clés — fabriquées ici, elles ne partent nulle part"
# ═════════════════════════════════════════════════════════════════════════════
#
#  ⚠ ON NE RÉGÉNÈRE JAMAIS. Un master qui change rend INVÉRIFIABLES tous les
#  reçus déjà scellés sous l'ancien : ils ne deviennent pas faux, ils deviennent
#  orphelins, et plus personne ne peut le démontrer. Un dossier qui existe est
#  donc RÉUTILISÉ, et le script le dit.
mkdir -p "$DOSSIER" || ko "impossible de créer « $DOSSIER »"
MAITRE="$DOSSIER/cle-maitre.hex"
OIDC_PRIV="$DOSSIER/oidc-prive.pem"
OIDC_PUB="$DOSSIER/oidc-public.pem"

if [ -f "$MAITRE" ]; then
  ok "clés RÉUTILISÉES depuis « $DOSSIER » — en fabriquer d'autres orphelinerait vos reçus"
else
  openssl rand -hex 32 > "$MAITRE" || ko "openssl n'a pas su fabriquer un master"
  openssl ecparam -name prime256v1 -genkey -noout -out "$OIDC_PRIV" 2>/dev/null \
    || ko "openssl n'a pas su fabriquer une paire OIDC"
  openssl ec -in "$OIDC_PRIV" -pubout -out "$OIDC_PUB" 2>/dev/null
  chmod 600 "$MAITRE" "$OIDC_PRIV"
  ok "clés fabriquées dans « $DOSSIER » (0600) — elles n'ont jamais quitté cette machine"
fi
MASTER="$(cat "$MAITRE")"
PEM="$(cat "$OIDC_PUB")"
note "master : $DOSSIER/cle-maitre.hex   ·   OIDC : $DOSSIER/oidc-{prive,public}.pem"

# ═════════════════════════════════════════════════════════════════════════════
titre "3. Le profil de PRODUCTION démarre — pas la démonstration"
# ═════════════════════════════════════════════════════════════════════════════
#
#  Quatre features (`postgres,oidc,pii,frontier-llm`) et un vrai Postgres. Sans
#  elles, le binaire sort en 78 plutôt que de dégrader en silence : la frontière
#  est un egress PAYANT vers un TIERS, et il n'en existe pas de version « à
#  moitié ». La première construction prend plusieurs minutes.
export EFE_NANO_AUDIT_HMAC_MASTER="$MASTER"
export EFE_OIDC_PUBLIC_KEY_PEM="$PEM"
export SCELLA_PORT_PROD="$PORT"
#  ⚠ On NOMME les deux services, on n'emploie PAS `--profile production up`.
#  `--profile X up` démarre les services du profil X **ET tous ceux qui n'ont pas
#  de profil** — donc la passerelle de DÉMONSTRATION, qui réclame le port 8080.
#  Sur une machine où 8080 est pris, tout échouait, et le message parlait du
#  profil de production alors que le coupable était la démo. Mesuré ici même, à
#  la première exécution de ce script. Nommer les services démarre exactement
#  ceux-là.
docker compose up -d --build base production >"$DOSSIER/demarrage.log" 2>&1 \
  || { tail -20 "$DOSSIER/demarrage.log"; ko "le profil de production n'a pas démarré — voir $DOSSIER/demarrage.log"; }

for _ in $(seq 1 90); do
  curl -sf "http://127.0.0.1:$PORT/healthz" >/dev/null 2>&1 && break
  sleep 1
done
curl -sf "http://127.0.0.1:$PORT/healthz" >/dev/null 2>&1 \
  || { docker compose logs --tail 30 production 2>&1 | tail -20
       ko "la passerelle n'a pas répondu en 90 s"; }
ok "la passerelle sert sur 127.0.0.1:$PORT, sous VOTRE master"

# ═════════════════════════════════════════════════════════════════════════════
titre "4. Un run, sous un jeton que vous seul pouvez forger"
# ═════════════════════════════════════════════════════════════════════════════
CODE=$(curl -s -o /dev/null -w '%{http_code}' -X POST "http://127.0.0.1:$PORT/v1/swarm/run" \
  -H 'content-type: application/json' -d @examples/run.json)
[ "$CODE" = "401" ] || ko "sans jeton, attendu 401, obtenu $CODE — l'authentification ne garde rien"
ok "sans jeton : 401 — la porte est fermée avant même de commencer"

JETON="$(bash scripts/p1-jeton.sh "$OIDC_PRIV" 2>/dev/null)" \
  || ko "impossible de forger un jeton depuis votre clé OIDC"
curl -s -o "$DOSSIER/run.json" -w '%{http_code}' -X POST "http://127.0.0.1:$PORT/v1/swarm/run" \
  -H "authorization: Bearer $JETON" -H 'content-type: application/json' \
  -d @examples/run.json > "$DOSSIER/code.txt"
[ "$(cat "$DOSSIER/code.txt")" = "200" ] \
  || { cat "$DOSSIER/run.json"; ko "run refusé : HTTP $(cat "$DOSSIER/code.txt")"; }
RUN="$(python3 -c "import json;print(json.load(open('$DOSSIER/run.json'))['run_id'])")"
ok "run servi : $RUN"

# ═════════════════════════════════════════════════════════════════════════════
titre "5. Le reçu PORTABLE — le bundle, et l'ancre publiée hors de lui"
# ═════════════════════════════════════════════════════════════════════════════
#
#  L'ancre est le seul témoin d'une TRONCATURE : le préfixe d'une chaîne scellée
#  est lui-même une chaîne scellée valide, et se re-prouve parfaitement. Rien,
#  de l'intérieur, ne trahit ce qui a été coupé. L'ancre annonce le nombre de
#  nœuds, et elle est publiée HORS du bundle — c'est ce qui la rend utile.
curl -s -o "$DOSSIER/bundle.json" -w '%{http_code}' -H "authorization: Bearer $JETON" \
  "http://127.0.0.1:$PORT/v1/swarm/bundle/$RUN" > "$DOSSIER/code-bundle.txt"
[ "$(cat "$DOSSIER/code-bundle.txt")" = "200" ] \
  || { cat "$DOSSIER/bundle.json"; ko "bundle refusé : HTTP $(cat "$DOSSIER/code-bundle.txt")"; }

python3 - "$DOSSIER" <<'PY' || ko "le bundle rendu n'a pas la forme attendue"
import json, sys, pathlib
d = pathlib.Path(sys.argv[1])
b = json.loads((d / "bundle.json").read_text())
(d / "bundle.jsonl").write_text(b["bundle_jsonl"])
(d / "racine.txt").write_text(b["manifest_root"] + "\n")
print(f"      {b['node_count']} nœud(s) · racine {b['manifest_root'][:16]}… · ancré : {b['anchored']}")
PY
RACINE="$(tr -d '\n' < "$DOSSIER/racine.txt")"
ok "bundle écrit : $DOSSIER/bundle.jsonl"

#  L'ancre vit dans le volume, et l'image est distroless (aucun shell) : `cp` est
#  le seul moyen de la lire — c'est le démon qui l'exécute, pas un shell absent.
docker compose cp production:/var/lib/scella/anchors.log \
  "$DOSSIER/ancres.log" >/dev/null 2>&1 \
  && ok "ancre extraite : $DOSSIER/ancres.log ($(wc -l < "$DOSSIER/ancres.log" | tr -d ' ') ligne(s))" \
  || note "ancre non extraite (le volume ne la porte pas encore) — le bundle reste vérifiable"

# ═════════════════════════════════════════════════════════════════════════════
titre "6. La clé qui re-prouve — dérivée de VOTRE master, jamais transmise"
# ═════════════════════════════════════════════════════════════════════════════
#
#  `verify-mini` n'attend pas le master : il attend la clé TENANT, que le
#  scellement dérive du master par HKDF. Le drapeau `--cle-tenant` appelle LA
#  MÊME fonction que le scellement — refaire la formule dans ce script en aurait
#  fait un second analyseur, et ce dépôt a déjà payé pour cette erreur-là.
#  ⚠ DEUX clés, et ce n'est pas une subtilité : le RUN est scellé sous la clé de la
#  pile A, le BUNDLE exporté sous celle de la pile B, dérivée de la première. Les
#  confondre fait échouer la re-preuve « au record 0 » — un message qui accuse la
#  chaîne alors que le fautif est la clé. Mesuré ici même, à la première exécution.
docker run --rm -e EFE_NANO_AUDIT_HMAC_MASTER="$MASTER" \
  --entrypoint /usr/local/bin/efe-swarm-gateway "$IMAGE" --cle-tenant --bundle \
  > "$DOSSIER/cle-bundle.hex" 2>/dev/null \
  || ko "la dérivation de la clé de bundle a échoué"
chmod 600 "$DOSSIER/cle-bundle.hex"
[ "$(wc -c < "$DOSSIER/cle-bundle.hex" | tr -d ' ')" -ge 64 ] \
  || ko "la clé dérivée n'a pas 64 caractères hexadécimaux"
ok "clé du bundle dérivée : $DOSSIER/cle-bundle.hex (0600) — un SECRET, au même titre que le master"

# ═════════════════════════════════════════════════════════════════════════════
titre "7. ET ON LE RE-PROUVE, ICI, MAINTENANT"
# ═════════════════════════════════════════════════════════════════════════════
#
#  Ce dépôt n'affirme rien qu'il n'ait mesuré. Vous dire « voici votre reçu » sans
#  l'avoir re-prouvé devant vous serait exactement le vert malhonnête que tous ses
#  contrôles existent pour interdire.
ABS="$(cd "$DOSSIER" && pwd)"
docker run --rm -v "$ABS:/p" --entrypoint /usr/local/bin/verify-mini "$IMAGE" \
  /p/bundle.jsonl --key-file /p/cle-bundle.hex --root "$RACINE" > "$DOSSIER/verdict.txt" 2>&1
VERDICT=$?
if [ "$VERDICT" != "0" ]; then
  cat "$DOSSIER/verdict.txt"
  ko "le reçu ne se re-prouve PAS — ne le montrez à personne tant que ceci n'est pas compris"
fi
ok "RE-PROUVÉ sous votre clé : $(head -1 "$DOSSIER/verdict.txt")"

# ═════════════════════════════════════════════════════════════════════════════
titre "Ce que vous avez, et ce que cela oppose"
# ═════════════════════════════════════════════════════════════════════════════
cat <<FIN

  Dans « $DOSSIER » :

    bundle.jsonl      le reçu portable — la chaîne scellée du run
    racine.txt        la racine du manifeste, engagée par l'ancre
    ancres.log        l'ancre, publiée HORS du bundle (le témoin de troncature)
    cle-bundle.hex    la clé qui le re-prouve            ⚠ SECRET, 0600
    cle-maitre.hex    votre master, d'où elle dérive     ⚠ SECRET, 0600
    oidc-prive.pem    la clé qui forge vos jetons        ⚠ SECRET, 0600

  LA COMMANDE QUE VOUS SEUL POUVEZ EXÉCUTER — elle ne cite jamais la clé, elle
  cite son FICHIER : un secret collé dans un terminal finit dans un historique.

    docker run --rm -v "$ABS:/p" \\
      --entrypoint /usr/local/bin/verify-mini $IMAGE \\
      /p/bundle.jsonl --key-file /p/cle-bundle.hex --root $RACINE

  CE QUE CE REÇU OPPOSE. Il est scellé sous une clé que **vous seul** détenez :
  personne d'autre ne peut en produire un qui se re-prouve sous elle. Il atteste
  l'INTÉGRITÉ de ce qui a été enregistré — que rien n'a été modifié, retiré ni
  ajouté après coup —, et une troncature est démasquée par un COMPTAGE contre
  l'ancre, sans clé et sans cryptographie.

  CE QU'IL N'OPPOSE PAS, et il faut le dire aussi fort. Il n'atteste pas la
  CORRECTION de la décision enregistrée : un raisonnement faux, scellé
  fidèlement, produit un reçu parfaitement valide. Il ne dit rien non plus de ce
  qui n'est pas entré dans la chaîne.

  QUAND VOUS AVEZ FINI :

    docker compose stop base production               # arrêter, garder les données
    docker compose down -v                            # et ne rien laisser derrière
                                                      #   (⚠ -v efface AUSSI le volume)

  Le dossier « $DOSSIER », lui, reste : c'est le vôtre.

FIN
exit 0
