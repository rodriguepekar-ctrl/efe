#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
#  Scella — le reçu servi, SANS CLÉ.
#
#  Aucun compte, aucune carte, aucune clé d'API, aucun octet qui sort.
#  Tout tourne sur cette machine.
#
#  Déroulé :
#    1. la passerelle sert un reçu et le re-prouve ;
#    2. une falsification est refusée — et LOCALISÉE ;
#    3. le vérifieur tiers a un arbre de dépendances VIDE, donc un
#       auditeur peut lire sa source en entier avant de le croire ;
#    4. et une TRONCATURE est démasquée par l'ancre, avec un simple
#       COMPTAGE — aucune clé, aucune cryptographie.
#
#  Usage :
#    bash scripts/demo-local-receipt.sh
#
#  Sortie 0 si la démo se déroule comme annoncé ; ≥ 1 sinon.
# ─────────────────────────────────────────────────────────────────────
set -euo pipefail

cd "$(dirname "$0")/.."
OUT="demo-bundles/local-receipt"

echo "═══ 1/4 · la passerelle, en local, sans clé ═══"
# On compile d'abord, en gardant la sortie de côté : les avertissements du compilateur
# n'ont rien à faire au milieu d'une démonstration, mais une ERREUR doit rester visible.
BUILD_LOG=$(mktemp)
trap 'rm -f "$BUILD_LOG"' EXIT
if ! cargo build -q --release -p efe-swarm-gateway \
       --example local_receipt_demo --features demo 2>"$BUILD_LOG"; then
  echo "ÉCHEC : la compilation de la démo a échoué." >&2
  cat "$BUILD_LOG" >&2
  exit 1
fi
cargo run -q --release -p efe-swarm-gateway \
  --example local_receipt_demo --features demo 2>/dev/null

echo
echo "═══ 2/4 · le vérifieur tiers n'a AUCUNE dépendance ═══"
# C'est ce qui donne son sens à « re-prouvé sans nous faire confiance » : un auditeur
# qui doit accorder sa confiance à un vérifieur peut d'abord en lire la TOTALITÉ.
# Si cette ligne échoue un jour, la promesse est cassée et la démo doit s'arrêter là.
DEPS=$(cargo tree -p verify-mini -e normal --prefix none 2>/dev/null | grep -cv '^verify-mini' || true)
if [ "$DEPS" -ne 0 ]; then
  echo "ÉCHEC : verify-mini a $DEPS dépendance(s) — l'auditeur ne peut plus tout lire." >&2
  exit 1
fi
echo "  cargo tree -p verify-mini -e normal  →  verify-mini SEUL (0 dépendance)"
echo "  ↑ la source entière du vérifieur tient sous les yeux d'un auditeur."

echo
echo "═══ 3/4 · l'ancre : la troncature démasquée SANS CLÉ ═══"
# Le piège : le préfixe d'une chaîne scellée valide est lui-même valide. Un bundle
# tronqué se re-prouve PARFAITEMENT — rien, à l'intérieur, ne trahit la coupure.
# Le seul témoin est hors du bundle : l'ancre, publiée à l'export, qui annonce le
# nombre de nœuds. Le contrôle est un COMPTAGE : ni clé, ni crypto, ni confiance.
ANCHOR="$OUT/anchors.log"
[ -s "$ANCHOR" ] || { echo "ÉCHEC : aucune ancre publiée." >&2; exit 1; }

ANNONCE=$(awk -F'\t' 'END {print $5}' "$ANCHOR")
INTACT=$(grep -c '[^[:space:]]' "$OUT/records.sealed.jsonl")
TRONQUE=$(grep -c '[^[:space:]]' "$OUT/records.TRUNCATED.jsonl")

echo "  l'ancre annonce            : $ANNONCE nœud(s)"
echo "  le bundle intact contient  : $INTACT   → concorde"
echo "  le bundle tronqué contient : $TRONQUE   → CONTREDIT l'ancre"

if [ "$INTACT" != "$ANNONCE" ]; then
  echo "ÉCHEC : le bundle intact ne concorde pas avec son ancre." >&2
  exit 1
fi
if [ "$TRONQUE" = "$ANNONCE" ]; then
  echo "ÉCHEC : la troncature n'a pas été détectée." >&2
  exit 1
fi
echo "  ↑ un auditeur vient de détecter une troncature avec 'grep -c' et 'awk'."

echo
echo "═══ 4/4 · ce qui demande la clé, et ce qui ne la demande pas ═══"
ROOT=$(awk -F'\t' 'END {print $4}' "$ANCHOR")
echo "  SANS clé  : le comptage ci-dessus, et la concordance de la racine publiée."
echo "  AVEC clé  : la re-preuve complète, que le tenant fait chez lui —"
echo "     verify-mini $OUT/records.sealed.jsonl \\"
echo "       --key-hex <clé tenant> --root $ROOT"
echo "  La clé n'est écrite nulle part dans cette démo : en production elle vient du"
echo "  responsable de traitement, pas d'un dossier de démonstration."

echo
echo "═══ DÉMO OK ═══"
echo "  · la passerelle a tourné en LOCAL, sans clé d'API, sans compte, sans egress ;"
echo "  · le reçu s'est re-prouvé, et une falsification a été refusée ET localisée ;"
echo "  · le vérifieur tiers est lisible en entier (0 dépendance) ;"
echo "  · une troncature a été démasquée par un COMPTAGE, sans aucune clé."
echo
echo "Ce que le reçu atteste : l'INTÉGRITÉ de ce qui est enregistré — jamais la"
echo "CORRECTION de la décision. Et il est tamper-evident pour un DÉTENTEUR DE CLÉ,"
echo "pas trustless pour un tiers. Le run de cette démo est SYNTHÉTIQUE : il démontre"
echo "le mécanisme, pas la substance. Le chemin avec un vrai modèle local existe"
echo "(selfhost_e2e_real, exige 'ollama serve')."
