#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
#  R1 — l'extraction de mode1.rs, CONTRÔLÉE.
#
#  Le critère de la feuille de route du 24 août est double et littéral :
#
#    (1) `cargo tree -p efe-meta-swarm-server -F core-integration`
#        énumère AU PLUS DIX crates du dépôt ;
#    (2) les tests de M1.d — le bout-en-bout du vrai contrôleur sur
#        gRPC — passent INCHANGÉS.
#
#  Ce script mesure le premier et exécute le second. Il ne recompte pas
#  à la main ce que cargo sait dire : il intersecte l'arbre réel avec la
#  liste réelle des membres du workspace, parce qu'un « grep -c » sur la
#  sortie de `cargo tree` compterait aussi les crates tierces.
#
#  Il vérifie en plus deux choses que le critère ne demande pas, et qui
#  sont la vraie garantie de non-régression :
#
#    · que `efe-mode1` soit RESTÉ une feuille (une seule dépendance du
#      dépôt) — sans quoi la fermeture repartira à la hausse sans que
#      personne ne s'en aperçoive ;
#    · que `mode1.rs` n'ait REPRIS aucun lien vers `efe-server` — c'est
#      l'unique `use crate::` de ce fichier qui avait attaché tout
#      l'assemblage à la pile applicative.
#
#  Usage :  bash scripts/r1-check.sh
#  Sortie 0 si tout tient ; ≥ 1 sinon.
# ─────────────────────────────────────────────────────────────────────
set -euo pipefail
cd "$(dirname "$0")/.."

TRAVAIL=$(mktemp -d)
trap 'rm -rf "$TRAVAIL"' EXIT
ECHECS=0

titre() { printf '\n\033[1;34m%s\033[0m\n' "$1"; }
ok()    { printf '  \033[0;32m✓\033[0m %s\n' "$1"; }
ko()    { printf '  \033[0;31m✗\033[0m %s\n' "$1"; ECHECS=$((ECHECS + 1)); }
note()  { printf '      %s\n' "$1"; }

# La liste des membres du workspace, relue MAINTENANT. Une liste mise en cache avant la création
# d'un crate le fait disparaître du compte — l'erreur exacte que la première mesure de R1 a faite,
# annonçant 9 là où il y en avait 10.
cargo metadata --no-deps --format-version 1 2>/dev/null \
  | python3 -c "import json,sys;print('\n'.join(sorted(p['name'] for p in json.load(sys.stdin)['packages'])))" \
  > "$TRAVAIL/ws"

fermeture() {
    cargo tree -p "$1" -e normal --prefix none ${2:-} 2>/dev/null \
      | awk '{print $1}' | sort -u | comm -12 - "$TRAVAIL/ws"
}

titre "1. La fermeture du méta-essaim — le critère chiffré"
fermeture efe-meta-swarm-server                       > "$TRAVAIL/def"
fermeture efe-meta-swarm-server "-F core-integration"  > "$TRAVAIL/ci"
D=$(wc -l < "$TRAVAIL/def" | tr -d ' '); C=$(wc -l < "$TRAVAIL/ci" | tr -d ' ')
note "par défaut : $D crates · à core-integration : $C crates  (avant R1 : 27)"
[ "$D" -eq 8 ]  && ok "la fermeture par défaut reste à 8 — R1 n'a rien ajouté au chemin gratuit" \
                || ko "fermeture par défaut attendue 8, mesurée $D"
[ "$C" -le 10 ] && ok "core-integration tient sous le plafond de dix ($C)" \
                || ko "core-integration vaut $C, le critère exige ≤ 10"
if grep -qx 'efe-server' "$TRAVAIL/ci"; then
    ko "efe-server est ENCORE dans la fermeture servie — R1 n'a pas eu lieu"
else
    ok "efe-server a quitté la fermeture servie"
fi
grep -qx 'efe-mode1' "$TRAVAIL/ci" \
  && ok "efe-mode1 y est bien — la fonction servie n'a pas disparu avec le poids" \
  || ko "efe-mode1 est ABSENT : la fermeture a maigri en perdant ce qui la fait marcher"
note "ce qui reste : $(tr '\n' ' ' < "$TRAVAIL/ci")"

