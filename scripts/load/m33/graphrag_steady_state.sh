#!/usr/bin/env bash
# M33 load test : sustained 200 RPS of GraphRAG search for 5 min.
# Asserts p99 < 100 ms + 5xx rate < 0.5%.

set -euo pipefail
DURATION="${DURATION:-5m}"
RPS="${RPS:-200}"
API_URL="${EFE_API_URL:?must set EFE_API_URL}"
TOKEN="${EFE_BEARER_TOKEN:?must set EFE_BEARER_TOKEN}"
COLLECTION="${EFE_COLLECTION:-loadtest}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
JS="$SCRIPT_DIR/graphrag_load.js"

cat > "$JS" <<'EOF'
import http from 'k6/http';
import { check } from 'k6';
import { Rate } from 'k6/metrics';

const TOKEN = __ENV.TOKEN;
const URL = __ENV.URL;
const COLLECTION = __ENV.COLLECTION;
const RPS = parseInt(__ENV.RPS || '200', 10);
const DUR = __ENV.DUR || '5m';

export const options = {
    scenarios: {
        steady: {
            executor: 'constant-arrival-rate',
            rate: RPS,
            timeUnit: '1s',
            duration: DUR,
            preAllocatedVUs: RPS,
            maxVUs: RPS * 2,
        }
    },
    thresholds: {
        'http_req_duration': ['p(99)<100'],
        'errors': ['rate<0.005']
    }
};

const errors = new Rate('errors');

const QV = (() => { const v = []; for (let i = 0; i < 768; i++) v.push(Math.random() - 0.5); return v; })();

export default function () {
    const headers = {
        'Authorization': `Bearer ${TOKEN}`,
        'X-Tenant-Id': 'load-test',
        'Content-Type': 'application/json'
    };
    const body = JSON.stringify({
        collection: COLLECTION,
        query_vector: QV,
        top_k: 10,
        max_hops: 2,
        alpha: 0.7,
        decay_rate: 0.5
    });
    const r = http.post(`${URL}/v1/graphrag/search`, body, { headers });
    check(r, { 'status 2xx or 4xx': r => r.status < 500 });
    errors.add(r.status >= 500);
}
EOF

echo "▶ k6 ${DURATION} @ ${RPS} rps GraphRAG"
TOKEN="$TOKEN" URL="$API_URL" COLLECTION="$COLLECTION" RPS="$RPS" DUR="$DURATION" \
    k6 run --quiet "$JS"
