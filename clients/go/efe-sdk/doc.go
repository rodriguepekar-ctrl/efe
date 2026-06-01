// Package efe is the official Go SDK for the EFE inference platform.
//
// # Quickstart
//
// Install the module (private during Φ-B, public at v1.0.0 GA) :
//
//	go get github.com/efe-ai/efe-sdk-go@v0.3.0
//
// Use the client :
//
//	package main
//
//	import (
//	    "context"
//	    "fmt"
//	    "log"
//	    "os"
//	    "time"
//
//	    efe "github.com/efe-ai/efe-sdk-go"
//	)
//
//	func main() {
//	    client, err := efe.NewClient(
//	        os.Getenv("EFE_ENDPOINT"),
//	        efe.WithJWT(os.Getenv("EFE_JWT_TOKEN")),
//	        efe.WithDeadline(10*time.Second),
//	    )
//	    if err != nil { log.Fatal(err) }
//	    defer client.Close()
//
//	    verdict, err := client.Classify(context.Background(), "I want a refund")
//	    if err != nil { log.Fatal(err) }
//	    fmt.Printf("label=%s confidence=%.2f\n",
//	        verdict.Label, verdict.Confidence)
//	}
//
// # Public surface
//
// The package exposes the following stable types + functions :
//
//   - [EfeClient] — main client struct, opaque (use [NewClient]).
//   - [Verdict] — classification verdict (immutable conventionally).
//   - [StreamItem] — one verdict in a [EfeClient.ClassifyStream].
//   - [ClientOption] — functional option type.
//   - 13 [WithXxx] options : auth, TLS, OTel, Prom, retry, etc.
//   - 10 sentinel errors : [ErrUnauthenticated], [ErrTimeout], etc.
//   - [IsRetryable] — error inspection helper.
//   - [ErrorCode] — sentinel → short code label (for metrics).
//
// # Soft dependencies
//
// OpenTelemetry tracing and Prometheus metrics are SOFT dependencies :
// the SDK no-ops them by default. Opt in explicitly :
//
//	client, _ := efe.NewClient(addr,
//	    efe.WithOTel(otelTracerProvider),
//	    efe.WithPrometheus(promRegistry),
//	)
//
// # Frameworks
//
// Three reference integrations live under examples/ :
//
//   - gin   : github.com/efe-ai/efe-sdk-go/examples/gin
//   - fiber : github.com/efe-ai/efe-sdk-go/examples/fiber
//   - chi   : github.com/efe-ai/efe-sdk-go/examples/chi
//
// Each is a SEPARATE Go sub-module — adding the SDK to your project
// does NOT pull gin/fiber/chi as transitive deps.
//
// # Mock for tests
//
// The mock/ sub-package ships an in-process gRPC mock server
// (backed by [google.golang.org/grpc/test/bufconn]) :
//
//	import efemock "github.com/efe-ai/efe-sdk-go/mock"
//
//	server, client := efemock.NewServer(t)
//	server.OnClassify(func(ctx context.Context, req *pb.ExecuteRequest) (*pb.ExecuteResponse, error) {
//	    return &pb.ExecuteResponse{ Output: ...}, nil
//	})
//	verdict, err := client.Classify(ctx, "...")
//
// # Versioning
//
// Aligned with the EFE platform's semver. Pre-GA tags are private
// (`v0.x` on the github.com/efe-ai org) ; v1.0.0 publishes to the
// global Go module proxy. Backward-compatible additions = minor
// bumps ; breaking changes = major bumps with deprecation cycle.
//
// Cf. docs/EFE_GO_SDK_EXTENSION_ARCHITECTURE.md.
package efe
