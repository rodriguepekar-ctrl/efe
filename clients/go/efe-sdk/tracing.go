// Sprint D23 (Φ-B) — OpenTelemetry auto-instrumentation.
//
// SOFT dependency : when [WithOTel] is unused, the SDK installs
// a no-op TracerProvider (zero overhead — `otel/trace/noop`).
//
// When opted-in, the SDK delegates to `otelgrpc` for the
// boilerplate-free interceptor pair (unary + stream). The
// interceptor :
//   - Creates a span per RPC named `<service>/<method>` (e.g.
//     `efe.v1.SwarmService/Execute`).
//   - Attaches canonical attributes : `rpc.system=grpc`,
//     `rpc.service=efe.v1.SwarmService`, `rpc.method=Execute`.
//   - Propagates the trace context as gRPC metadata (W3C Trace
//     Context format).
//
// Pin tested by `test_d23_go_sdk_otel_no_op_when_unset`.

package efe

import (
	"go.opentelemetry.io/contrib/instrumentation/google.golang.org/grpc/otelgrpc"
	"go.opentelemetry.io/otel/trace"
	"google.golang.org/grpc"
)

// otelUnaryInterceptor returns the unary OTel interceptor.
//
// `tp` MUST be non-nil. The default no-op provider (set in
// `defaultOptions()`) is acceptable — `otelgrpc` handles it
// gracefully (no spans created, zero allocation).
func otelUnaryInterceptor(tp trace.TracerProvider) grpc.UnaryClientInterceptor {
	return otelgrpc.UnaryClientInterceptor(
		otelgrpc.WithTracerProvider(tp),
	)
}

// otelStreamInterceptor returns the streaming OTel interceptor.
func otelStreamInterceptor(tp trace.TracerProvider) grpc.StreamClientInterceptor {
	return otelgrpc.StreamClientInterceptor(
		otelgrpc.WithTracerProvider(tp),
	)
}
