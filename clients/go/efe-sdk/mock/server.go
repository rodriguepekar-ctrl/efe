// Sprint D23 (Φ-B) — bufconn-based mock gRPC server.
//
// `NewServer(t)` returns :
//   - a *Server you can configure via OnClassify(fn) hooks ;
//   - a *efe.EfeClient connected to it via bufconn (no real network).
//
// Cleanup is registered on the test via t.Cleanup() — no manual
// teardown needed.
//
// Use cases :
//   - Test the FULL gRPC stack : interceptors, retry, OTel context
//     propagation, auth metadata.
//   - Simulate transient errors (Unavailable + retry) and verify
//     the SDK's retry counter.
//   - Validate end-to-end protobuf wire compatibility.

package mock

import (
	"context"
	"net"
	"testing"

	"github.com/stretchr/testify/require"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/health"
	"google.golang.org/grpc/health/grpc_health_v1"
	"google.golang.org/grpc/status"
	"google.golang.org/grpc/test/bufconn"

	efe "github.com/efe-ai/efe-sdk-go"
	pb "github.com/efe-ai/efe-sdk-go/internal/proto/efe/v1"
)

// Server is the bufconn-backed mock. Configure handlers via
// `OnClassify(fn)` ; default = Unimplemented.
//
// **Audit fix M9-A6** — pre-fix Server embedded `pb.SwarmServiceServer`
// (an interface, not a struct), which served no purpose : (a) the
// embed of an interface was nil → calling promoted methods would
// panic ; (b) Server's own Execute method shadowed the embed anyway,
// so the embed was unreachable. Removed for clarity.
type Server struct {
	listener   *bufconn.Listener
	grpcServer *grpc.Server

	onClassify func(context.Context, *pb.ExecuteRequest) (*pb.ExecuteResponse, error)
}

// (compile-time guard) — Server MUST satisfy pb.SwarmServiceServer.
var _ pb.SwarmServiceServer = (*Server)(nil)

const bufSize = 1024 * 1024

// NewServer spins a bufconn-backed gRPC server in a goroutine,
// constructs an [efe.EfeClient] connected to it, and registers
// cleanup on `t`.
//
// The returned client uses NO TLS (insecure dialer to bufconn).
// In tests for TLS-specific behaviour, use the lower-level
// [NewServerNoClient] + dial manually.
func NewServer(t testing.TB, opts ...efe.ClientOption) (*Server, *efe.EfeClient) {
	t.Helper()
	s := &Server{
		listener:   bufconn.Listen(bufSize),
		grpcServer: grpc.NewServer(),
	}
	pb.RegisterSwarmServiceServer(s.grpcServer, s)

	// Standard gRPC health service stub — always SERVING.
	healthSrv := health.NewServer()
	healthSrv.SetServingStatus("", grpc_health_v1.HealthCheckResponse_SERVING)
	grpc_health_v1.RegisterHealthServer(s.grpcServer, healthSrv)

	go func() {
		_ = s.grpcServer.Serve(s.listener)
	}()
	t.Cleanup(s.grpcServer.GracefulStop)

	// The bufconn dialer ignores the target address — we pass
	// "passthrough:bufconn" as a stable placeholder that grpc.NewClient
	// accepts.
	dialer := func(ctx context.Context, _ string) (net.Conn, error) {
		return s.listener.DialContext(ctx)
	}
	// **Audit fix M9-A1** — passing the custom dialer is sufficient ;
	// the SDK now auto-selects `insecure.NewCredentials()` when the
	// dialer is set + no explicit TLS material (since bufconn is a
	// memory pipe, NOT a TLS-capable peer). Pre-fix the mock added
	// WithInsecureSkipVerify() but that still triggered TLS handshake
	// bytes on the bufconn pipe → mock server rejected them.
	allOpts := append(
		[]efe.ClientOption{
			efe.WithCustomDialer(dialer),
		},
		opts...,
	)
	client, err := efe.NewClient("passthrough:bufconn", allOpts...)
	require.NoError(t, err, "build mock-connected EfeClient")
	t.Cleanup(func() { _ = client.Close() })

	return s, client
}

// OnClassify sets the handler invoked on each Execute RPC.
// Concurrent-safe per-test (each test owns its own *Server).
func (s *Server) OnClassify(
	fn func(context.Context, *pb.ExecuteRequest) (*pb.ExecuteResponse, error),
) {
	s.onClassify = fn
}

// Execute implements the gRPC server interface — dispatches to
// OnClassify. If unset, returns the canonical "unimplemented" error.
func (s *Server) Execute(
	ctx context.Context,
	req *pb.ExecuteRequest,
) (*pb.ExecuteResponse, error) {
	if s.onClassify == nil {
		return nil, errUnimplemented
	}
	return s.onClassify(ctx, req)
}

// OkResponse is the canonical "happy-path" projection of a
// *efe.Verdict into a *pb.ExecuteResponse — convenient for tests :
//
//	server.OnClassify(func(ctx context.Context, _ *pb.ExecuteRequest) (*pb.ExecuteResponse, error) {
//	    return mock.OkResponse(mock.NewVerdict(mock.WithLabel("fraud"))), nil
//	})
func OkResponse(v *efe.Verdict) *pb.ExecuteResponse {
	if v == nil {
		v = NewVerdict()
	}
	return &pb.ExecuteResponse{
		SwarmId:        v.SwarmID,
		RequestId:      v.RequestID,
		Output:         &pb.DecodedOutput{Text: v.Label},
		Confidence:     float32(v.Confidence),
		Verdict:        verdictKindFromZone(v),
		RoundsExecuted: uint32(v.Rounds),
		AlignmentDelta: float32(v.AlignmentDelta),
		TotalMs:        float32(v.ElapsedMs),
	}
}

func verdictKindFromZone(v *efe.Verdict) pb.Verdict {
	switch {
	case v.Reject || v.Zone == "reject":
		return pb.Verdict_VERDICT_DIVERGENCE
	default:
		return pb.Verdict_VERDICT_CONVERGED
	}
}

// errUnimplemented is the gRPC-style "unimplemented" sentinel
// returned when OnClassify isn't wired.
//
// **Audit fix M9-A2** — uses the canonical `status.Error()` so the
// SDK's `fromGRPC` correctly translates it via `status.FromError`.
// The pre-fix hand-rolled struct returned `GRPCStatus() interface{}`
// which did NOT satisfy the gRPC interface check
// `interface { GRPCStatus() *status.Status }` (signature mismatch).
var errUnimplemented = status.Error(codes.Unimplemented, "mock: OnClassify not configured")
