// Sprint **D23 (Φ-B)** — Go SDK for EFE.
//
// Module path is **immutable** post v1.0.0 GA. Consumers `go get`
// this path : github.com/efe-ai/efe-sdk-go@v0.3.0 (private during
// Φ-B) → @v1.0.0 (public proxy at GA).
//
// Cf. `docs/EFE_GO_SDK_EXTENSION_ARCHITECTURE.md`.
module github.com/efe-ai/efe-sdk-go

// Go 1.22 minimum : `slog` stable + `for range int` + `errors.Join`
// + better generics. v1.0.0 GA refuses anything older.
go 1.22

require (
	github.com/cenkalti/backoff/v4 v4.2.1
	github.com/prometheus/client_golang v1.18.0
	github.com/stretchr/testify v1.8.4
	go.opentelemetry.io/contrib/instrumentation/google.golang.org/grpc/otelgrpc v0.49.0
	go.opentelemetry.io/otel v1.24.0
	go.opentelemetry.io/otel/trace v1.24.0
	google.golang.org/grpc v1.62.0
	google.golang.org/protobuf v1.33.0
)

// Test-only transitive entries elided — `go mod tidy` populates
// go.sum at first build.

// Sprint D23 — placeholder retract list.
// Once shipped privately, retract any `v0.3.0-rc*` tags that
// were intermediate.
retract ()
