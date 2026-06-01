# `efe-sdk-go` — fiber example

Demonstrates the canonical EFE SDK integration in a [fiber](https://gofiber.io)
fasthttp-based HTTP handler.

## Run

```bash
EFE_ENDPOINT=staging.efe.ai:50051 \
EFE_JWT_TOKEN=<token> \
go run main.go
```

## Endpoints

Same as the [gin example](../gin/) — `POST /classify` + `GET /healthz`.

## Why fiber

Fasthttp-based — ~2-5× higher RPS vs net/http when the bottleneck
is HTTP framing (not gRPC). Stable choice for high-RPS gateways.

## Layout

Separate Go sub-module — adding the SDK to your project does NOT
pull `fiber`.

## See also

* [SDK README](../../README.md)
* [`gin` example](../gin/)
* [`chi` example](../chi/)
