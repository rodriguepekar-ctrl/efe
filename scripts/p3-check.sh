#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
#  P3 — le nettoyage, CONTRÔLÉ.
#
#  La feuille de route du 24 août donnait deux critères :
#
#    (1) `grep -c <cible abandonnée>` → 0
#    (2) « la fermeture à frontière perd 4 crates »
#
#  Le premier est tenu, et mieux que par un grep : par un GARDE que ce
#  script EXÉCUTE. Un grep mesure un instant ; un garde tient un état,
#  et ce dépôt a déjà vu trois fois un contrôle devenir faux un module
#  plus tard.
#
#  Le second est RÉFUTÉ, et ce script porte la mesure qui le réfute.
#  Ce n'est pas un renoncement : c'est le résultat. Un critère qu'on
#  découvre faux se remplace par ce qu'on peut réellement tenir, et on
#  écrit pourquoi — sinon il reparaîtra au prochain plan, identique.
#
#  Usage :  bash scripts/p3-check.sh
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

titre "1. Le nom de la cible abandonnée — le GARDE, exécuté (pas un grep affaibli)"
if cargo test -q -p audit-console --test p3_aucune_cible_abandonnee >"$TRAVAIL/g" 2>&1; then
    ok "les 4 tests du garde passent (détecteur contre-éprouvé compris)"
else
    ko "le garde échoue"; sed 's/^/      /' "$TRAVAIL/g" | tail -20
fi
# Une mesure indépendante du garde : s'ils divergeaient, l'un des deux mentirait.
RESTE=$(grep -rni 'kovant' --include='*.rs' --include='*.toml' crates/ Cargo.toml 2>/dev/null \
        | grep -vcE 'efe-meta-swarm|efe-meta-verify|verify-meta-mini' || true)
[ "$RESTE" -eq 0 ] && ok "contre-mesure indépendante : 0 occurrence hors du méta-essaim" \
                   || ko "$RESTE occurrence(s) subsistent dans le périmètre des deux produits"
note "les documents de juin gardent leur PROSE : ce sont des archives datées d'un engagement"
note "réel, et les réécrire serait falsifier un registre. Seuls leurs noms de fichier ont changé."

titre "2. Les deux fermetures, mesurées"
cargo metadata --no-deps --format-version 1 2>/dev/null \
  | python3 -c "import json,sys;print('\n'.join(sorted(p['name'] for p in json.load(sys.stdin)['packages'])))" \
  > "$TRAVAIL/ws"
mesure() { cargo tree -p efe-swarm-gateway -e normal --prefix none ${2:-} 2>/dev/null \
           | awk '{print $1}' | sort -u | comm -12 - "$TRAVAIL/ws" > "$1"; }
mesure "$TRAVAIL/defaut"
mesure "$TRAVAIL/frontiere" "-F frontier-llm"
D=$(wc -l < "$TRAVAIL/defaut" | tr -d ' '); F=$(wc -l < "$TRAVAIL/frontiere" | tr -d ' ')
note "par défaut : $D crates · à frontière : $F crates"
[ "$D" -eq 14 ] && ok "la fermeture par défaut vaut 14, comme annoncé" || ko "attendu 14, mesuré $D"
[ "$F" -eq 18 ] && ok "la fermeture à frontière vaut 18, comme annoncé" || ko "attendu 18, mesuré $F"
AJOUTS=$(comm -13 "$TRAVAIL/defaut" "$TRAVAIL/frontiere" | tr '\n' ' ')
note "ce que « frontier-llm » ajoute : $AJOUTS"
[ "$(echo "$AJOUTS" | wc -w | tr -d ' ')" -eq 4 ] \
  && ok "quatre crates ajoutés — c'est bien le chiffre que le plan visait" \
  || ko "le plan parlait de 4 crates ajoutés"

titre "3. LA RÉFUTATION — pourquoi « perdre 4 crates » est arithmétiquement impossible"
cargo tree -p efe-nano-app-coordinator -e normal --prefix none 2>/dev/null \
  | awk '{print $1}' | sort -u | comm -12 - "$TRAVAIL/ws" > "$TRAVAIL/nano"
N=$(wc -l < "$TRAVAIL/nano" | tr -d ' ')
note "le plan affirmait : « frontier-llm tire efe-nano-app-coordinator, ET AVEC LUI TROIS DE PLUS »"
note "mesuré : la fermeture propre de efe-nano-app-coordinator vaut $N crate(s) du workspace"
[ "$N" -eq 1 ] \
  && ok "il n'en traîne AUCUN autre — les « trois de plus » n'existent pas" \
  || ko "attendu 1 (lui-même), mesuré $N"
for c in efe-multi-llm provenance-capture; do
  if cargo tree -p efe-swarm-gateway -e normal -F frontier-llm -i "$c" --prefix depth 2>/dev/null \
     | grep -q '^1efe-swarm-gateway'; then
    ok "$c est tiré DIRECTEMENT par la passerelle — rien à voir avec le Cadrage"
  else
    ko "$c : la parenté attendue n'est pas celle mesurée"
  fi
done
note "reste efe-coordinator, qui EST la couche des trois providers frontière : la retirer d'une"
note "feature d'egress payant reviendrait à retirer la feature. Le maximum atteignable est −1."

titre "4. Et même ce −1 n'est pas disponible"
SITES=$(grep -rn 'efe_nano_app_coordinator' crates/efe-coordinator/src/ 2>/dev/null | wc -l | tr -d ' ')
note "efe-coordinator/src importe efe-nano-app-coordinator en $SITES endroits"
note "ce qu'il en importe : TenantId, TaskClass, Budget, LlmMetadata, SlmCircuitBreaker,"
note "RobotError, ReformulatorClient, RouterClient — son vocabulaire de types, pas le Cadrage"
[ "$SITES" -ge 30 ] \
  && ok "le crate porte le vocabulaire de types de efe-coordinator : le lien n'est pas retirable" \
  || ko "attendu ≥ 30 sites, mesuré $SITES — la conclusion mérite d'être reprise"

titre "5. CE QU'ON PEUT TENIR — le Cadrage reste mort sur le chemin payant"
if [ -z "$(grep -rn 'Compressor' crates/efe-swarm-gateway/src 2>/dev/null || true)" ]; then
    ok "la passerelle ne contient aucune mention du Cadrage"
else
    ko "le Cadrage est apparu dans la passerelle — MZ l'a déclaré TUÉ, il ne doit pas revenir"
    grep -rn 'Compressor' crates/efe-swarm-gateway/src | sed 's/^/      /'
fi
CAD=$(wc -l crates/efe-nano-app-coordinator/src/clients/compressor.rs \
             crates/efe-nano-app-coordinator/src/types/compressor.rs 2>/dev/null \
      | tail -1 | awk '{print $1}')
note "poids réel du Cadrage : $CAD lignes, liées et jamais appelées depuis la passerelle"
note "le sortir du binaire à frontière est possible (feature), mais traverse le contrat de"
note "métrologie (Phase::Compressor ∈ Phase::ALL) — ce n'est pas le « 1 jour » que P3 budgétait."

titre "Verdict"
if [ "$ECHECS" -eq 0 ]; then
    printf '  \033[0;32mP3 : le nom est parti et un garde le tient ; le critère de fermeture est\n  RÉFUTÉ par la mesure, et le Cadrage est prouvé mort sur le chemin payant.\033[0m\n\n'
    exit 0
fi
printf '  \033[0;31m%s contrôle(s) en échec.\033[0m\n\n' "$ECHECS"
exit 1
