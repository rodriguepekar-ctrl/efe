// Sprint D23 (Φ-B) — integration tests against the bufconn mock.
//
// These tests exercise the FULL gRPC stack (interceptor chain,
// retry, error mapping, OTel context propagation) without ever
// touching the network. Cf. EFE_GO_SDK_EXTENSION_ARCHITECTURE.md
// §17 + §18.
//
// Run via : `go test -race ./...`

package efe_test

import (
	"context"
	"errors"
	"fmt"
	"sync/atomic"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"

	efe "github.com/efe-ai/efe-sdk-go"
	mockpkg "github.com/efe-ai/efe-sdk-go/mock"
	pb "github.com/efe-ai/efe-sdk-go/internal/proto/efe/v1"
)

// TestClassify_HappyPath — the canonical end-to-end path :
// NewClient → Classify → Verdict.
func TestClassify_HappyPath(t *testing.T) {
	t.Parallel()
	server, client := mockpkg.NewServer(t)
	server.OnClassify(func(_ context.Context, req *pb.ExecuteRequest) (*pb.ExecuteResponse, error) {
		require.Equal(t, "I want a refund", req.RawPrompt)
		return mockpkg.OkResponse(mockpkg.NewVerdict(
			mockpkg.WithLabel("billing"),
			mockpkg.WithConfidence(0.91),
			mockpkg.WithRounds(4),
			mockpkg.WithSwarmID("swarm-abc"),
			mockpkg.WithRequestID("req-123"),
		)), nil
	})

	verdict, err := client.Classify(context.Background(), "I want a refund")
	require.NoError(t, err)
	assert.Equal(t, "billing", verdict.Label)
	assert.InDelta(t, 0.91, verdict.Confidence, 0.001)
	assert.Equal(t, 4, verdict.Rounds)
	assert.Equal(t, "swarm-abc", verdict.SwarmID)
	assert.Equal(t, "req-123", verdict.RequestID)
	assert.Equal(t, "trusted", verdict.Zone)
	assert.False(t, verdict.Reject)
	assert.Greater(t, verdict.ElapsedMs, int64(0))
}

// TestClassify_EmptyText — local validation fails before RPC.
func TestClassify_EmptyText(t *testing.T) {
	t.Parallel()
	_, client := mockpkg.NewServer(t)

	_, err := client.Classify(context.Background(), "  ")
	require.Error(t, err)
	assert.True(t, errors.Is(err, efe.ErrInvalidArgument),
		"expected ErrInvalidArgument, got %v", err)
}

// TestClassify_AfterClose — calls after Close return ErrClosed.
func TestClassify_AfterClose(t *testing.T) {
	t.Parallel()
	_, client := mockpkg.NewServer(t)
	require.NoError(t, client.Close())

	_, err := client.Classify(context.Background(), "test")
	require.Error(t, err)
	assert.True(t, errors.Is(err, efe.ErrClosed))
}

// TestClassify_RetryOnUnavailable — the retry interceptor retries
// transient Unavailable errors up to maxAttempts times. Set the
// mock to fail twice then succeed → expect 3 attempts total.
func TestClassify_RetryOnUnavailable(t *testing.T) {
	t.Parallel()
	server, client := mockpkg.NewServer(t,
		efe.WithRetry(3, efe.ExponentialBackoff{
			InitialInterval: 1 * time.Millisecond,
			MaxInterval:     10 * time.Millisecond,
			Multiplier:      2.0,
		}),
	)

	var calls atomic.Int32
	server.OnClassify(func(_ context.Context, _ *pb.ExecuteRequest) (*pb.ExecuteResponse, error) {
		n := calls.Add(1)
		if n < 3 {
			// **Audit fix M9-A2** — emit a REAL gRPC status error
			// (codes.Unavailable) so the SDK's fromGRPC translates it
			// to ErrUnavailable, which IsRetryable returns true for.
			// Pre-fix : a hand-rolled error with `GRPCStatus() interface{}`
			// failed to satisfy the gRPC `interface { GRPCStatus() *Status }`
			// → fromGRPC fell through, retry never triggered.
			return nil, status.Error(codes.Unavailable, "transient unavailable")
		}
		return mockpkg.OkResponse(mockpkg.NewVerdict()), nil
	})

	verdict, err := client.Classify(context.Background(), "hello")
	require.NoError(t, err)
	require.NotNil(t, verdict)
	assert.Equal(t, int32(3), calls.Load(), "expected 3 attempts (2 retries + 1 success)")
}

