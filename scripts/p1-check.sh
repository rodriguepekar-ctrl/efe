#!/usr/bin/env bash
# SPDX-License-Identifier: UNLICENSED
#
# P1 — le profil de production, VÉRIFIÉ EN LE DÉMARRANT.
#
# ── Ce que ce script prouve, et pourquoi dans cet ordre ───────────────────────
#
#   1. les quatre features cohabitent — sans quoi rien de ce qui suit n'existe ;
#   2. le garde REFUSE trois fois, chacune en NOMMANT sa précondition — un garde
#      dont on ne sait pas s'il peut tomber ne garde rien ;
#   3. la triade complète démarre, et le rapport dit la VÉRITÉ, y compris que la
#      frontière est REFUSÉE faute de fournisseur ;
#   4. un run souverain est servi avec un vrai jeton ES256 ;
#   5. il SURVIT à un rollout — processus tué, autre processus démarré, et
#      `verify` répond `verified:true` depuis le neuf.
#
# ── La règle apprise trois fois dans ce dépôt ─────────────────────────────────
#
#  Une vérification qui ne lance pas ce que le client lancerait ne vérifie pas le
#  produit. `m6-check.sh` lançait des tests et un exemple ; `m4-check.sh` validait
#  un fichier compose. Les deux étaient verts pendant que le module ne marchait
#  pas. Ce script démarre donc un vrai binaire, contre un vrai Postgres, sur une
#  vraie socket, et il tue le processus pour de bon.
#
# ── Le piège qu'il refuse ─────────────────────────────────────────────────────
#
#  `ApiKey::from_env` ne valide qu'un PRÉFIXE (`sk-ant-`) sans appel réseau : une
#  clé bidon suffirait à faire dire « frontière SERVIE ». On ne s'en sert PAS.
#  L'étape 3 exige au contraire « REFUSÉE » — la réponse honnête sans clé. Un
#  contrôle qui maquille son résultat est le vert malhonnête que tout ce dépôt
#  combat.
#
#   variables : P1_PORT (défaut 18080) · P1_DATABASE_URL (sinon un conteneur)
#
set -uo pipefail
cd "$(dirname "$0")/.."

ok()    { printf '  \033[32m✓\033[0m %s\n' "$*"; }
ko()    { printf '  \033[31m✗\033[0m %s\n' "$*"; exit 1; }
note()  { printf '  \033[33m·\033[0m %s\n' "$*"; }
titre() { printf '\n\033[1m%s\033[0m\n' "$*"; }

FEAT="postgres,oidc,pii,frontier-llm"
PORT="${P1_PORT:-18080}"
BIN="target/debug/efe-swarm-gateway"
TMP=""
PID=""
CONTENEUR=""          # non vide ⟹ c'est NOUS qui l'avons créé, donc à nous de le retirer

# Un SEUL trap, un SEUL ménage. La leçon de `mz-check.sh` : un seul `trap … EXIT`
# survit, donc un second écrase le premier — et le serveur qu'il devait tuer
# survivait au script.
menage() {
  [ -n "$PID" ] && { kill "$PID" 2>/dev/null; wait "$PID" 2>/dev/null; } || true
  [ -n "$CONTENEUR" ] && docker rm -f "$CONTENEUR" >/dev/null 2>&1 || true
  [ -n "$TMP" ] && rm -rf "$TMP" || true
}
trap menage EXIT

printf '\033[1mP1 · le profil de production, démarré\033[0m\n'

