#!/usr/bin/env bash
# SPDX-License-Identifier: UNLICENSED
#
# M1.b — « un seul analyseur d'autorité », vérifié.
#
# Ce script ne demande pas si M1.b a été CODÉ. Il demande si les deux propriétés qui le
# définissent tiennent :
#
#   1. il n'existe plus qu'UN analyseur d'autorité dans le dépôt, exemptions VIDES ;
#   2. ce qui est déclaré local l'est aussi pour le client qui va composer l'appel.
#
# La seconde est celle qui compte. La première était déjà « satisfaite substantiellement » en
# M1.a — deux analyseurs restaient, exemptés avec cette raison écrite : « sûr aujourd'hui par
# composition ». Elle était FAUSSE, et la mesure l'a montrée. C'est pourquoi ce script contient
# une CONTRE-ÉPREUVE : il rejoue l'ancien découpage et exige qu'il échoue. Un garde dont on ne
# sait pas s'il peut tomber ne garde rien.

set -uo pipefail
cd "$(dirname "$0")/.."

ok()    { printf '  \033[32m✓\033[0m %s\n' "$*"; }
ko()    { printf '  \033[31m✗\033[0m %s\n' "$*"; exit 1; }
note()  { printf '  \033[33m·\033[0m %s\n' "$*"; }
titre() { printf '\n\033[1m%s\033[0m\n' "$*"; }

printf '\033[1mM1.b · un seul analyseur d'"'"'autorité\033[0m\n'

titre "1. Combien d'analyseurs le dépôt contient-il ?"
#
#  Le bon motif est le littéral `"://"`, pas un nom de fonction : trois des cinq analyseurs
#  historiques ne s'appelaient pas `is_local_endpoint`, donc le critère d'origine de la feuille de
#  route pouvait sortir 0 avec le différentiel entier. Les mentions en COMMENTAIRE ne comptent
#  pas — elles ne peuvent diverger de personne.
FICHIERS=$(grep -rl '"://"' --include='*.rs' crates 2>/dev/null | grep -v '^crates/scella-authority/' || true)
CODE=""
for f in $FICHIERS; do
  # une ligne de code (hors `//`) portant le motif
  if grep -vE '^\s*//' "$f" | grep -q '"://"'; then CODE="$CODE $f"; fi
done
if [ -n "${CODE// /}" ]; then
  ko "analyseur(s) hors du crate unique :$CODE"
fi
ok "un seul analyseur : \`crates/scella-authority\` (les autres mentions sont des commentaires)"

titre "2. La liste d'exemptions est-elle VIDE ? (la DoD de M1.b)"
if grep -qE '^const EXEMPTIONS: &\[\(&str, &str\)\] = &\[\];' crates/scella-authority/tests/un_seul_analyseur.rs; then
  ok "\`EXEMPTIONS\` est littéralement vide dans la source"
else
  ko "la liste d'exemptions n'est plus vide — M1.b est défait"
fi
cargo test -q -p scella-authority --test un_seul_analyseur >/dev/null 2>&1 \
  || ko "le garde du dépôt échoue"
ok "le garde s'exécute et passe (détecteur et validation d'exemption ont leurs contre-épreuves)"

titre "3. CONTRE-ÉPREUVE — le garde peut-il tomber ?"
#
#  Sans cela, §1 et §2 sont deux `grep` dont on ignore s'ils voient quoi que ce soit. On
#  réintroduit un analyseur dans un crate quelconque et on exige que le garde le refuse.
CIBLE="crates/provenance-dag/src/zz_m1b_contre_epreuve.rs"
cleanup() { rm -f "$CIBLE"; }
trap cleanup EXIT
cat > "$CIBLE" <<'RS'
// Fichier temporaire de contre-épreuve — supprimé par scripts/m1b-check.sh.
pub fn hote(e: &str) -> &str { e.split_once("://").map_or(e, |(_, r)| r) }
RS
if cargo test -q -p scella-authority --test un_seul_analyseur >/dev/null 2>&1; then
  cleanup
  ko "le garde N'A PAS vu un analyseur réintroduit — il est décoratif"
fi
cleanup
ok "un analyseur réintroduit fait TOMBER le garde"
cargo test -q -p scella-authority --test un_seul_analyseur >/dev/null 2>&1 \
  || ko "le garde reste rouge après nettoyage — le fichier temporaire n'a pas été retiré"
ok "et il redevient vert une fois le fichier retiré"