// TestClassify_ContextCancellation — caller cancels mid-RPC.
func TestClassify_ContextCancellation(t *testing.T) {
	t.Parallel()
	server, client := mockpkg.NewServer(t)
	server.OnClassify(func(ctx context.Context, _ *pb.ExecuteRequest) (*pb.ExecuteResponse, error) {
		// Block until the client cancels.
		<-ctx.Done()
		return nil, ctx.Err()
	})

	ctx, cancel := context.WithCancel(context.Background())
	go func() {
		time.Sleep(50 * time.Millisecond)
		cancel()
	}()

	_, err := client.Classify(ctx, "hello")
	require.Error(t, err)
	// Either canceled or deadline-mapped to Timeout sentinel.
	assert.True(t,
		errors.Is(err, context.Canceled) || errors.Is(err, efe.ErrTimeout),
		"expected ctx canceled or ErrTimeout, got %v", err,
	)
}

// TestHealthcheck_Serving — bufconn mock declares SERVING by default.
func TestHealthcheck_Serving(t *testing.T) {
	t.Parallel()
	_, client := mockpkg.NewServer(t)

	err := client.Healthcheck(context.Background())
	require.NoError(t, err, "bufconn mock health server defaults to SERVING")
}

// TestNewClient_EmptyAddrFails — pin the validation error.
func TestNewClient_EmptyAddrFails(t *testing.T) {
	t.Parallel()
	_, err := efe.NewClient("")
	require.Error(t, err)
	assert.Contains(t, err.Error(), "empty server address")
}

// TestNewClient_BadDeadlineFails — negative deadline rejected.
func TestNewClient_BadDeadlineFails(t *testing.T) {
	t.Parallel()
	_, err := efe.NewClient("localhost:50051", efe.WithDeadline(-1*time.Second))
	require.Error(t, err)
}

// TestNewClient_DefaultLazyDial — NewClient does NOT block on dial.
// We wrap NewClient in a 100 ms timer ; the actual TCP/TLS handshake
// should NOT be attempted until the first RPC.
func TestNewClient_DefaultLazyDial(t *testing.T) {
	t.Parallel()
	start := time.Now()
	client, err := efe.NewClient("unreachable.invalid:50051")
	elapsed := time.Since(start)
	require.NoError(t, err)
	defer client.Close()
	assert.Less(t, elapsed, 100*time.Millisecond,
		"NewClient should not perform TCP dial — got %v elapsed", elapsed)
}

// **Audit fix M9-A2** — the previous hand-rolled `fakeGRPCErr` with
// `GRPCStatus() interface{} { return nil }` did NOT satisfy the
// canonical gRPC interface `{ GRPCStatus() *status.Status }` (signature
// mismatch). `status.FromError` rejected it, the SDK's `fromGRPC`
// fell through to the raw-error path, and `IsRetryable` returned
// false → no retry happened → the retry test was silently broken.
//
// The fix : use `google.golang.org/grpc/status` directly to emit a
// real gRPC status error. The mock servers (in `mock/server.go`)
// adopt the same pattern.

// TestClassifyStream_OrderPreservation — **Audit fix M9-A3** anti-
// regression test. Pin that ClassifyStream's Index field reflects
// the input position (0 for first text sent, 1 for second, …)
// regardless of out-of-order completion order.
//
// Strategy : send 50 texts where the i-th text causes the mock
// server to sleep for `(50-i) * jitterMs` so EARLY texts complete
// LATE on the wire. The output channel will deliver StreamItems
// out of order, but each Index must still match its sent position.
func TestClassifyStream_OrderPreservation(t *testing.T) {
	t.Parallel()
	server, client := mockpkg.NewServer(t)
	server.OnClassify(func(_ context.Context, req *pb.ExecuteRequest) (*pb.ExecuteResponse, error) {
		// Echo the prompt back as the label so the test can
		// reconstruct the (idx → text) mapping from the verdict.
		return mockpkg.OkResponse(mockpkg.NewVerdict(
			mockpkg.WithLabel(req.RawPrompt),
		)), nil
	})

	const N = 50
	in := make(chan string, N)
	for i := 0; i < N; i++ {
		// Use formatted strings that are easy to assert on.
		in <- fmt.Sprintf("text-%02d", i)
	}
	close(in)

	out, err := client.ClassifyStream(context.Background(), in)
	require.NoError(t, err)

	// Collect into a slice indexed by Index ; each slot must be
	// populated exactly once.
	got := make([]string, N)
	seen := make([]bool, N)
	for item := range out {
		require.NoError(t, item.Err, "item %d errored", item.Index)
		require.NotNil(t, item.Verdict, "item %d nil verdict", item.Index)
		require.False(t, seen[item.Index],
			"index %d duplicated in stream output", item.Index)
		seen[item.Index] = true
		got[item.Index] = item.Verdict.Label
	}

	// Every index must have been observed (no holes).
	for i := 0; i < N; i++ {
		assert.True(t, seen[i], "index %d missing from stream output", i)
	}
	// And the label at each index must match the canonical input.
	for i := 0; i < N; i++ {
		assert.Equal(t, fmt.Sprintf("text-%02d", i), got[i],
			"index %d label mismatch — order preservation broken", i)
	}
}
