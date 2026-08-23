#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
#  M6 — la conservation limitée, éprouvée.
#
#  Le critère : « un cas expiré disparaît, et tout le reste se re-prouve
#  encore ». Il a l'air contradictoire — une chaîne scellée se replie sur
#  les OCTETS ENTIERS de chaque nœud, donc retirer un record casse tous
#  les sceaux qui suivent.
#
#  Il ne l'est pas, parce qu'on ne retire rien : les champs effaçables
#  sont scellés CHIFFRÉS, et broyer détruit la clé du cas. Pas un octet
#  du DAG ne bouge.
#
#  Ce script vérifie les quatre choses qui font que ce n'est pas du
#  théâtre :
#    1. le critère lui-même, sur le manifest-root AVANT et APRÈS ;
#    2. le VÉRIFIEUR TIERS zéro-dépendance re-prouve encore — sans quoi
#       la promesse du produit tomberait le jour où l'on exerce un droit ;
#    3. le clair n'est pas dans les octets publiés ;
#    4. le garde : UN seul point de scellement, aucun joker de `match`.
#
#  Usage :  bash scripts/m6-check.sh
#  Sortie 0 si la conservation tient ; ≥ 1 sinon.
# ─────────────────────────────────────────────────────────────────────
set -euo pipefail
cd "$(dirname "$0")/.."

ok()   { printf '  \033[32m✓\033[0m %s\n' "$*"; }
ko()   { printf '  \033[31m✗\033[0m %s\n' "$*"; exit 1; }
titre(){ printf '\n\033[1m%s\033[0m\n' "$*"; }

titre "1. Le critère : « un cas expiré disparaît, et tout le reste se re-prouve encore »"
cargo test -q -p provenance-capture --test m6_conservation_limitee >/dev/null 2>&1 \
  || ko "le critère de M6 ne passe pas"
ok "le DAG est octet pour octet identique après le broyage, et le contenu est parti"

titre "2. Le vérifieur TIERS zéro-dépendance re-prouve une chaîne broyée"
cargo test -q -p verify-mini --test m6_broyage_invisible_au_verifieur >/dev/null 2>&1 \
  || ko "verify-mini ne re-prouve plus après broyage — la promesse du produit tomberait"
ok "même verdict, même manifest-root : le broyage lui est invisible"

titre "3. Le garde : un seul point de scellement, aucun joker"
cargo test -q -p provenance-capture --test m6_un_seul_scellement >/dev/null 2>&1 \
  || ko "le garde de M6 est tombé"
ok "un seul \`self.dag.append(\` dans le store — aucun tap ne peut oublier d'enrober"
ok "aucun joker de \`match\` : une variante de \`Node\` ajoutée casse la compilation"

titre "4. La primitive et sa règle la plus fragile"
cargo test -q -p provenance-dag --lib shred >/dev/null 2>&1 || ko "la primitive de broyage est cassée"
ok "aucune clé de cas n'est dérivée — une clé dérivée se re-dérive, la broyer ne détruirait rien"

# NOTE — ce script EXÉCUTE les gardes, il ne les RÉ-IMPLÉMENTE pas.
#
# La première version faisait ici son propre `grep -q 'fn derive'`. Il tombait sur la LISTE
# D'INTERDITS du garde lui-même (`for interdit in ["fn derive", …]`) : un second analyseur, plus
# faible que le premier, qui refusait du code correct. C'est le défaut que M1 a passé un module
# entier à supprimer — cinq analyseurs d'autorité dont un seul était juste. On n'en rouvre pas un
# ici pour le confort d'un script.

titre "5. La démonstration, à l'œil"
SORTIE=$(cargo run -q -p provenance-capture --example m6_conservation 2>&1)
AVANT=$(printf '%s' "$SORTIE" | grep -m1 -A1 'AVANT le broyage' | grep -oE '[0-9a-f]{64}')
APRES=$(printf '%s' "$SORTIE" | grep -m1 -A1 'APRÈS le broyage' | grep -oE '[0-9a-f]{64}')
[ -n "$AVANT" ] && [ "$AVANT" = "$APRES" ] \
  || ko "les racines diffèrent : avant=$AVANT après=$APRES"
