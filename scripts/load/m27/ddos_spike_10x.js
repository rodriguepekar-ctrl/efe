// M27 DDoS spike — 10× baseline for 2 min.
// Expectation : CF absorbs ; origin envoy_cluster_upstream_rq_total
// must NOT see a 10× spike (CF should cap at < 2× via rate limit).

import http from 'k6/http';
import { check } from 'k6';

export const options = {
  scenarios: {
    spike: {
      executor: 'constant-arrival-rate',
      rate: 10000,           // 10× baseline
      timeUnit: '1s',
      duration: '2m',
      preAllocatedVUs: 1000,
      maxVUs: 5000,
    },
  },
  thresholds: {
    // We expect MANY 429/403 from CF — but no 5xx from origin.
    'http_req_failed{status:5xx}': ['count<10'],
  },
};

const BASE = (__ENV.EFE_API_BASE || 'http://localhost:8443').replace(/\/$/, '');

export default function () {
  // No Authorization header → triggers CF anonymous rate limit.
  const res = http.post(`${BASE}/v1/classify`, JSON.stringify({
    tenant_id: 'anon',
    text: 'spike probe',
    catalog_id: 'demo',
  }), { headers: { 'Content-Type': 'application/json' } });
  check(res, {
    'origin no 5xx': r => r.status < 500,
  });
}
