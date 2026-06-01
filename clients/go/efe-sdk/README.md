# `efe-sdk-go` — official Go SDK for EFE

> **Sprint D23 (Φ-B)** — pure-Go client for the EFE (Essaim Fluide
> Éphémère) inference platform.
>
> **Status** : Beta (`v0.3.0` private during Φ-B). GA target
> `v1.0.0` post-D40.
> **Go version** : 1.22+
> **License** : Apache-2.0
> **Wire contract** : `efe.v1` proto family (gRPC).

---

## Install

### Private (during Φ-B)

```bash
# Configure private module access (one-time per machine).
export GOPRIVATE=github.com/efe-ai/*

# Auth via netrc OR SSH (org bot account).
cat ~/.netrc
# machine github.com login efe-ai-bot password <PAT>

go get github.com/efe-ai/efe-sdk-go@v0.3.0
```

### Public (post-GA)

```bash
go get github.com/efe-ai/efe-sdk-go@latest
```

---

## Quickstart

```go
package main

import (
    "context"
    "fmt"
    "log"
    "os"
    "time"

    efe "github.com/efe-ai/efe-sdk-go"
)

func main() {
    client, err := efe.NewClient(
        os.Getenv("EFE_ENDPOINT"),
        efe.WithJWT(os.Getenv("EFE_JWT_TOKEN")),
        efe.WithDeadline(10*time.Second),
    )
    if err != nil {
        log.Fatal(err)
    }
    defer client.Close()

    verdict, err := client.Classify(context.Background(), "I want a refund")
    if err != nil {
        log.Fatal(err)
    }
    fmt.Printf("label=%s confidence=%.2f cone=%s rounds=%d\n",
        verdict.Label, verdict.Confidence,
        verdict.Catalog, verdict.Rounds,
    )
}
```

---

## Functional options

```go
client, err := efe.NewClient(addr,
    efe.WithJWT("eyJhbGc..."),
    efe.WithMTLS("/etc/efe/cert.pem", "/etc/efe/key.pem", "/etc/efe/ca.pem"),
    efe.WithDeadline(10*time.Second),
    efe.WithRetry(3, efe.ExponentialBackoff{
        InitialInterval: 100 * time.Millisecond,
        MaxInterval:     30 * time.Second,
        Multiplier:      2.0,
    }),
    efe.WithOTel(otelTracerProvider),
    efe.WithPrometheus(promRegistry),
    efe.WithLogger(slog.Default()),
    efe.WithUserAgent("my-app/1.0"),
    efe.WithDialTimeout(5 * time.Second),
)
```

---

## Errors

The SDK exposes 10 sentinel errors — use `errors.Is` :

```go
verdict, err := client.Classify(ctx, "...")
if err != nil {
    switch {
    case errors.Is(err, efe.ErrUnauthenticated):
        // 401 — refresh token + retry
    case errors.Is(err, efe.ErrRateLimited):
        // 429 — backoff
    case errors.Is(err, efe.ErrTimeout):
        // 504 — surface timeout to caller
    case errors.Is(err, efe.ErrCatalogNotFound):
        // 404 — catalog deleted ?
    default:
        // 500
    }
}
```

`efe.IsRetryable(err)` returns `true` for the retry-eligible subset
(`ErrUnavailable`, `ErrTimeout`, `ErrRateLimited`).

---

## Three framework examples

Live under `examples/` — each is a **separate Go sub-module** so
the SDK itself does NOT pull gin/fiber/chi as transitive deps :

```
clients/go/efe-sdk/examples/
├── gin/                    # github.com/gin-gonic/gin
├── fiber/                  # github.com/gofiber/fiber/v2
└── chi/                    # github.com/go-chi/chi/v5
```

Run any of them :

```bash
cd examples/gin
EFE_ENDPOINT=staging.efe.ai:50051 \
EFE_JWT_TOKEN=<token> \
go run main.go
```

Then test :

```bash
curl -X POST http://localhost:8080/classify \
  -H "Content-Type: application/json" \
  -d '{"text": "I want a refund"}'
```

---

## Mock for unit tests

```go
import efemock "github.com/efe-ai/efe-sdk-go/mock"

func TestMyHandler(t *testing.T) {
    server, client := efemock.NewServer(t)
    server.OnClassify(func(ctx context.Context, req *pb.ExecuteRequest) (*pb.ExecuteResponse, error) {
        return efemock.OkResponse(efemock.NewVerdict(
            efemock.WithLabel("fraud"),
            efemock.WithConfidence(0.92),
        )), nil
    })

    handler := MyHandler{efe: client}
    // ... assert handler behaviour
}
```

The `MockEfeClient` (no gRPC, no network) is also available for
the simplest unit tests :

```go
m := efemock.NewClient()
m.SetVerdict(efemock.NewVerdict(efemock.WithLabel("fraud")))
verdict, _ := m.Classify(ctx, "...")
require.Equal(t, "fraud", verdict.Label)
```

---

## Development

```bash
# 1. Generate gRPC stubs from protos/ (one-time + on proto changes).
make gen-proto

# 2. Build.
make build

# 3. Test (race detector + coverage).
make test
make cover

# 4. Lint.
make lint

# 5. Build all 3 examples.
make examples
```

---

## SLOs

| ID | Target |
|:--:|---|
| `SLO-GO-1` | `NewClient` cold start < 50 ms |
| `SLO-GO-2` | `Classify` p99 ≤ server p99 + 50 ms overhead |
| `SLO-GO-3` | < 50 MiB RSS under 1 000 RPS / 1 h |
| `SLO-GO-4` | Coverage non-generated ≥ 80 % |
| `SLO-GO-5` | `golangci-lint run` zero warnings |
| `SLO-GO-6` | Go ≥ 1.22 supported |
| `SLO-GO-7` | godoc clean on pkg.go.dev |

---

## References

* [`docs/EFE_GO_SDK_EXTENSION_ARCHITECTURE.md`](../../../docs/EFE_GO_SDK_EXTENSION_ARCHITECTURE.md)
  — sprint D23 architecture (~2 040 LOC).
* [`protos/efe/v1/`](../../../protos/efe/v1/) — wire contract.
* [`docs/EFE_PYTHON_SDK_EXTENSION_ARCHITECTURE.md`](../../../docs/EFE_PYTHON_SDK_EXTENSION_ARCHITECTURE.md)
  — sister SDK (Python, M8).