ok "manifest-root avant = après = ${AVANT:0:16}…"
printf '%s' "$SORTIE" | grep -q 'détruit à t=' || ko "aucun contenu n'a été détruit — la démo serait vide"
ok "et le contenu des dossiers expirés est détruit"

titre "6. M6 CÂBLÉ — sur un vrai processus"
#
#  L'audit M6→M8 a trouvé que M6 n'était allumé par AUCUN binaire : la façade
#  existait sur AuditService, personne ne l'appelait. Cette section démarre le
#  service réel avec une conservation déclarée, et vérifie que la destruction
#  atteint le DISQUE et SURVIT au redémarrage.
#
cargo test -q -p audit-console --test m6_cable_au_service >/dev/null 2>&1 \
  || ko "les tests du câblage ne passent pas"
ok "critère du câblage, durabilité, et l'ordre disque-avant-mémoire"

TRAVAIL="$(mktemp -d)"
trap 'rm -rf "$TRAVAIL"' EXIT
MAINTENANT=$(date +%s)
CLE_A=$(printf 'a%.0s' {1..64})
CLE_B=$(printf 'b%.0s' {1..64})

printf '# trousseau de vérification\nVIEUX\t%s\t0\nRECENT\t%s\t%s\n' \
  "$CLE_A" "$CLE_B" "$MAINTENANT" > "$TRAVAIL/acme.keys"
chmod 600 "$TRAVAIL/acme.keys"

cat > "$TRAVAIL/decl.json" <<JSON
{
  "tenants": [{ "id": "acme",
                "conservation": { "horizon_s": 3600, "trousseau": "$TRAVAIL/acme.keys" } }],
  "sessions": [{ "token": "jeton-verif-m6", "tenant": "acme", "role": "auditor", "subject": "a" }]
}
JSON

cargo build -q -p audit-console 2>/dev/null || ko "le binaire ne compile pas"
BIN="target/debug/audit-console"
PORT=18266

demarrer() {
  EFE_CONSOLE_ADDR="127.0.0.1:$PORT" \
  EFE_CONSOLE_ROOT="$(printf '0%.0s' {1..64})" \
  EFE_CONSOLE_DEPLOIEMENT="$TRAVAIL/decl.json" \
    "$BIN" >/dev/null 2> "$1" &
  PID=$!
  for _ in $(seq 1 100); do
    curl -sf "http://127.0.0.1:$PORT/healthz" >/dev/null 2>&1 && break
    sleep 0.1
  done
  kill "$PID" 2>/dev/null || true
  wait "$PID" 2>/dev/null || true
}

demarrer "$TRAVAIL/boot1.log"
grep -q "conservation « acme » : 2 clé(s) de cas chargée(s)" "$TRAVAIL/boot1.log" \
  || { cat "$TRAVAIL/boot1.log"; ko "la conservation n'a pas été activée au démarrage"; }
ok "la conservation est ACTIVÉE par le binaire (2 clés chargées)"

grep -q "1 cas broyé(s), destruction DURABLE" "$TRAVAIL/boot1.log" \
  || { cat "$TRAVAIL/boot1.log"; ko "le balayage d'amorçage n'a rien broyé"; }
ok "le balayage d'amorçage a broyé le cas expiré"

grep -q "^VIEUX	BROYE	" "$TRAVAIL/acme.keys" || ko "la clé de VIEUX est encore sur le disque"
grep -q "$CLE_A" "$TRAVAIL/acme.keys" && ko "la clé de VIEUX est encore lisible dans le fichier"
grep -q "$CLE_B" "$TRAVAIL/acme.keys" || ko "la clé de RECENT a disparu — trop broyé"
DATE_BROYAGE=$(awk '$1=="VIEUX"{print $3}' "$TRAVAIL/acme.keys")
ok "sur le DISQUE : clé détruite, cas conservé, date inscrite ($DATE_BROYAGE)"

