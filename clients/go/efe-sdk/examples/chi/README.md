# `efe-sdk-go` — chi example

Demonstrates the canonical EFE SDK integration in a [chi](https://github.com/go-chi/chi)
net/http-compatible router. Highlights :

* Stdlib `net/http` interfaces — works with any net/http middleware
  (incl. `otelhttp`, `promhttp`).
* `chi.Router.Use(...)` middleware pattern : `RequestID` + `RealIP`
  + `Logger` + `Recoverer` + `Timeout` wired by default.

## Run

```bash
EFE_ENDPOINT=staging.efe.ai:50051 \
EFE_JWT_TOKEN=<token> \
go run main.go
```

## Endpoints

Same as the gin / fiber examples — `POST /classify` + `GET /healthz`.

## Why chi

Lightweight (≈ 1 KB binary impact). Native `context.Context` flow
per-request, plays well with the SDK's ctx-driven cancellation.

## Layout

Separate Go sub-module — adding the SDK to your project does NOT
pull `chi`.

## See also

* [SDK README](../../README.md)
* [`gin` example](../gin/)
* [`fiber` example](../fiber/)