# ═════════════════════════════════════════════════════════════════════════════
titre "1. Les quatre features cohabitent-elles ?"
#
#  `postgres` (store durable), `oidc` (authentification), `pii` (capacité de
#  caviarder) et `frontier-llm` (le coordinator). Le garde n'existe que sous la
#  dernière : sans elle, `EFE_LLM_FRONTIER_ENABLED=true` ne refuse RIEN.
#
#  ⭐ **É4 — LA PORTE DE QUALITÉ, SUR CES FEATURES.**
#
#  `cargo clippy --workspace --all-targets` compile les features PAR DÉFAUT, et les quatre
#  d'ici n'en sont pas : `frontier-llm` gate `routes/llm.rs` en entier (`routes/mod.rs:18`) et
#  allume trois fournisseurs d'`efe-coordinator` dont `gemini` est lui-même sous `cfg`. La porte
#  du dépôt pouvait donc être VERTE pendant que le chemin d'egress payant portait sa dette
#  intacte — et elle l'était : **sept défauts** y dormaient le 4 septembre, dont une paire
#  d'attributs `#[cfg]`/`#[test]` ORPHELINE que le rapport de P1 citait déjà comme la preuve que
#  ce chemin vivait hors de la rigueur du dépôt. Aucune porte ne pouvait la voir, puisque aucune
#  porte ne compilait cette feature.
#
#  Un contrôle qui DÉMARRE le profil de production sans en éprouver la qualité ne vérifie que la
#  moitié de son sujet. La porte vient donc en premier, avant même le build.
JOURNAL_PORTE="$(mktemp)"
if ! cargo clippy -q --locked -p efe-swarm-gateway --features "$FEAT" --all-targets \
       -- -D warnings > "$JOURNAL_PORTE" 2>&1; then
  grep -E 'error|warning' "$JOURNAL_PORTE" | head -20 | sed 's/^/      /'
  rm -f "$JOURNAL_PORTE"
  ko "la porte de qualité TOMBE sur les features de production. Le workspace peut être vert et \
ce chemin rouge : c'est exactement l'angle mort que ce contrôle ferme."
fi
rm -f "$JOURNAL_PORTE"
ok "clippy -D warnings passe sur $FEAT — la dette du chemin frontière est mesurée, pas supposée"

cargo build -q -p efe-swarm-gateway --features "$FEAT" 2>&1 \
  | grep -E '^error' && ko "le build de production ne compile pas"
[ -x "$BIN" ] || ko "binaire introuvable après build : $BIN"

#  ⚠ « le fichier existe » est un PROXY, pas la propriété — et c'est ainsi que la première version
#  de ce script s'est trompée. `target/debug/` est un chemin PARTAGÉ : n'importe quel
#  `cargo build`/`cargo test` sans features, lancé entre-temps (une autre session, un éditeur), y
#  réinstalle un binaire NU. Le garde disparaît alors avec sa feature, `EFE_LLM_FRONTIER_ENABLED`
#  ne refuse plus rien, et l'étape 2 attend indéfiniment un processus qui s'est mis à SERVIR.
#
#  On demande donc au binaire ce qu'il PORTE. Il sait le dire : `--profile-report` rend la liste
#  des features compilées, et ce champ existe précisément pour qu'on n'ait pas à la déduire.
porte_les_features() {
  local vues
  vues="$("$BIN" --profile-report 2>/dev/null \
    | python3 -c "import json,sys; print(','.join(sorted(json.load(sys.stdin).get('features', []))))" \
    2>/dev/null)"
  [ "$vues" = "frontier-llm,oidc,pii,postgres" ]
}
porte_les_features || ko "le binaire ne porte pas les quatre features. Un build sans features a \
réinstallé $BIN ; relancez, ou isolez avec CARGO_TARGET_DIR."
ok "postgres + oidc + pii + frontier-llm compilent ensemble, ET le binaire les porte"

#  ⚠ AVEC LES MÊMES FEATURES, et ce n'est pas un détail : la première version de ce script lançait
#  ce test SANS elles. Cargo re-liait alors un binaire NU dans `target/debug/`, détruisant la
#  propriété vérifiée deux lignes plus haut — et l'étape 2 attendait indéfiniment un processus qui
#  s'était mis à SERVIR. Un contrôle qui invalide lui-même ce qu'il vient d'établir, en PENDANT
#  plutôt qu'en échouant. Toute commande cargo de ce script porte donc le même jeu.
cargo test -q -p efe-swarm-gateway --features "$FEAT" --test p1_injections >/dev/null 2>&1 \
  || ko "l'invariant des injections échoue — un profil frontière ment sur ce qu'il exige"
ok "chaque profil frontière déclare EXACTEMENT les injections du garde"

#  Et on re-vérifie APRÈS : `target/debug/` est partagé, une session concurrente peut y réinstaller
#  un binaire nu à tout instant. Mieux vaut le dire que se taire et attendre.
porte_les_features || ko "le binaire a perdu ses features entre les deux étapes — \
un build concurrent a réinstallé $BIN. Relancez, ou isolez avec CARGO_TARGET_DIR."

# ═════════════════════════════════════════════════════════════════════════════
titre "2. Le garde refuse-t-il, et NOMME-t-il sa précondition ?"
#
#  Trois marches, trois `exit 78` (EX_CONFIG, non-retryable). On n'exige pas
#  seulement le refus : on exige que le message NOMME la variable manquante.
#  Un opérateur qui lit `exit 78` n'a que ce texte pour savoir quoi poser — et
#  l'un des trois messages ne la nommait pas avant P1.
TMP="$(mktemp -d)"
openssl ecparam -name prime256v1 -genkey -noout -out "$TMP/cle.pem" 2>/dev/null \
  || ko "openssl indisponible — impossible de fabriquer une paire ES256"
