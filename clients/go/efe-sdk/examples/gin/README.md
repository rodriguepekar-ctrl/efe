# `efe-sdk-go` — gin example

Demonstrates the canonical EFE SDK integration in a [gin](https://gin-gonic.com)
HTTP handler.

## Run

```bash
EFE_ENDPOINT=staging.efe.ai:50051 \
EFE_JWT_TOKEN=<token> \
go run main.go
```

## Endpoints

* `POST /classify` — body `{"text": "..."}`. Returns the SDK Verdict
  as JSON, status 200 on success or 4xx/5xx mapped from the SDK
  sentinel errors (see `efeErrorStatus` in `main.go`).
* `GET /healthz` — probes the EFE server's gRPC Health endpoint.
  Returns 200 on `SERVING`, 503 otherwise.

## Test

```bash
curl -X POST http://localhost:8080/classify \
    -H 'Content-Type: application/json' \
    -d '{"text": "I want a refund"}'

curl http://localhost:8080/healthz
```

## Layout

This is a **separate Go sub-module** (`go.mod`) from the main SDK.
Adding the SDK to your project does NOT pull `gin` ; you opt-in
explicitly when you copy this example.

## See also

* [SDK README](../../README.md) — quickstart + full API surface.
* [`fiber` example](../fiber/) — same handler, fasthttp-based.
* [`chi` example](../chi/) — same handler, `net/http`-compatible.
