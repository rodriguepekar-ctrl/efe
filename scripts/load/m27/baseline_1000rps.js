// M27 baseline — 1000 RPS sustained 5 min, p99 < 200ms.
// Run :
//   k6 run -e EFE_API_BASE=https://api.efe.ai \
//          -e EFE_BENCH_TOKEN=<token> \
//          scripts/load/m27/baseline_1000rps.js

import http from 'k6/http';
import { check, sleep } from 'k6';

export const options = {
  scenarios: {
    sustained: {
      executor: 'constant-arrival-rate',
      rate: 1000,
      timeUnit: '1s',
      duration: '5m',
      preAllocatedVUs: 200,
      maxVUs: 1000,
    },
  },
  thresholds: {
    'http_req_duration{status:200}': ['p(99)<200'],
    'http_req_failed': ['rate<0.01'],
  },
};

const TOKEN = __ENV.EFE_BENCH_TOKEN;
const BASE = (__ENV.EFE_API_BASE || 'http://localhost:8443').replace(/\/$/, '');

export default function () {
  const url = `${BASE}/v1/classify`;
  const payload = JSON.stringify({
    tenant_id: 'load-test',
    text: `random_${__VU}_${__ITER}_${Math.random()}`,
    catalog_id: 'demo',
  });
  const params = {
    headers: {
      'Authorization': `Bearer ${TOKEN}`,
      'Content-Type': 'application/json',
    },
  };
  const res = http.post(url, payload, params);
  check(res, {
    'status 200': r => r.status === 200,
    'p99 < 200ms': r => r.timings.duration < 200,
  });
}