openssl ec -in "$TMP/cle.pem" -pubout -out "$TMP/cle.pub" 2>/dev/null
MASTER="$(openssl rand -hex 32)"
PEM="$(cat "$TMP/cle.pub")"

#  BORNÉE dans le temps, et c'est le point : si le garde ne refuse pas, le binaire SERT — et une
#  attente infinie est la pire des issues pour un contrôle. Il ne dit alors ni oui ni non. macOS
#  n'a pas `timeout` ; on surveille donc le processus à la main.
marche() {  # $1 = variable attendue dans le message ; le reste = l'environnement
  local attendu="$1"; shift
  local sortie code fils journal
  journal="$TMP/marche.$$"
  env -i PATH="$PATH" EFE_SWARM_GATEWAY_ADDR="127.0.0.1:$PORT" \
      EFE_LLM_FRONTIER_ENABLED=true "$@" "$BIN" > "$journal" 2>&1 &
  fils=$!
  for _ in $(seq 1 20); do
    kill -0 "$fils" 2>/dev/null || break
    sleep 1
  done
  if kill -0 "$fils" 2>/dev/null; then
    kill "$fils" 2>/dev/null; wait "$fils" 2>/dev/null
    ko "le binaire n'a PAS refusé : il s'est mis à SERVIR alors que $attendu manque. \
Le garde vit sous la feature \`frontier-llm\` — le binaire la porte-t-il encore ?"
  fi
  wait "$fils"; code=$?
  sortie="$(cat "$journal")"
  [ "$code" = "78" ] || ko "attendu exit 78, obtenu $code — le garde n'a pas refusé"
  case "$sortie" in
    *"$attendu"*) ok "refus en 78, et le message nomme $attendu" ;;
    *) ko "refus en 78 mais SANS nommer $attendu :\n      $(printf '%s' "$sortie" | tail -1)" ;;
  esac
}
marche EFE_NANO_AUDIT_HMAC_MASTER
marche EFE_OIDC_PUBLIC_KEY_PEM   EFE_NANO_AUDIT_HMAC_MASTER="$MASTER"
marche DATABASE_URL              EFE_NANO_AUDIT_HMAC_MASTER="$MASTER" EFE_OIDC_PUBLIC_KEY_PEM="$PEM"

# ═════════════════════════════════════════════════════════════════════════════
titre "3. La triade complète démarre — et le rapport dit la vérité"
#
#  Un vrai Postgres. `P1_DATABASE_URL` permet d'en désigner un existant ; sinon
#  on en démarre un. S'il n'y en a NI l'un NI l'autre, on échoue franchement :
#  sauter cette étape en silence rendrait le script vert sur un demi-produit.
if [ -n "${P1_DATABASE_URL:-}" ]; then
  DB="$P1_DATABASE_URL"
  note "Postgres fourni par P1_DATABASE_URL"
elif docker info >/dev/null 2>&1; then
  CONTENEUR="scella-p1-$$"
  PGPORT=$((PORT + 5000))
  docker run -d --name "$CONTENEUR" -e POSTGRES_PASSWORD=p1 -e POSTGRES_DB=scella \
    -p "127.0.0.1:$PGPORT:5432" postgres:16-alpine >/dev/null 2>&1 \
    || ko "impossible de démarrer le conteneur Postgres"
  #  ⚠ `pg_isready` SANS `-h` interroge la socket UNIX du conteneur — et celle-ci répond déjà
  #  pendant `initdb`, quand le serveur temporaire tourne mais que le TCP ne sert pas encore. La
  #  première version de ce script s'y est laissé prendre : sonde verte, puis
  #  « unexpected response from SSLRequest: 0x00 » côté passerelle. Un proxy à la place de la
  #  propriété, pour la deuxième fois dans ce fichier.
  #
  #  On sonde donc DEPUIS L'HÔTE, sur le port mappé, exactement comme sqlx : un SSLRequest, et
  #  Postgres répond `S` ou `N`. Rien d'autre ne répond cela.
  pret() {
    python3 - "$PGPORT" <<'SONDE'
import socket, struct, sys
try:
    s = socket.create_connection(("127.0.0.1", int(sys.argv[1])), timeout=2)
    s.sendall(struct.pack("!ii", 8, 80877103))   # longueur, code SSLRequest
    r = s.recv(1); s.close()
    sys.exit(0 if r in (b"S", b"N") else 1)
except Exception:
    sys.exit(1)
SONDE
  }
  for _ in $(seq 1 60); do pret && break; sleep 1; done
  pret || ko "Postgres n'accepte pas de connexion sur 127.0.0.1:$PGPORT après 60 s"
  DB="postgres://postgres:p1@127.0.0.1:$PGPORT/scella"
  ok "Postgres 16 démarré (conteneur $CONTENEUR)"
