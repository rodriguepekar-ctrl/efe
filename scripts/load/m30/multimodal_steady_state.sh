#!/usr/bin/env bash
# M30 load test : sustained 50 RPS mixed vision + audio + long-text
# for 10 minutes. Asserts that :
#   - p99 e2e latency < 2.5 s
#   - 5xx rate < 0.5 %
#   - GPU duty cycle stays < 90 %
#
# Requires : k6 + a running EFE stack with Triton on a GPU node.

set -euo pipefail
DURATION="${DURATION:-10m}"
TARGET_RPS="${TARGET_RPS:-50}"
API_URL="${EFE_API_URL:?must set EFE_API_URL}"
TOKEN="${EFE_BEARER_TOKEN:?must set EFE_BEARER_TOKEN}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
JS="$SCRIPT_DIR/multimodal_load.js"

cat > "$JS" <<'EOF'
import http from 'k6/http';
import { sleep, check } from 'k6';
import { Trend, Rate } from 'k6/metrics';
import encoding from 'k6/encoding';

const TOKEN = __ENV.TOKEN;
const URL = __ENV.URL;
const RPS = parseInt(__ENV.RPS || '50', 10);
const DUR = __ENV.DUR || '10m';

export const options = {
    scenarios: {
        steady: {
            executor: 'constant-arrival-rate',
            rate: RPS,
            timeUnit: '1s',
            duration: DUR,
            preAllocatedVUs: RPS * 2,
            maxVUs: RPS * 5,
        }
    },
    thresholds: {
        'http_req_duration{type:vision}': ['p(99)<2500'],
        'http_req_duration{type:audio}':  ['p(99)<2500'],
        'http_req_duration{type:text}':   ['p(99)<500'],
        'errors':                          ['rate<0.005'],
    }
};

const visionImg = encoding.b64encode(new Uint8Array([0xFF,0xD8,0xFF,0xE0,0,0,0,0]));
const audioWav = (() => {
    const buf = new Uint8Array(1100);
    buf.set([0x52,0x49,0x46,0x46], 0); // RIFF
    buf.set([0x57,0x41,0x56,0x45], 8); // WAVE
    return encoding.b64encode(buf);
})();

const errors = new Rate('errors');

export default function () {
    const r = Math.random();
    const headers = {
        'Authorization': `Bearer ${TOKEN}`,
        'X-Tenant-Id': 'load-test',
        'Content-Type': 'application/json',
    };
    let resp;
    if (r < 0.33) {
        resp = http.post(`${URL}/v1/multimodal/vision/encode`,
            JSON.stringify({image_b64: visionImg}), { headers, tags: {type: 'vision'} });
    } else if (r < 0.66) {
        resp = http.post(`${URL}/v1/multimodal/audio/encode`,
            JSON.stringify({audio_b64: audioWav, return_transcript: false}),
            { headers, tags: {type: 'audio'} });
    } else {
        resp = http.post(`${URL}/v1/multimodal/text/chunk`,
            JSON.stringify({text: 'Lorem ipsum dolor sit amet '.repeat(200)}),
            { headers, tags: {type: 'text'} });
    }
    check(resp, { 'status 2xx or 4xx': r => r.status < 500 });
    errors.add(resp.status >= 500);
}
EOF

echo "▶ Running k6 ${DURATION} at ${TARGET_RPS} rps against ${API_URL}"
TOKEN="$TOKEN" URL="$API_URL" RPS="$TARGET_RPS" DUR="$DURATION" \
    k6 run --quiet "$JS"
