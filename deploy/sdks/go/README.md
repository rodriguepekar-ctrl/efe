# EFE Client — Go

Thin Go client for the EFE gRPC service.

## Install

```
go get github.com/efe-project/efe-client-go@latest
```

## Quickstart

See `example/main.go` for a self-contained example. Essentials:

```go
import (
    "context"
    "crypto/tls"
    "log"
    "os"

    "google.golang.org/grpc"
    "google.golang.org/grpc/credentials"
    "google.golang.org/grpc/metadata"

    efev1 "github.com/efe-project/efe-client-go/efe/mode1/v1"
)

func main() {
    endpoint := os.Getenv("EFE_ENDPOINT")          // e.g. efe.platform.svc:50051
    token    := os.Getenv("EFE_JWT")               // Bearer token
    caPath   := os.Getenv("EFE_TLS_CA")            // path to CA PEM

    creds, err := credentials.NewClientTLSFromFile(caPath, "")
    if err != nil {
        log.Fatalf("tls: %v", err)
    }

    conn, err := grpc.Dial(endpoint, grpc.WithTransportCredentials(creds))
    if err != nil {
        log.Fatalf("dial: %v", err)
    }
    defer conn.Close()

    cli := efev1.NewMode1ServiceClient(conn)

    ctx := metadata.AppendToOutgoingContext(context.Background(),
        "authorization", "Bearer "+token,
    )

    resp, err := cli.Classify(ctx, &efev1.ClassifyRequest{
        Text:       "My credit card was charged without consent",
        Catalog:    "support_triage",
        TenantId:   "team-support",
        RequestId:  "example-1",
    })
    if err != nil {
        log.Fatalf("classify: %v", err)
    }
    log.Printf("label=%s confidence=%.3f zone=%s",
        resp.Label, resp.Confidence, resp.Zone)
}
```

## Regenerating stubs

From the repo root:

```
bash deploy/proto-gen/scripts/gen-go.sh
```

## Versioning

The SDK is pinned to a proto version (`efe.mode1.v1`). A new major proto
version ships as a new package path (`efe.mode1.v2`) — both can coexist.

## Retry policy

Use a standard `grpc-go` retry interceptor with these defaults:

```go
opts := []grpc.DialOption{
    grpc.WithUnaryInterceptor(retry.UnaryClientInterceptor(
        retry.WithBackoff(retry.BackoffExponentialWithJitter(200*time.Millisecond, 0.2)),
        retry.WithMax(3),
        retry.WithCodes(codes.Unavailable, codes.DeadlineExceeded),
    )),
}
```

Do **not** retry on `InvalidArgument`, `PermissionDenied`, `NotFound`,
`Unauthenticated` — these are client bugs.

## Error mapping

| gRPC code             | Action                                      |
|-----------------------|---------------------------------------------|
| `OK`                  | process `resp`                              |
| `UNAVAILABLE`         | retry with backoff                          |
| `DEADLINE_EXCEEDED`   | retry; consider increasing budget client-side |
| `RESOURCE_EXHAUSTED`  | backoff; respect `retry-after` metadata     |
| `INVALID_ARGUMENT`    | fix caller                                  |
| `UNAUTHENTICATED`     | renew token                                 |
| `PERMISSION_DENIED`   | check JWT scopes                            |
| `FAILED_PRECONDITION` | transient catalog reload; retry             |
| `INTERNAL` / `UNKNOWN`| escalate to EFE platform                    |