else
  ko "ni P1_DATABASE_URL ni démon Docker : P1 exige un store DURABLE, et le vérifier \
sans base serait vérifier autre chose"
fi

lancer() {  # $1 = fichier de journal
  env -i PATH="$PATH" \
      EFE_SWARM_GATEWAY_ADDR="127.0.0.1:$PORT" \
      EFE_LLM_FRONTIER_ENABLED=true \
      EFE_NANO_AUDIT_HMAC_MASTER="$MASTER" \
      EFE_OIDC_PUBLIC_KEY_PEM="$PEM" \
      DATABASE_URL="$DB" \
      EFE_SWARM_BUNDLE_ENABLED=true \
      EFE_PII_DEFAULT_POLICY=redact \
      "$BIN" > "$1" 2>&1 &
  PID=$!
  for _ in $(seq 1 40); do
    curl -sf "http://127.0.0.1:$PORT/healthz" >/dev/null 2>&1 && return 0
    kill -0 "$PID" 2>/dev/null || { cat "$1"; ko "la passerelle est morte au démarrage"; }
    sleep 1
  done
  cat "$1"; ko "la passerelle n'a pas répondu en 40 s"
}
lancer "$TMP/boot1.log"
ok "la passerelle écoute — le garde a laissé passer"

grep -q 'préflight frontière OK' "$TMP/boot1.log" \
  || ko "le préflight n'a pas annoncé OK"
grep -q 'PgRunStore + PgMemoryStore câblés' "$TMP/boot1.log" \
  || ko "le store durable n'est pas câblé — le connect/migrate a-t-il échoué ?"

ligne() { grep -E "^│  $1 " "$TMP/boot1.log" | head -1; }
etat()  { ligne "$1" | awk '{print $3}'; }
for cap in provenance authentification; do
  [ "$(etat "$cap")" = "SERVIE" ] || ko "$cap : attendu SERVIE, obtenu « $(etat "$cap") »"
done
[ "$(ligne 'store durable' | awk '{print $4}')" = "SERVIE" ] \
  || ko "store durable : attendu SERVIE"
ok "provenance, store durable et authentification : SERVIE"

# ⭐ L'assertion la plus importante du script.
[ "$(etat 'frontière')" = "REFUSÉE" ] \
  || ko "frontière : attendu REFUSÉE (aucune clé de fournisseur), obtenu « $(etat 'frontière') ». \
Une clé bidon suffirait à la faire dire SERVIE — c'est exactement ce qu'on refuse de faire."
ok "frontière : REFUSÉE, en nommant la cause — le 3ᵉ état de M0, honnête"

# Le rapport MACHINE doit dire la même chose que le rapport de démarrage. Ils sont
# calculés par deux chemins (`Resolved` et `Observed`) ; s'ils divergeaient, l'un
# des deux mentirait à quelqu'un.
RAPPORT="$(env -i PATH="$PATH" \
    EFE_LLM_FRONTIER_ENABLED=true EFE_NANO_AUDIT_HMAC_MASTER="$MASTER" \
    EFE_OIDC_PUBLIC_KEY_PEM="$PEM" DATABASE_URL="$DB" EFE_SWARM_BUNDLE_ENABLED=true \
    EFE_PII_DEFAULT_POLICY=redact "$BIN" --profile-report 2>/dev/null)"
FLAG_FRONT="$(printf '%s' "$RAPPORT" | python3 -c "
import json,sys
d=json.load(sys.stdin)
print(next(c['status'] for c in d['capabilities'] if c['capability']=='frontière'))")"
[ "$FLAG_FRONT" = "REFUSÉE" ] \
  || ko "--profile-report dit « $FLAG_FRONT » là où le démarrage dit REFUSÉE : deux rapports, deux vérités"
ok "\`--profile-report\` concorde avec le rapport de démarrage"

