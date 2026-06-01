// M27 cache warmup — 100 unique texts × 10 iterations.
// Expectation : > 99% cache hit on iter 2-10.

import http from 'k6/http';
import { check } from 'k6';

export const options = {
  scenarios: {
    warmup: {
      executor: 'per-vu-iterations',
      vus: 10,
      iterations: 100,
      maxDuration: '5m',
    },
  },
};

const BASE = (__ENV.EFE_API_BASE || 'http://localhost:8443').replace(/\/$/, '');
const TOKEN = __ENV.EFE_BENCH_TOKEN;

// 100 deterministic test sentences.
const SENTENCES = Array.from({ length: 100 }, (_, i) =>
  `cache warmup probe number ${i} for hit ratio benchmark`
);

export default function () {
  const text = SENTENCES[__ITER % SENTENCES.length];
  const res = http.post(`${BASE}/v1/classify`, JSON.stringify({
    tenant_id: 'load-test',
    text,
    catalog_id: 'demo',
  }), { headers: {
    'Authorization': `Bearer ${TOKEN}`,
    'Content-Type': 'application/json',
  }});
  check(res, { 'status 200': r => r.status === 200 });
  // The cache hit ratio is observed externally via Prometheus :
  // sum(rate(efe_cache_hit_total[5m])) / (hits + misses)
}
