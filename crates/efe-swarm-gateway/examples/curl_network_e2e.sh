#!/usr/bin/env bash
# SPDX-License-Identifier: UNLICENSED
# E2E **Network réel** : run (mode network) → routeur frontière → provider → re-preuve.
#
# Pré-requis (2 autres terminaux) :
#   T1 (routeur, AVEC tes clés) : voir l'en-tête de litellm-config.yaml
#       export ANTHROPIC_API_KEY=...   # tes clés, dans CE terminal uniquement
#       litellm --config crates/efe-swarm-gateway/examples/litellm-config.yaml --port 4000
#   T2 (gateway, pointé sur le routeur) :
#       EFE_SWARM_ROUTER_ENDPOINT=http://127.0.0.1:4000/v1 \
#         cargo run -p efe-swarm-gateway --example serve_standalone
#   T3 (ce script) : ./crates/efe-swarm-gateway/examples/curl_network_e2e.sh
#
# Aucune clé ici. Nécessite `curl` et `jq`.
set -euo pipefail

BASE="${EFE_SWARM_GATEWAY_BASE:-http://127.0.0.1:8080}"
PROVIDER="${1:-anthropic}"   # ex: ./curl_network_e2e.sh openai

echo "==> POST ${BASE}/v1/swarm/run  (mode network : provider=${PROVIDER})"
RUN=$(curl -s -X POST "${BASE}/v1/swarm/run" \
  -H 'content-type: application/json' \
  -d "{
    \"task\": { \"text\": \"The court ruled the clause unenforceable.\", \"expected\": \"legal\" },
    \"topology\": { \"kind\": \"consensus\",
                  \"generator\": \"Classify the topic in one word from {technology,finance,health,legal}.\",
                  \"judges\": [\"Judge how well the label fits the text.\"],
                  \"k\": 3, \"pattern\": \"borda\" },
    \"budget\": { \"max_cents\": 5000 },
    \"mode\": { \"network\": { \"provider\": \"${PROVIDER}\" } }
  }")

echo "${RUN}" | jq '{
  run_id,
  verified: .run.provenance.verified,
  root: .run.provenance.root,
  backend: .run.backend,            # engine=openai_local + le modèle RÉSOLU (mapping provider→modèle)
  n17: .run.n17,                    # pont forensique par-appel (retry_count si re-tentatives)
  spent_cents: .run.budget.spent_cents
}'

RUN_ID=$(echo "${RUN}" | jq -r '.run_id')
if [ "${RUN_ID}" = "null" ] || [ -z "${RUN_ID}" ]; then
  echo "ÉCHEC : pas de run_id. Réponse brute :"; echo "${RUN}" | jq .
  echo "Pièges : 501 = EFE_SWARM_ROUTER_ENDPOINT absent ; erreur backend = routeur down / URL sans /v1 / model_name non mappé."
  exit 1
fi

echo "==> GET ${BASE}/v1/swarm/verify/${RUN_ID}  (le moat : re-preuve indépendante)"
curl -s "${BASE}/v1/swarm/verify/${RUN_ID}" \
  | jq '{ verified, chain_intact, content_intact, dag_consistent, records, traced_pct, root }'

echo "OK — run Network réel exécuté via le routeur (clés server-side), puis re-prouvé hors du chemin d'exécution."
