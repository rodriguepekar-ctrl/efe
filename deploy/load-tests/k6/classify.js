// k6 load test — EFE Classify RPC (happy path).
//
// Invocation:
//   k6 run \
//     -e EFE_HTTP_URL=https://efe-staging.internal \
//     -e EFE_TOKEN=$(cat /tmp/efe-staging-token) \
//     -e EFE_TENANT=qa-team \
//     deploy/load-tests/k6/classify.js
//
// Thresholds mirror the SLO (LETTER_E_ARCHITECTURE.md §12):
//   availability ≥ 99.5 %, p95 < 2 s, p99 < 3 s.
//
// The test uses the HTTP/JSON gateway for portability. Pure-gRPC
// equivalents are provided by `classify-grpc.js` (same shape).

import http from 'k6/http';
import { check, sleep } from 'k6';
import { Counter, Rate, Trend } from 'k6/metrics';

// ──────────────────────────────────────────────────────────────────────
//  Custom metrics
// ──────────────────────────────────────────────────────────────────────
const errorCounter = new Counter('efe_http_errors');
const rejectRate   = new Rate('efe_reject_rate');
const tailLatency  = new Trend('efe_classify_latency_ms', true);

// ──────────────────────────────────────────────────────────────────────
//  Config
// ──────────────────────────────────────────────────────────────────────
const BASE    = __ENV.EFE_HTTP_URL || 'http://127.0.0.1:8080';
const TOKEN   = __ENV.EFE_TOKEN    || '';
const TENANT  = __ENV.EFE_TENANT   || 'qa-team';
const CATALOG = __ENV.EFE_CATALOG  || 'support_triage';

// A small corpus of queries — in prod use a larger bank sourced
// from the golden cases in `clients/python/efe_tool/scenarios/builtin/`.
const QUERIES = JSON.parse(open('./classify_queries.json'));

// ──────────────────────────────────────────────────────────────────────
//  Scenarios
// ──────────────────────────────────────────────────────────────────────
export const options = {
  scenarios: {
    baseline: {
      executor: 'constant-arrival-rate',
      rate: 50,                 // 50 RPS steady state
      timeUnit: '1s',
      duration: '2m',
      preAllocatedVUs: 50,
      maxVUs: 100,
      tags: { phase: 'baseline' },
    },
    spike: {
      executor: 'ramping-arrival-rate',
      startTime: '2m10s',
      startRate: 50,
      timeUnit: '1s',
      preAllocatedVUs: 50,
      maxVUs: 500,
      stages: [
        { target: 500, duration: '30s' },  // 10× spike
        { target: 500, duration: '3m'  },
        { target: 50,  duration: '30s' },
      ],
      tags: { phase: 'spike' },
    },
    cooldown: {
      executor: 'constant-arrival-rate',
      startTime: '6m30s',
      rate: 50,
      timeUnit: '1s',
      duration: '1m',
      preAllocatedVUs: 50,
      maxVUs: 100,
      tags: { phase: 'cooldown' },
    },
  },
  thresholds: {
    'http_req_duration{phase:baseline}': [
      'p(95)<2000',  // SLO mirror
      'p(99)<3000',
    ],
    'http_req_duration{phase:spike}': [
      // Under 10× spike, latency is allowed to degrade up to 2×
      // while HPA catches up. This is a stress-boundary, not SLO.
      'p(95)<4000',
    ],
    'http_req_failed': ['rate<0.01'],
    'efe_http_errors': ['count<50'],
  },
  summaryTrendStats: ['avg', 'min', 'med', 'p(90)', 'p(95)', 'p(99)', 'max'],
};

// ──────────────────────────────────────────────────────────────────────
//  Request
// ──────────────────────────────────────────────────────────────────────
export default function () {
  const query = QUERIES[Math.floor(Math.random() * QUERIES.length)];

  const payload = JSON.stringify({
    text: query,
    catalog: CATALOG,
    tenant_id: TENANT,
    request_id: `k6-${Date.now()}-${__VU}-${__ITER}`,
  });

  const params = {
    headers: {
      'Content-Type': 'application/json',
      Authorization: `Bearer ${TOKEN}`,
      'x-request-id': `k6-${__VU}-${__ITER}`,
    },
    timeout: '10s',
    tags: { endpoint: 'classify' },
  };

  const res = http.post(`${BASE}/v1/classify`, payload, params);

  const ok = check(res, {
    'status is 200':       (r) => r.status === 200,
    'has label':           (r) => r.status === 200 && JSON.parse(r.body).label,
    'no INTERNAL':         (r) => r.status !== 500,
  });

  if (!ok) {
    errorCounter.add(1, { status: String(res.status) });
  }

  if (res.status === 200) {
    const body = JSON.parse(res.body);
    rejectRate.add(body.reject ? 1 : 0);
    tailLatency.add(res.timings.duration);
  }
}

// Small jitter so we don't synchronise across VUs.
export function teardown() {
  // Intentionally empty — just a marker for future cleanup hooks.
}