# ═════════════════════════════════════════════════════════════════════════════
titre "4. Un run souverain, avec un vrai jeton"
#
#  ⚠ Activer l'OIDC ferme AUSSI le chemin souverain : sans jeton, 401. C'est
#  correct, et c'est la découverte qui a justifié `p1-jeton.sh`.
CODE="$(curl -s -o /dev/null -w '%{http_code}' -X POST "http://127.0.0.1:$PORT/v1/swarm/run" \
        -H 'content-type: application/json' -d @examples/run.json)"
[ "$CODE" = "401" ] || ko "sans jeton, attendu 401, obtenu $CODE — l'auth ne garde pas le chemin souverain"
ok "sans jeton : 401 — l'OIDC garde aussi le chemin souverain"

JWT="$(bash scripts/p1-jeton.sh "$TMP/cle.pem" 2>/dev/null)" \
  || ko "scripts/p1-jeton.sh n'a pas su forger de jeton"
curl -s -o "$TMP/run.json" -w '%{http_code}' -X POST "http://127.0.0.1:$PORT/v1/swarm/run" \
  -H "authorization: Bearer $JWT" -H 'content-type: application/json' \
  -d @examples/run.json > "$TMP/code.txt"
[ "$(cat "$TMP/code.txt")" = "200" ] \
  || { cat "$TMP/run.json"; ko "run refusé : HTTP $(cat "$TMP/code.txt")"; }
RUN="$(python3 -c "import json;print(json.load(open('$TMP/run.json'))['run_id'])")"
ok "run souverain servi : $RUN"

# ═════════════════════════════════════════════════════════════════════════════
titre "5. LE CRITÈRE — le reçu survit-il au rollout ?"
#
#  Le message du garde dit « les reçus sont perdus à chaque rollout ». Voici ce
#  rollout, joué pour de vrai : on tue le processus et on en démarre un AUTRE.
kill "$PID" 2>/dev/null; wait "$PID" 2>/dev/null; PID=""
ok "passerelle arrêtée"
lancer "$TMP/boot2.log"
ok "un AUTRE processus écoute sur le même port"

curl -s -o "$TMP/verify.json" -w '%{http_code}' -H "authorization: Bearer $JWT" \
  "http://127.0.0.1:$PORT/v1/swarm/verify/$RUN" > "$TMP/code2.txt"
[ "$(cat "$TMP/code2.txt")" = "200" ] \
  || ko "verify après redémarrage : HTTP $(cat "$TMP/code2.txt") — le reçu n'a pas survécu"
python3 - "$TMP/verify.json" <<'PY' || exit 1
import json, sys
d = json.load(open(sys.argv[1]))
manques = [k for k in ("verified", "root_matches", "chain_intact", "content_intact") if not d.get(k)]
if manques:
    print(f"  \033[31m✗\033[0m le reçu a survécu mais ne se re-prouve plus : {manques} — {d}")
    sys.exit(1)
print(f"  \033[32m✓\033[0m verified · root_matches · chain_intact · content_intact "
      f"({d.get('records', '?')} enregistrements)")
PY
ok "LE REÇU SURVIT AU ROLLOUT et se re-prouve depuis un processus neuf"

# ═════════════════════════════════════════════════════════════════════════════
titre "6. L'ÉPREUVE DE RESTAURATION — détruire, et remonter"
#
#  Le trou que cette section ferme n'était pas « il manque un script de sauvegarde ». C'était
#  plus bas : `RunStore` a `put`, `put_stored` et `get`, et **aucune méthode n'énumère**. Un
#  registre que l'AI Act art. 12 demande de garder dix ans, et qu'aucun code ne sait PARCOURIR,
#  ne peut être ni exporté, ni sauvegardé, ni migré. Il manquait la seule opération dont un
#  script de sauvegarde aurait eu besoin.
#
#  `--exporter-runs` / `--importer-runs` la fournissent. Cette section les éprouve de la seule
#  façon qui compte : en DÉTRUISANT le store et en le remontant. Une sauvegarde qu'on n'a jamais
#  restaurée n'est pas une sauvegarde, c'est un fichier.
#
#  ⚠ ELLE NE DÉTRUIT QUE CE QU'ELLE A CRÉÉ. Si l'exploitant a désigné sa propre base par
#  `P1_DATABASE_URL`, l'épreuve est SAUTÉE et le dit : effacer la table d'une base qu'on ne
#  possède pas serait un défaut bien pire que celui qu'on cherche à fermer.
if [ -z "$CONTENEUR" ]; then
  note "épreuve de restauration SAUTÉE : la base vient de P1_DATABASE_URL, elle ne nous"
  note "appartient pas, et cette épreuve efface la table. Rejouer sans P1_DATABASE_URL."
