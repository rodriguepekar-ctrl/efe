// M28 — webhook delivery burst.
// Run after seeding 1000 webhook subscriptions pointing at httpbin.
// Expect : 100 deliveries/s sustained, p99 < 5s.
import http from 'k6/http';

export const options = {
  scenarios: {
    burst: {
      executor: 'constant-arrival-rate',
      rate: 100,
      timeUnit: '1s',
      duration: '2m',
      preAllocatedVUs: 100,
      maxVUs: 500,
    },
  },
  thresholds: {
    'http_req_duration{status:200}': ['p(99)<5000'],
  },
};

const BASE = (__ENV.EFE_API_BASE || 'http://localhost:8080').replace(/\/$/, '');
const TOKEN = __ENV.EFE_BENCH_TOKEN;

export default function () {
  http.get(`${BASE}/v1/webhooks`, {
    headers: { Authorization: `Bearer ${TOKEN}` },
  });
}