titre "4. Ce que nous déclarons local l'est-il pour le VRAI client ?"
#
#  C'est la propriété que M1.a n'avait pas mesurée. L'invariant est confronté à
#  `tonic::transport::Endpoint` — le type que `connect_with_tls` construit trois lignes après la
#  décision — sur un corpus ENGENDRÉ, pas énuméré. Y figure aussi la contre-épreuve de
#  l'invariant : l'ancien découpage doit le violer, sinon le corpus est trop pauvre.
SORTIE=$(cargo test -q -p efe-meta-swarm-server --test m1b_pas_de_degradation_silencieuse \
         -- --nocapture 2>&1)
echo "$SORTIE" | grep -qE 'test result: ok' || { echo "$SORTIE" | tail -25; ko "l'invariant échoue"; }
echo "$SORTIE" | grep -E 'invariant tenu|contre-épreuve' | sed 's/^ */    /'
ok "l'invariant tient, et l'ancien découpage le VIOLE (donc le test a des dents)"

titre "5. Les deux sites migrés répondent-ils toujours à leurs cas d'origine ?"
#
#  Les tests de PR #16 n'ont pas été réécrits : ils ont été REDIRIGÉS sur l'analyseur unique.
#  C'est ce qui en fait une preuve de non-régression et non une preuve écrite après coup.
cargo test -q -p efe-meta-swarm-server --lib -- loopback_tests pr16_extract_san m1b_ >/dev/null 2>&1 \
  || ko "un cas historique de PR #16 ne passe plus"
N=$(cargo test -p efe-meta-swarm-server --lib -- loopback_tests pr16_extract_san m1b_ 2>&1 \
    | grep -oE 'test result: ok\. [0-9]+' | grep -oE '[0-9]+' | head -1)
ok "les cas historiques passent sur l'analyseur unique (${N:-?} tests)"

titre "6. Le journal recopie-t-il encore l'URL ?"
#
#  Les formes qui trompaient l'analyseur sont précisément celles qui portent un `@` :
#  `http://user:pass@hôte/` déposait un secret dans une sortie collectée et archivée. Le geste de
#  P1 : on nomme l'HÔTE, jamais l'URL.
if grep -nE 'endpoint = %endpoint_str' crates/efe-meta-swarm-server/src/client/remote_participant.rs >/dev/null 2>&1; then
  ko "le contournement TLS journalise encore l'endpoint entier (fuite de userinfo)"
fi
grep -q 'host = scella_authority::host_of' crates/efe-meta-swarm-server/src/client/remote_participant.rs \
  || ko "le contournement TLS ne nomme pas l'hôte via l'analyseur unique"
ok "le contournement TLS nomme l'hôte, pas l'URL"

titre "7. Deux défauts trouvés en construisant le falsifieur — au-delà de M1.b"
cat <<'FIN'
    En vérifiant M1.b, deux comportements de `tonic` ont été MESURÉS, dont l'un
    était documenté à l'envers dans le dépôt :

      · `http://` AVEC `tls_config`  ⟹ TLS n'est PAS appliqué. Conserver un
        `http://` écrit par l'opérateur donnait donc du gRPC en clair avec la
        pile mTLS entièrement configurée.
      · `https://` SANS `tls_config` ⟹ la connexion échoue. Donc le
        contournement loopback d'AD18 ne pouvait pas fonctionner dès que TLS
        était configuré : `iterate.rs` composait `https://`, puis
        `connect_with_tls` retirait la configuration.

    Cause commune, et c'est celle que M1 existe pour fermer : DEUX endroits
    décidaient d'une même chose. Le schéma est désormais une fonction du
    prédicat qui décide TLS. Les deux comportements sont épinglés par
    `le_schema_et_la_configuration_tls_doivent_saccorder` : si `tonic` change,
    ce test tombe au lieu que la pile mTLS se taise.
FIN
ok "les deux mesures sont épinglées par un test, pas par un commentaire"

titre "Ce que M1.b ne prouve PAS, et qu'il faut redire"
cat <<'FIN'
    · Que `scella-authority` soit CORRECT dans l'absolu. Il est prouvé d'accord
      avec `url` (`accord_avec_url.rs`) et avec `tonic` (ici) sur des corpus
      engendrés. Un corpus n'est pas une preuve : c'est un falsifieur qui n'a
      pas encore tiré.
    · Que la production soit à l'abri d'une mauvaise configuration. Les formes
      hostiles viennent d'`EFE_META_SWARM_PARTICIPANTS`, donc d'un opérateur ou
      de l'outillage qui écrit sa ConfigMap. M1.b supprime le DOWNGRADE
      SILENCIEUX, pas la faute de frappe.
    · Que le méta-essaim soit dans le produit. Il n'y est pas — la passerelle ne
      le tire pas. M1.b ferme une dette de dépôt et un contournement TLS réel,
      pas une vulnérabilité du produit servi.
FIN

printf '\n\033[32mM1.b tient.\033[0m  (liste d'"'"'exemptions vide, invariant mesuré contre le vrai client)\n'