else
  psql() { docker exec "$CONTENEUR" psql -qtAX -U postgres -d scella "$@"; }
  export_bin() { env -i PATH="$PATH" DATABASE_URL="$DB" "$BIN" "$@"; }

  export_bin --exporter-runs > "$TMP/registre.jsonl" 2>"$TMP/export.log" \
    || ko "l'export a échoué : $(tail -2 "$TMP/export.log")"
  LIGNES=$(wc -l < "$TMP/registre.jsonl" | tr -d ' ')
  # Plancher anti-vacuité : un export qui rend zéro ligne sortirait 0 et se croirait fait.
  [ "${LIGNES:-0}" -ge 1 ] || ko "l'export a rendu $LIGNES ligne(s) — il n'a rien parcouru"
  grep -q "$RUN" "$TMP/registre.jsonl" || ko "l'export ne contient pas le run servi ($RUN)"
  ok "export : $LIGNES ligne(s), dont le run servi"

  #  On DÉTRUIT. C'est le seul geste qui distingue une sauvegarde d'une copie d'espoir.
  psql -c "DELETE FROM swarm_runs" >/dev/null 2>&1 || ko "la destruction du store a échoué"
  RESTE=$(psql -c "SELECT count(*) FROM swarm_runs" 2>/dev/null | tr -d ' \r')
  [ "$RESTE" = "0" ] || ko "la table n'est pas vide après destruction ($RESTE ligne(s))"

  #  Et on VÉRIFIE que le reçu a bien disparu. Sans ce contrôle, un cache rendrait la
  #  restauration verte sans qu'elle ait servi à quoi que ce soit — le pire des faux positifs :
  #  celui qui valide une sauvegarde qui ne restaure rien.
  CODE_VIDE=$(curl -s -o /dev/null -w '%{http_code}' -H "authorization: Bearer $JWT" \
    "http://127.0.0.1:$PORT/v1/swarm/verify/$RUN")
  [ "$CODE_VIDE" != "200" ] \
    || ko "le reçu se re-prouve encore alors que la table est VIDE — un cache rend cette épreuve \
sans objet, et la sauvegarde ne prouverait rien."
  ok "store détruit : le reçu ne répond plus (HTTP $CODE_VIDE)"

  export_bin --importer-runs < "$TMP/registre.jsonl" 2>"$TMP/import.log" \
    || ko "l'import a échoué : $(tail -2 "$TMP/import.log")"
  grep -qE "$LIGNES ligne\(s\) lue\(s\), $LIGNES écrite" "$TMP/import.log" \
    || ko "l'import n'a pas réécrit tout ce qu'il a lu : $(cat "$TMP/import.log")"
  ok "import : $(tr -d '\n' < "$TMP/import.log" | sed 's/^→ //')"

  curl -s -o "$TMP/verify3.json" -w '%{http_code}' -H "authorization: Bearer $JWT" \
    "http://127.0.0.1:$PORT/v1/swarm/verify/$RUN" > "$TMP/code3.txt"
  [ "$(cat "$TMP/code3.txt")" = "200" ] \
    || ko "verify après restauration : HTTP $(cat "$TMP/code3.txt") — le reçu n'est pas remonté"
  python3 - "$TMP/verify3.json" <<'PY2' || exit 1
import json, sys
d = json.load(open(sys.argv[1]))
manques = [k for k in ("verified", "root_matches", "chain_intact", "content_intact") if not d.get(k)]
if manques:
    print(f"  \033[31m✗\033[0m restauré mais ne se re-prouve plus : {manques} — {d}")
    sys.exit(1)
print(f"  \033[32m✓\033[0m LE REÇU EST REVENU D'UN STORE DÉTRUIT et se re-prouve "
      f"({d.get('records', '?')} enregistrements)")
PY2

  #  L'idempotence : le store est write-once (`ON CONFLICT DO NOTHING`). Rejouer une restauration
  #  ne doit rien écraser — sans quoi personne n'oserait la rejouer, et une restauration qu'on
  #  n'ose pas rejouer est une restauration qu'on ne fera pas à trois heures du matin.
  export_bin --importer-runs < "$TMP/registre.jsonl" 2>"$TMP/import2.log" \
    || ko "le second import a échoué"
  grep -qE "$LIGNES ligne\(s\) lue\(s\), 0 écrite" "$TMP/import2.log" \
    || ko "rejouer l'import a RÉÉCRIT des lignes : $(cat "$TMP/import2.log") — le write-once ne tient pas"
  ok "rejouer la restauration n'écrase rien (0 écrite, $LIGNES existaient déjà)"
