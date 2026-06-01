# EFE Load Tests (k6)

All scripts in `k6/` target the HTTP/REST gateway (grpc-gateway) for
portability; pure-gRPC equivalents can be produced with
[`xk6-grpc`](https://github.com/grafana/xk6-grpc).

## Scenarios

| Script            | Duration | Peak RPS | Purpose                                     |
|-------------------|----------|----------|---------------------------------------------|
| `classify.js`     | ~8 min   | 500      | Baseline + spike + cooldown                 |
| `soak.js`         | 4 h      | 50       | Memory leaks, cert rotation, HPA flapping   |
| `admin.js`        | ~10 s    | n/a      | Admin plane smoke (register/list/get/delete)|

## Quick start

```bash
# Staging
k6 run \
  -e EFE_HTTP_URL=https://efe-staging.internal \
  -e EFE_TOKEN=$(vault read -field=token secret/efe/staging/qa-team) \
  -e EFE_TENANT=qa-team \
  -e EFE_CATALOG=support_triage \
  deploy/load-tests/k6/classify.js
```

## Nightly run in CI

Triggered by `.github/workflows/load-test.yml` every night at 02:00 UTC
against the staging cluster. Results archived under the workflow's
artefacts for 30 days.

## Interpreting results

The `classify.js` thresholds mirror the production SLO:

- `p95 < 2 s` during baseline.
- `p95 < 4 s` during 10× spike (HPA catching up).
- `error rate < 1 %` overall.

A breach blocks release unless explicitly overridden by the Platform
team with a postmortem.

## Custom queries

Edit `k6/classify_queries.json` to include domain-specific messages
for your catalog. The file is picked up at runtime (no rebuild).
