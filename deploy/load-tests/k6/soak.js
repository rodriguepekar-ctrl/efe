// Soak test — 4 hours of moderate traffic (50 RPS).
// Goals:
//   - Expose memory leaks (pod memory growing linearly).
//   - Catch cert-manager rotation issues (cert expires mid-run).
//   - Catch HPA flapping.
//
// Invocation:
//   k6 run \
//     -e EFE_HTTP_URL=https://efe-staging.internal \
//     -e EFE_TOKEN=... \
//     deploy/load-tests/k6/soak.js

import http from 'k6/http';
import { check } from 'k6';
import { Counter } from 'k6/metrics';

const errorCounter = new Counter('efe_http_errors');

const BASE   = __ENV.EFE_HTTP_URL || 'http://127.0.0.1:8080';
const TOKEN  = __ENV.EFE_TOKEN    || '';
const TENANT = __ENV.EFE_TENANT   || 'qa-team';

const QUERIES = JSON.parse(open('./classify_queries.json'));

export const options = {
  scenarios: {
    soak: {
      executor: 'constant-arrival-rate',
      rate: 50,
      timeUnit: '1s',
      duration: '4h',
      preAllocatedVUs: 50,
      maxVUs: 100,
    },
  },
  thresholds: {
    http_req_duration: ['p(95)<2000', 'p(99)<3500'],
    http_req_failed:   ['rate<0.005'],
  },
};

export default function () {
  const q = QUERIES[Math.floor(Math.random() * QUERIES.length)];
  const res = http.post(
    `${BASE}/v1/classify`,
    JSON.stringify({
      text: q, catalog: 'support_triage', tenant_id: TENANT,
      request_id: `soak-${Date.now()}-${__VU}-${__ITER}`,
    }),
    {
      headers: {
        'Content-Type': 'application/json',
        Authorization: `Bearer ${TOKEN}`,
      },
      timeout: '10s',
    },
  );
  const ok = check(res, { 'status 200': (r) => r.status === 200 });
  if (!ok) errorCounter.add(1, { status: String(res.status) });
}