demarrer "$TRAVAIL/boot2.log"
grep -q "conservation « acme » : 1 clé(s) de cas chargée(s)" "$TRAVAIL/boot2.log" \
  || { cat "$TRAVAIL/boot2.log"; ko "le redémarrage n'a pas rechargé le trousseau amputé"; }
DATE_APRES=$(awk '$1=="VIEUX"{print $3}' "$TRAVAIL/acme.keys")
[ "$DATE_BROYAGE" = "$DATE_APRES" ] \
  || ko "la date de destruction a changé au redémarrage ($DATE_BROYAGE → $DATE_APRES)"
ok "après REDÉMARRAGE : toujours broyé, et à la MÊME date"

titre "7. Une destruction qui n'atteint pas le disque n'a PAS lieu"
# Le RENAME atomique a besoin d'écrire dans le DOSSIER : c'est lui qu'on verrouille, pas le
# fichier. Les journaux restent dans le parent, sans quoi le binaire ne pourrait pas s'exprimer —
# la première version de cette section verrouillait le dossier des journaux et se dégradait en un
# faux succès.
mkdir -p "$TRAVAIL/verrou"
printf 'VIEUX2\t%s\t0\n' "$CLE_A" > "$TRAVAIL/verrou/fige.keys"
sed "s|$TRAVAIL/acme.keys|$TRAVAIL/verrou/fige.keys|" "$TRAVAIL/decl.json" > "$TRAVAIL/decl2.json"
chmod 500 "$TRAVAIL/verrou"
EFE_CONSOLE_ADDR="127.0.0.1:$((PORT+1))" \
EFE_CONSOLE_ROOT="$(printf '0%.0s' {1..64})" \
EFE_CONSOLE_DEPLOIEMENT="$TRAVAIL/decl2.json" \
  "$BIN" >/dev/null 2> "$TRAVAIL/boot3.log" &
PID=$!
sleep 2
kill "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
chmod 700 "$TRAVAIL/verrou"
if grep -q "ONT EXPIRÉ et n'ont PAS été broyés" "$TRAVAIL/boot3.log"; then
  grep -q "$CLE_A" "$TRAVAIL/verrou/fige.keys" || ko "la clé a disparu alors que l'écriture avait échoué"
  ok "écriture impossible ⟹ RIEN n'est détruit, et le journal le CRIE"
else
  cat "$TRAVAIL/boot3.log" >&2
  ko "l'écriture aurait dû échouer et être CRIÉE — un balayage muet sur un trousseau verrouillé \
      serait la panne silencieuse que ce module existe pour supprimer"
fi

titre "Ce que M6 NE fait PAS, et qu'il faut redire"
cat <<'FIN'
  · Le NŒUD reste. Un auditeur voit toujours qu'un acte a eu lieu, son rôle,
    sa date, ses arêtes — il ne peut plus lire ce qu'il disait. C'est la
    limitation de conservation, pas l'effacement d'une trace.
  · Rien ne protège le PASSÉ : un record scellé avant l'activation d'un
    trousseau porte son contenu en clair pour toujours.
  · Ce n'est pas une PREUVE de destruction : sauvegardes et instantanés
    peuvent conserver une clé ailleurs. La discipline des copies reste une
    affaire d'exploitation.
  · Le trousseau se recharge désormais au redémarrage, depuis le fichier
    déclaré — mais son ENTROPIE reste le geste du déployeur : ce service lit
    des clés, il n'en fabrique aucune. Une clé qu'il pourrait dériver se
    re-dériverait, et le broyage ne détruirait rien.
    (Cette ligne disait le contraire jusqu'à l'audit M6→M8 : la limite
    déclarée par M6 était devenue fausse un module plus tard.)
  · Un trousseau perdu se lit « clé absente », jamais « broyé » : on n'affirme
    pas une destruction qu'on n'a pas faite. Un cas broyé, lui, garde sa DATE
    dans le fichier — supprimer sa ligne aurait effacé le procès-verbal de
    l'effacement.
FIN

printf '\n\033[32mM6 tient.\033[0m\n'
