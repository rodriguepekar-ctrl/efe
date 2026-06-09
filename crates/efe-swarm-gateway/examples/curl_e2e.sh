#!/usr/bin/env bash
# SPDX-License-Identifier: UNLICENSED
# E2E de la passerelle d'essaim : run → verify → records → metrics.
# Pré-requis : `cargo run -p efe-swarm-gateway --example serve_standalone` (autre terminal).
# Aucune clé : keyless. Nécessite `curl` et `jq`.
set -euo pipefail

BASE="${EFE_SWARM_GATEWAY_BASE:-http://127.0.0.1:8080}"

echo "==> POST ${BASE}/v1/swarm/run (consensus keyless)"
RUN=$(curl -s -X POST "${BASE}/v1/swarm/run" \
  -H 'content-type: application/json' \
  -d '{
    "task": { "text": "The court ruled the clause unenforceable.", "expected": "legal" },
    "topology": { "kind": "consensus", "generator": "echo_generator",
                  "judges": ["consistency", "length_band", "j_non_empty"], "k": 5,
                  "pattern": "borda" },
    "budget": { "max_cents": 1000, "on_exceed": "stop" },
    "mode": "keyless"
  }')
echo "${RUN}" | jq '{ run_id, verified: .run.provenance.verified, root: .run.provenance.root, spent: .run.budget.spent_cents }'

RUN_ID=$(echo "${RUN}" | jq -r '.run_id')

echo "==> GET ${BASE}/v1/swarm/verify/${RUN_ID} (le moat : re-preuve indépendante)"
curl -s "${BASE}/v1/swarm/verify/${RUN_ID}" | jq '{ verified, root, chain_intact, content_intact, dag_consistent, records, traced_pct }'

echo "==> GET ${BASE}/v1/swarm/runs/${RUN_ID}/records (export hors-ligne, N premiers octets)"
curl -s "${BASE}/v1/swarm/runs/${RUN_ID}/records" | jq 'length as $n | { records: $n, first_role: .[0].agent_role }'

echo "==> GET ${BASE}/metrics (Prometheus)"
curl -s "${BASE}/metrics" | grep -E '^efe_swarm_prov_(records_total|traced_ratio|chain_breaks_total)' || true

echo "OK — run exécuté, re-prouvé, exporté, métré."
