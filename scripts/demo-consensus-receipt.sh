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
#       prouve rien ;
#    5. et une TRONCATURE, que la re-preuve NE VOIT PAS (le préfixe
#       d'une chaîne HMAC valide est lui-même valide), est démasquée
#       par la racine publiée — SANS AUCUNE CLÉ.
#
#  Usage :
#    bash scripts/demo-consensus-receipt.sh          # participants MOCK
#    bash scripts/demo-consensus-receipt.sh --real   # participants efe-core RÉELS
#
#  Le mode --real fait tourner de VRAIS participants efe-core : le reçu
#  affiche alors trois `config_hash` réels — « cet agent, dans cette
#  version précise, a contribué ceci » — et chaque participant passe le
#  canari H1 de boot (fail-closed). Il exige la feature
#  `core-integration`, qui tire efe-core + efe-server : comptez quelques
#  minutes au premier build, quelques secondes ensuite.
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

MODE="${1:-}"
# Branche explicite plutôt que des tableaux d'arguments : sous `set -u`, bash 3.2
# (celui livré par macOS) traite un TABLEAU VIDE comme une variable NON DÉFINIE et
# avorte. Deux lignes de plus valent mieux qu'une démo qui meurt sur l'interpréteur
# par défaut de la machine où on la montre.
if [ "$MODE" = "--real" ]; then
  echo "═══ 1/4 · la ronde servie sur de VRAIS participants efe-core, et son reçu ═══"
  cargo run -q --release -p efe-meta-swarm-server --example consensus_receipt_demo \
    --features core-integration -- --engine
else
  echo "═══ 1/4 · la ronde servie (participants mock), et son reçu ═══"
  cargo run -q --release -p efe-meta-swarm-server --example consensus_receipt_demo
fi

echo
echo "═══ 2/4 · le tiers re-prouve (binaire zéro-dépendance) ═══"
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
echo "═══ 3/4 · et il REFUSE une identité réécrite ═══"
# Le cas que le schéma v4 rend détectable : la chaîne des pas est
# parfaitement intacte, seul un NOM a changé dans le manifeste. Sans le
# recoupement de liaison, ce bundle passerait — et l'auditeur lirait
# « acme-consulting » là où « supplier-scoring » avait voté.
if "$VERIFY" "$BUNDLE/meta_steps.jsonl" "$BUNDLE/meta_manifest.FORGED.json" --key-hex "$KEY"; then
  echo "ÉCHEC : une identité falsifiée a été ACCEPTÉE — le contrôle ne mord pas." >&2
  exit 1
fi

echo
echo "═══ 4/4 · l'ANCRE : ce que la clé seule NE VOIT PAS ═══"
# Le cas vicieux. On coupe un pas et on « corrige » le manifeste. Le préfixe d'une
# chaîne HMAC valide étant lui-même une chaîne valide, ce bundle se re-prouve
# PARFAITEMENT — rien, à l'intérieur, ne trahit la coupure.
echo "  a) le bundle TRONQUÉ, re-prouvé AVEC la clé :"
if ! "$VERIFY" "$BUNDLE/meta_steps.TRUNCATED.jsonl" "$BUNDLE/meta_manifest.TRUNCATED.json" \
     --key-hex "$KEY" | sed 's/^/     /'; then
  echo "ÉCHEC : le bundle tronqué aurait dû PASSER la re-preuve (c'est le fond du problème)." >&2
  exit 1
fi
echo "     ↑ vert. La chaîne EST intègre — la re-preuve ne peut pas voir une troncature."
echo
echo "  b) le MÊME bundle, confronté à l'ancre publiée — et SANS AUCUNE CLÉ :"
if "$VERIFY" "$BUNDLE/meta_steps.TRUNCATED.jsonl" "$BUNDLE/meta_manifest.TRUNCATED.json" \
     --anchor "$BUNDLE/anchors.log" 2>&1 | sed 's/^/     /'; then
  echo "ÉCHEC : une troncature a été ACCEPTÉE malgré l'ancre." >&2
  exit 1
fi
echo
echo "  c) et le bundle INTACT, toujours sans clé :"
if ! "$VERIFY" "$BUNDLE/meta_steps.jsonl" "$BUNDLE/meta_manifest.json" \
     --anchor "$BUNDLE/anchors.log" | sed 's/^/     /'; then
  echo "ÉCHEC : le bundle intact ne concorde pas avec son ancre." >&2
  exit 1
fi

echo
echo "═══ DÉMO OK ═══"
echo "  · le reçu d'une ronde servie se re-prouve hors-ligne, sans nous ;"
echo "  · il nomme ses agents et sa règle ;"
echo "  · réécrire l'un des deux le fait REFUSER ;"
echo "  · et une TRONCATURE — invisible à la re-preuve, puisque la chaîne restante"
echo "    est intègre — est démasquée par la racine publiée, SANS AUCUNE CLÉ."
echo
echo "Ce qu'il atteste : l'INTÉGRITÉ de la suite de pas — jamais la CORRECTION"
echo "de la décision de consensus. Et il est tamper-evident pour un DÉTENTEUR"
echo "DE CLÉ, pas trustless pour un tiers : la re-preuve exige la clé du tenant."
