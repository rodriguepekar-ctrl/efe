# `@efe/sdk` — Express BFF example

Demonstrates the canonical EFE SDK integration in an Express
back-end-for-frontend that exposes a thin REST API on top of the
gRPC-based EFE service.

## Run

```bash
pnpm install
EFE_ENDPOINT=staging.efe.ai:50051 \
EFE_JWT_TOKEN=<token> \
pnpm dev
```

## Endpoints

* `POST /classify` — body `{ "text": "…" }`. Returns the SDK Verdict
  as JSON, status 200 on success or 4xx/5xx mapped from the SDK
  sentinels (`efeErrorStatus()`).
* `GET /healthz` — probes the EFE server's gRPC Health endpoint.
  Returns 200 on `SERVING`, 503 otherwise.

## Test

```bash
curl -X POST http://localhost:3000/classify \
  -H 'Content-Type: application/json' \
  -d '{"text": "I want a refund"}'

curl http://localhost:3000/healthz
```

## See also

* [SDK README](../../README.md)
* [`react` example](../react/) — frontend
* [`vue` example](../vue/) — frontend
