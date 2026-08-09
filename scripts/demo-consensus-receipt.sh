#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
#  scripts/demo-consensus-receipt.sh
#
#  EFE · Couche 3 — la démo d'une commande : une ronde de consensus
#  SERVIE, son reçu nominatif, et sa re-preuve par un tiers.
#
#  Déroulé :
#    1. une ronde passe par le chemin gRPC réellement servi ;
#    2. elle rend un reçu en schéma v4 — qui NOMME ses agents et sa
#       règle d'agrégation, là où la v3 ne scellait que des positions ;
#    3. un binaire à l'arbre de dépendances VIDE le re-prouve ;
#    4. le même binaire REFUSE une variante dont une identité a été
#       réécrite — parce qu'un contrôle qui ne peut pas échouer ne
#       prouve rien.
#
#  Usage :
#    bash scripts/demo-consensus-receipt.sh
#
#  Sortie 0 si la démo se déroule comme annoncé (re-preuve OK **et**
#  falsification refusée) ; ≥ 1 sinon.
# ─────────────────────────────────────────────────────────────────────
set -euo pipefail

cd "$(dirname "$0")/.."
BUNDLE="demo-bundles/meta-consensus-served"
# La clé de DÉVELOPPEMENT — publique, donc le reçu de cette démo est
# FORGEABLE. C'est acceptable pour démontrer le mécanisme, et il faut le
# dire : un reçu de production est scellé sous une clé tenant dérivée
# d'un master côté serveur, que personne d'autre ne détient.
KEY="4f849020850db9409ad503b209412186e588114247a023ad70b128b0acfb560d"

echo "═══ 1/3 · la ronde servie, et son reçu ═══"
cargo run -q --release -p efe-meta-swarm-server --example consensus_receipt_demo

echo
echo "═══ 2/3 · le tiers re-prouve (binaire zéro-dépendance) ═══"
cargo build -q --release -p verify-meta-mini
VERIFY="./target/release/verify_meta_mini"

# `cargo tree` doit montrer verify-meta-mini SEUL : c'est ce qui donne son
# sens à « re-prouvé sans nous faire confiance ». Si cette ligne échoue un
# jour, la promesse est cassée et la démo doit s'arrêter là.
DEPS=$(cargo tree -p verify-meta-mini -e normal --prefix none 2>/dev/null | grep -cv '^verify-meta-mini' || true)
if [ "$DEPS" -ne 0 ]; then
  echo "ÉCHEC : verify-meta-mini n'est plus zéro-dépendance ($DEPS dépendance(s))." >&2
  exit 1
fi
echo "   arbre de dépendances du vérifieur : VIDE (vérifié à l'instant)"
echo

if ! "$VERIFY" "$BUNDLE/meta_steps.jsonl" "$BUNDLE/meta_manifest.json" --key-hex "$KEY"; then
  echo "ÉCHEC : le reçu servi ne se re-prouve pas." >&2
  exit 1
fi

echo
echo "═══ 3/3 · et il REFUSE une identité réécrite ═══"
# Le cas que le schéma v4 rend détectable : la chaîne des pas est
# parfaitement intacte, seul un NOM a changé dans le manifeste. Sans le
# recoupement de liaison, ce bundle passerait — et l'auditeur lirait
# « acme-consulting » là où « supplier-scoring » avait voté.
if "$VERIFY" "$BUNDLE/meta_steps.jsonl" "$BUNDLE/meta_manifest.FORGED.json" --key-hex "$KEY"; then
  echo "ÉCHEC : une identité falsifiée a été ACCEPTÉE — le contrôle ne mord pas." >&2
  exit 1
fi

echo
echo "═══ DÉMO OK ═══"
echo "  · le reçu d'une ronde servie se re-prouve hors-ligne, sans nous ;"
echo "  · il nomme ses agents et sa règle ;"
echo "  · réécrire l'un des deux le fait REFUSER."
echo
echo "Ce qu'il atteste : l'INTÉGRITÉ de la suite de pas — jamais la CORRECTION"
echo "de la décision de consensus. Et il est tamper-evident pour un DÉTENTEUR"
echo "DE CLÉ, pas trustless pour un tiers : la re-preuve exige la clé du tenant."