fi

# ═════════════════════════════════════════════════════════════════════════════
titre "7. Le critère de M3, que personne ne jouait"
#
#  ⭐ **É4** — `routes/llm.rs` vit sous `#[cfg(feature = "frontier-llm")]`, et les deux tests
#  qui portent le critère de M3 — « le SLM meurt, la mission passe, et la RAISON remonte » —
#  y sont en plus `#[cfg(not(feature = "oidc"))]`. Les deux conditions se composent :
#
#      cargo test -p efe-swarm-gateway --lib m3_le_slm
#        (défaut)                      → 0 test  · le module n'est pas compilé
#        --features frontier-llm       → 2 tests ✓
#        --features frontier-llm,oidc  → 0 test  · `cfg(not(oidc))` les retire
#
#  Ni `cargo test --workspace` ni le profil de production n'emploient cette combinaison.
#  Deux portes vertes, et ces deux tests ne tournaient nulle part.
#
#  ⚠ Cette commande est ici, EN DERNIER, et pas au début : elle porte un jeu de features
#  DIFFÉRENT, donc cargo relie un binaire différent dans `target/debug/` — exactement le
#  piège que l'étape 1 décrit et que `porte_les_features` surveille. Placée plus haut, elle
#  détruirait la propriété que ce script vient d'établir. Ici, plus rien n'en dépend.
M3=$(cargo test -q --locked -p efe-swarm-gateway --features frontier-llm --lib m3_le_slm 2>&1 \
     | grep -E '^test result:' | head -1)
case "$M3" in
  *"2 passed; 0 failed"*) ok "les deux tests M3 passent ($M3)" ;;
  *"0 passed"*) ko "AUCUN test M3 n'a été joué — un \`cfg\` a bougé, et le critère de M3 \
n'est plus éprouvé nulle part. C'est le trou que cette étape existe pour rendre visible." ;;
  *) ko "les tests M3 ne passent pas : $M3" ;;
esac

# ═════════════════════════════════════════════════════════════════════════════
titre "8. Les TROIS destinations d'egress ne sont plus des constantes"
#
#  La PREMIÈRE ligne de « ce que ce contrôle ne prouve pas » disait : « ANTHROPIC_URL est une
#  constante EN DUR : il n'existe aucun moyen de pointer un faux fournisseur local, ni un Azure
#  OpenAI, ni un mandataire d'entreprise. » C'était vrai, et c'était le trou qui rendait
#  l'aller-retour frontière intestable — et qui limitait la souveraineté annoncée à une adresse
#  gravée vers les États-Unis.
#
#  Les trois les remplacent — `ClaudeProvider::with_endpoint` (URL entière : un seul chemin),
#  `GptProvider::with_base_url` et `GeminiProvider::with_base_url` (une BASE : deux et trois
#  chemins en dérivent) — toutes VALIDÉES par l'analyseur unique du dépôt
#  (`scella-authority`) : `https` partout, `http` seulement vers la machine locale, et aucun
#  `userinfo` — parce que l'endpoint est SCELLÉ DANS LE REÇU et qu'on n'y met pas d'identifiants.
#
#  ⚠ Même piège que la section 7, et même remède : ces tests vivent sous
#  `#![cfg(all(feature = "anthropic", feature = "openai", feature = "gemini"))]`, aucune n'étant
#  dans le défaut de `efe-coordinator`
#  (`default = []`). `cargo test --workspace` les compte donc pour ZÉRO. C'est ici qu'ils
#  tournent — et « 0 passed » est un ÉCHEC, pas un succès : ce serait le signe qu'un `cfg` a
#  bougé et que plus rien ne garde la destination de l'egress.
EP=$(cargo test -q -p efe-coordinator --features anthropic,openai,gemini --test endpoint_configurable 2>&1 | grep -E '^test result' | head -1)
case "$EP" in
  *"10 passed; 0 failed"*) ok "les 3 destinations sont configurables, validées, et le reçu annonce la VRAIE ($EP)" ;;
  *"0 passed"*) ko "AUCUN test d'endpoint n'a été joué — un \`cfg\` a bougé, et plus rien ne \
garde la destination de l'egress ni ce que le reçu en dit." ;;
  *) ko "les tests d'endpoint ne passent pas : $EP" ;;
