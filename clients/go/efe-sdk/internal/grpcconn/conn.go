// Package grpcconn houses connection helpers consumed by the SDK.
// Internal — NOT exported through the public surface.
//
// Sprint D23 (Φ-B) — currently exposes two helpers used by `client.go` :
//
//   - StripScheme    : normalise dial target (strip `http://` / `grpc://` / ...)
//   - DefaultKeepalive : canonical keepalive policy
//
// **Audit fix M9-A5** — earlier scaffolding shipped 3 additional
// helpers (`ResolveCredentials`, `BufconnDialer`, `EnsureNonZeroDialer`)
// that ended up unused once `client.go` consolidated TLS resolution
// in `tls.go`. Removed to satisfy `golangci-lint -E unused`.
package grpcconn

import (
	"strings"
	"time"

	"google.golang.org/grpc/keepalive"
)

// StripScheme normalises addr to the host:port shape grpc.NewClient
// accepts. Strips http:// / https:// / grpc:// / passthrough: prefixes.
func StripScheme(addr string) string {
	for _, prefix := range []string{
		"http://", "https://", "grpc://", "grpcs://",
		"passthrough:",
	} {
		if strings.HasPrefix(strings.ToLower(addr), prefix) {
			return addr[len(prefix):]
		}
	}
	return addr
}

// DefaultKeepalive is the canonical keepalive policy : ping every
// 30s, fail after 10s if no ack. Tuned for Lettre E gRPC stack.
func DefaultKeepalive() keepalive.ClientParameters {
	return keepalive.ClientParameters{
		Time:                30 * time.Second,
		Timeout:             10 * time.Second,
		PermitWithoutStream: false,
	}
}
