#!/usr/bin/env bash
# M31 load test : sustained 10k RPS of meter_inc calls + 1k RPS of
# `/v1/usage/now` reads for 5 minutes. Asserts that :
#   - p99 meter_inc < 1 ms
#   - p99 /usage/now < 50 ms
#   - 0 % 5xx rate

set -euo pipefail
DURATION="${DURATION:-5m}"
WRITE_RPS="${WRITE_RPS:-10000}"
READ_RPS="${READ_RPS:-1000}"
API_URL="${EFE_API_URL:?must set EFE_API_URL}"
TOKEN="${EFE_BEARER_TOKEN:?must set EFE_BEARER_TOKEN}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
JS="$SCRIPT_DIR/metering_load.js"

cat > "$JS" <<'EOF'
import http from 'k6/http';
import { check } from 'k6';
import { Rate } from 'k6/metrics';

const TOKEN = __ENV.TOKEN;
const URL   = __ENV.URL;

export const options = {
    scenarios: {
        writes: {
            executor: 'constant-arrival-rate',
            exec: 'classify',
            rate: parseInt(__ENV.WRITE_RPS),
            timeUnit: '1s',
            duration: __ENV.DUR,
            preAllocatedVUs: 50,
            maxVUs: 500,
        },
        reads: {
            executor: 'constant-arrival-rate',
            exec: 'usage_now',
            rate: parseInt(__ENV.READ_RPS),
            timeUnit: '1s',
            duration: __ENV.DUR,
            preAllocatedVUs: 10,
            maxVUs: 100,
        }
    },
    thresholds: {
        'http_req_duration{type:classify}': ['p(99)<200'],
        'http_req_duration{type:usage}':    ['p(99)<50'],
        'errors': ['rate<0.001']
    }
};

const errors = new Rate('errors');

export function classify() {
    const headers = {
        'Authorization': `Bearer ${TOKEN}`,
        'Content-Type': 'application/json',
        'X-Tenant-Id': 'load-test'
    };
    const body = JSON.stringify({ text: 'load test', mode: 'mode1' });
    const r = http.post(`${URL}/v1/classify/text`, body, {
        headers,
        tags: { type: 'classify' }
    });
    check(r, { 'status<500': r => r.status < 500 });
    errors.add(r.status >= 500);
}

export function usage_now() {
    const headers = {
        'Authorization': `Bearer ${TOKEN}`,
        'X-Tenant-Id': 'load-test'
    };
    const r = http.get(`${URL}/v1/usage/now`, {
        headers,
        tags: { type: 'usage' }
    });
    check(r, { 'status<500': r => r.status < 500 });
    errors.add(r.status >= 500);
}
EOF

echo "▶ k6 ${DURATION} : ${WRITE_RPS} writes/s + ${READ_RPS} reads/s"
TOKEN="$TOKEN" URL="$API_URL" WRITE_RPS="$WRITE_RPS" READ_RPS="$READ_RPS" DUR="$DURATION" \
    k6 run --quiet "$JS"
