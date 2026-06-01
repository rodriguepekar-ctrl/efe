// Admin plane smoke test — register + list + delete a catalog.
// Runs as a single linear scenario, not a load test.
//
// Invocation:
//   k6 run -e EFE_ADMIN_URL=https://efe-admin.internal \
//          -e EFE_ADMIN_TOKEN=$(cat /tmp/efe-admin-token) \
//          deploy/load-tests/k6/admin.js

import http from 'k6/http';
import { check, sleep } from 'k6';
import encoding from 'k6/encoding';   // PROPER base64 (fixes previous hex bug)

const BASE  = __ENV.EFE_ADMIN_URL  || 'http://127.0.0.1:8081';
const TOKEN = __ENV.EFE_ADMIN_TOKEN || '';

const CATALOG_YAML = `
apiVersion: efe.mode1/v1
kind: AnchorCatalog
metadata:
  name: k6_smoke
  version: 1.0.0
embedder:
  family: sentence-transformers
  model: paraphrase-multilingual-MiniLM-L12-v2
  dim: 384
cone_policy:
  max_projection_distance: 0.5
thresholds:
  confidence_high: 0.62
  confidence_medium: 0.50
classes:
  - name: alpha
    canonical_sentences: ["hi there"]
  - name: beta
    canonical_sentences: ["bye bye"]
`;

export const options = {
  vus: 1,
  iterations: 1,
  thresholds: { http_req_failed: ['rate==0'] },
};

export default function () {
  const headers = {
    'Content-Type': 'application/json',
    Authorization: `Bearer ${TOKEN}`,
  };

  // 1. Register — payload uses real base64 (not the hex stub that
  // the previous version emitted). The AdminService validates the
  // YAML after decoding; a malformed payload returns INVALID_ARGUMENT.
  let res = http.post(`${BASE}/admin/v1/catalogs`, JSON.stringify({
    name: 'k6_smoke', version: '1.0.0', tag: 'canary',
    yaml_content_b64: encoding.b64encode(CATALOG_YAML),
    embedder_fingerprint: 'sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2@384',
    registered_by: 'k6-smoke',
  }), { headers });
  check(res, { 'register 200': (r) => r.status === 200 });

  sleep(2);

  // 2. List — must include our catalog
  res = http.get(`${BASE}/admin/v1/catalogs`, { headers });
  check(res, {
    'list 200':              (r) => r.status === 200,
    'list contains k6_smoke': (r) => JSON.parse(r.body).catalogs.some(c => c.name === 'k6_smoke'),
  });

  // 3. Get
  res = http.get(`${BASE}/admin/v1/catalogs/k6_smoke@1.0.0`, { headers });
  check(res, { 'get 200': (r) => r.status === 200 });

  // 4. Delete
  res = http.del(`${BASE}/admin/v1/catalogs/k6_smoke@1.0.0`, null, { headers });
  check(res, { 'delete 200': (r) => r.status === 200 });
}
