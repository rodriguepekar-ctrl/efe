// Sprint D23 (Φ-B) — Auth interceptors (JWT bearer).
//
// The auth interceptor is FIRST in the chain (before logging /
// OTel / Prom / retry) so the token is attached BEFORE any
// downstream layer sees the request. This matches the canonical
// gRPC interceptor ordering (`docs/EFE_GO_SDK_EXTENSION_ARCHITECTURE.md`
// §9.3).
//
// Trust boundary :
//   - The SDK never inspects / verifies the JWT (signature, exp, …).
//     That's the server's job.
//   - The token is held in the `options` struct, never serialised to
//     logs (the slog handler in WithLogger redacts it via the
//     `attrReplaceFn`).
//
// Pin tested by `test_d23_go_sdk_auth_redacted_in_logs`.

package efe

import (
	"context"

	"google.golang.org/grpc"
	"google.golang.org/grpc/metadata"
)

// authUnaryInterceptor returns a gRPC unary client interceptor
// that attaches `authorization: Bearer <token>` to every outgoing
// RPC's metadata.
//
// Empty token = pass-through (no metadata added). Most production
// callers SHOULD provide a token via [WithJWT] ; the empty path is
// preserved for unauthenticated mock-server tests.
func authUnaryInterceptor(token string) grpc.UnaryClientInterceptor {
	if token == "" {
		// No-op interceptor — avoid the metadata copy cost.
		return func(
			ctx context.Context,
			method string,
			req, reply any,
			cc *grpc.ClientConn,
			invoker grpc.UnaryInvoker,
			opts ...grpc.CallOption,
		) error {
			return invoker(ctx, method, req, reply, cc, opts...)
		}
	}
	bearer := "Bearer " + token
	return func(
		ctx context.Context,
		method string,
		req, reply any,
		cc *grpc.ClientConn,
		invoker grpc.UnaryInvoker,
		opts ...grpc.CallOption,
	) error {
		ctx = metadata.AppendToOutgoingContext(ctx, "authorization", bearer)
		return invoker(ctx, method, req, reply, cc, opts...)
	}
}

// authStreamInterceptor is the streaming equivalent.
func authStreamInterceptor(token string) grpc.StreamClientInterceptor {
	if token == "" {
		return func(
			ctx context.Context,
			desc *grpc.StreamDesc,
			cc *grpc.ClientConn,
			method string,
			streamer grpc.Streamer,
			opts ...grpc.CallOption,
		) (grpc.ClientStream, error) {
			return streamer(ctx, desc, cc, method, opts...)
		}
	}
	bearer := "Bearer " + token
	return func(
		ctx context.Context,
		desc *grpc.StreamDesc,
		cc *grpc.ClientConn,
		method string,
		streamer grpc.Streamer,
		opts ...grpc.CallOption,
	) (grpc.ClientStream, error) {
		ctx = metadata.AppendToOutgoingContext(ctx, "authorization", bearer)
		return streamer(ctx, desc, cc, method, opts...)
	}
}