titre "2. La feuille est-elle restée une feuille ?"
fermeture efe-mode1 > "$TRAVAIL/leaf"
L=$(wc -l < "$TRAVAIL/leaf" | tr -d ' ')
note "fermeture de efe-mode1 : $L crates du dépôt — $(tr '\n' ' ' < "$TRAVAIL/leaf")"
[ "$L" -eq 2 ] \
  && ok "deux crates : lui-même et efe-core (dont la fermeture propre vaut 1)" \
  || ko "attendu 2, mesuré $L — une dépendance du dépôt s'est ajoutée, la fermeture va repartir"

titre "3. Le lien qui avait tout attaché ne s'est pas reformé"
# Le contrôle porte sur le MANIFESTE, pas sur une recherche de texte : la prose de ce crate
# explique d'où il vient et cite `efe_server` plusieurs fois, et une cible de journalisation
# conserve délibérément l'ancien nom (cf. mode1.rs). Chercher la chaîne aurait crié sur les
# commentaires qui rendent le déménagement compréhensible — un garde qui punit l'explication.
if grep -qE '^efe-server' crates/efe-mode1/Cargo.toml 2>/dev/null; then
    ko "efe-mode1 DÉCLARE efe-server — la feuille n'en est plus une"
else
    ok "le manifeste de efe-mode1 ne déclare pas efe-server"
fi
if grep -nE '^[[:space:]]*use[[:space:]]+efe_server' crates/efe-mode1/src/*.rs 2>/dev/null; then
    ko "un `use efe_server` est réapparu dans le code de la feuille"
else
    ok "aucun import de efe_server dans le code de la feuille"
fi
N=$(grep -c 'use crate::' crates/efe-mode1/src/mode1.rs || true)
note "mode1.rs : $N « use crate:: » (avant R1 : 1, qui pointait vers efe-server::services)"
[ "$N" -eq 1 ] && ok "l'unique import interne pointe maintenant DANS la feuille" \
              || ko "attendu 1, trouvé $N"
grep -q 'use crate::backend::' crates/efe-mode1/src/mode1.rs \
  && ok 'et il pointe vers crate::backend — le déménagement, pas une réécriture' \
  || ko "l'import n'est pas celui attendu"

titre "4. M1.d — le vrai contrôleur sur gRPC, inchangé"
if cargo test -q -p efe-meta-swarm-server --features core-integration \
     --test m1d_core_serving_e2e > "$TRAVAIL/m1d" 2>&1; then
    grep -E '^test result' "$TRAVAIL/m1d" | sed 's/^/      /'
    ok "les tests d'acceptation de M1.d passent"
else
    sed 's/^/      /' "$TRAVAIL/m1d" | tail -20; ko "M1.d échoue"
fi

titre "5. Ce que R1 n'a PAS fait, et qu'il faut redire"
note 'R1 a DÉPLACÉ le trait ExecuteBackend depuis efe-server ; il ne l'\''a pas UNIFIÉ.'
note "efe-core en possède déjà un de signature identique, et y implémente"
note 'ExecuteBackend for SwarmController — ce qui rend SwarmControllerBackend'
note "redondant. Le dépôt le sait et l'a écrit dans efe-core/src/swarm/backend.rs :"
note "« sa suppression est DT-C-AUDIT-CLEANUP-1 ». Unifier traverse une quarantaine"
note "de sites du chemin de requête du serveur : un autre travail, un autre risque."
if grep -q 'DT-C-AUDIT-CLEANUP-1' crates/efe-core/src/swarm/backend.rs; then
    ok "le ticket est toujours tracé à sa source (rien n'a été effacé en passant)"
else
    ko "la trace de DT-C-AUDIT-CLEANUP-1 a disparu d'efe-core"
fi

titre "Verdict"
if [ "$ECHECS" -eq 0 ]; then
    printf '  \033[0;32mR1 tient : 27 → %s crates, la feuille est une feuille, et le vrai\n  contrôleur sert toujours sur gRPC.\033[0m\n\n' "$C"
    exit 0
fi
printf '  \033[0;31m%s contrôle(s) en échec.\033[0m\n\n' "$ECHECS"
exit 1