esac

# ═════════════════════════════════════════════════════════════════════════════
titre "9. L'aller-retour frontière, sur une vraie socket, pour les trois"
#
#  Ce que la section 8 rend possible, celle-ci l'EXÉCUTE. Un serveur HTTP réel écoute sur la
#  boucle locale, le provider est pointé vers lui, et l'appel part, arrive, et revient — avec
#  ses en-têtes vérifiés depuis L'EXTÉRIEUR, en regardant la requête reçue plutôt que le code
#  qui la construit.
#
#  Ce chemin — l'egress payant, de bout en bout — n'avait JAMAIS été parcouru dans ce dépôt, pour
#  AUCUN des trois. Il ne pouvait pas l'être : les destinations étaient des constantes, donc les
#  atteindre exigeait une clé, de l'argent et un réseau.
#
#  Chaque cas vérifie aussi ses EN-TÊTES depuis l'extérieur : `anthropic-version` + `x-api-key`,
#  `authorization: Bearer`, et `x-goog-api-key` — ce dernier gardant la règle §2.3 de Gemini,
#  « la clé en en-tête, jamais dans l'URL », contre une régression qui la remettrait en query.
#
#  Ce qu'il ne prouve toujours pas : qu'Anthropic réponde ainsi. Le faux fournisseur rend la
#  forme documentée de l'API ; si l'API changeait, ce test resterait vert. Il prouve le CHEMIN,
#  jamais le PARTENAIRE.
AR=$(cargo test -q -p efe-coordinator --features anthropic,openai,gemini --test aller_retour_frontiere 2>&1 | grep -E '^test result' | head -1)
case "$AR" in
  *"5 passed; 0 failed"*) ok "les 3 allers-retours partent, arrivent et reviennent ($AR)" ;;
  *"0 passed"*) ko "AUCUN aller-retour n'a été joué — le chemin d'egress n'est de nouveau \
éprouvé nulle part." ;;
  *) ko "l'aller-retour ne passe pas : $AR" ;;
esac

# ═════════════════════════════════════════════════════════════════════════════
titre "Ce que ce contrôle ne prouve PAS, et qu'il faut redire"
cat <<'FIN'
    · Un aller-retour vers un VRAI fournisseur. Cette ligne disait autre chose :
      « `ANTHROPIC_URL` est une constante EN DUR : aucun moyen de pointer un faux
      fournisseur local, ni un Azure OpenAI, ni un mandataire d'entreprise ». Ce
      n'est plus vrai d'AUCUN des trois — les constantes sont des défauts, les
      destinations se configurent et se valident (section 7), et les trois
      allers-retours SUR SOCKET sont joués (section 9).
      Ce qui reste manquant est plus étroit, et plus honnête : que le partenaire
      réponde comme le faux fournisseur le suppose. Si l'API d'Anthropic changeait,
      les sections 7 et 8 resteraient vertes. Elles prouvent le CHEMIN, jamais le
      PARTENAIRE — et vers lui, ce qui manque tient en une clé : la vôtre, et elle
      coûte de l'argent.
    · Que votre IdP fonctionne. La paire ES256 de ce script est fabriquée sur
      place : quiconque la détient forge un jeton. Elle démontre le MÉCANISME,
      jamais la SUBSTANCE. Et la passerelle n'accepte QUE ES256 — RS256, défaut
      de Keycloak et d'Auth0, sera refusé.
    · Que l'allowlist d'egress soit écrite. Absente ⟹ 403 pour TOUT tenant :
      même avec une vraie clé, la frontière ne sert alors rien.
    · La tenue en charge. Toujours absente : aucun contrôle de ce dépôt ne
      mesure la passerelle sous charge, et ce script n'y touche pas.

    Cette ligne disait aussi « ni sauvegarde, ni restauration, ni rotation de
    clé — c'est P2 ». Elle a cessé d'être vraie le jour où P2 a été livré, et
    elle est corrigée ici plutôt que laissée à vieillir : c'est le défaut
    récurrent de ce dépôt — un contrôle dont le TEXTE devient faux un module
    plus tard — et le tolérer une fois de plus reviendrait à faire mentir le
    document qui sert justement à ne pas se mentir. Les trois existent :
    `scella-conserver`, et `bash scripts/p2-check.sh` les rejoue.
FIN

printf '\n\033[32mP1 tient.\033[0m  (démarré, servi, tué, redémarré, DÉTRUIT, restauré, re-prouvé)\n'
