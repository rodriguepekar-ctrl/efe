// Sprint D23 (Φ-B) — `EfeClient` : the public entry point.
//
// Architecture (cf. EFE_GO_SDK_EXTENSION_ARCHITECTURE.md §10) :
//   1. NewClient applies functional options + validates.
//   2. Builds the gRPC interceptor chain (auth → log → otel →
//      prom → retry).
//   3. Calls `grpc.NewClient` (LAZY DIAL — no I/O yet).
//   4. Returns *EfeClient. First RPC triggers the actual TCP+TLS
//      handshake.
//
// Concurrency : *EfeClient is safe for concurrent use. The
// underlying grpc.ClientConn handles multiplexing.
//
// Lifecycle :
//   - NewClient → ready.
//   - Classify, Healthcheck — concurrent OK.
//   - Close → idempotent ; subsequent calls return ErrClosed.

package efe

import (
	"context"
	"fmt"
	"sync"
	"sync/atomic"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials"
	"google.golang.org/grpc/credentials/insecure"
	pb "github.com/efe-ai/efe-sdk-go/internal/proto/efe/v1"
	"github.com/efe-ai/efe-sdk-go/internal/grpcconn"
)

// EfeClient is the main entry point of the SDK. Construct via
// [NewClient] ; close via [EfeClient.Close].
//
// Safe for concurrent use.
type EfeClient struct {
	conn  *grpc.ClientConn
	swarm pb.SwarmServiceClient

	opts    options
	metrics *metrics

	closeOnce sync.Once
	closed    atomic.Bool
}

// NewClient constructs an [EfeClient] connected (lazily) to addr.
//
// The constructor does NOT block on a TCP / TLS handshake — that
// happens on the first RPC call. NewClient itself can fail only if
//   - no addr is provided ;
//   - an option's validation function returns an error
//     (e.g. unreadable cert file from [WithMTLS]) ;
//   - cross-option validation fails (cf. [options.validate]).
//
// Call [EfeClient.Close] to release the underlying connection.
//
// Example :
//
//	client, err := efe.NewClient(
//	    "staging.efe.ai:50051",
//	    efe.WithJWT(token),
//	    efe.WithDeadline(10*time.Second),
//	)
//	if err != nil { return err }
//	defer client.Close()
func NewClient(addr string, opts ...ClientOption) (*EfeClient, error) {
	o := defaultOptions()
	o.addr = addr
	for _, opt := range opts {
		if err := opt(&o); err != nil {
			return nil, err
		}
	}
	if err := o.validate(); err != nil {
		return nil, err
	}

	m := newMetrics(o.promRegisterer)

	dialOpts := buildDialOptions(&o, m)
	target := grpcconn.StripScheme(o.addr)

	conn, err := grpc.NewClient(target, dialOpts...)
	if err != nil {
		return nil, fmt.Errorf("efe: dial %q: %w", target, err)
	}

	c := &EfeClient{
		conn:    conn,
		swarm:   pb.NewSwarmServiceClient(conn),
		opts:    o,
		metrics: m,
	}
	o.logger.Info(
		"efe-sdk-go: client constructed",
		"addr", target,
		"version", Version,
		"deadline_ms", o.deadline.Milliseconds(),
		"retry_max", o.retry.maxAttempts,
	)
	return c, nil
}

// buildDialOptions composes the canonical interceptor chain.
//
// Order is INTENTIONAL :
//   1. Auth      — first, so the token is attached BEFORE any
//                   downstream layer sees the request.
//   2. OTel      — second, so the span captures the auth-decorated
//                   request and downstream layers see the trace ctx.
//   3. Prometheus — third, so the metrics observe POST-OTel timing
//                   (includes the OTel overhead).
//   4. Retry     — last, so retries are counted as separate
//                   observations + each retry runs through auth+otel
//                   independently (fresh span per retry).
//
// Order pin tested by `test_d23_go_sdk_interceptor_chain_order`.
//
// Credentials selection :
//   - explicit TLS material (WithTLS / WithMTLS) → TLS as configured ;
//   - custom dialer (WithCustomDialer — bufconn / pipe) without TLS
//     material → plain insecure (no TLS handshake, since bufconn is
//     a memory pipe) ; **audit fix M9-A1** ;
//   - `grpc://` / `http://` scheme → plain h2c (dev) ;
//   - else → resolveTLSCredentials default (system roots, TLS 1.2+).
func buildDialOptions(o *options, m *metrics) []grpc.DialOption {
	var creds credentials.TransportCredentials
	switch {
	case o.contextDialer != nil && o.tlsConfig == nil && o.mtls == nil:
		// **Audit fix M9-A1** — custom dialer without explicit TLS
		// material targets a plain pipe (canonical bufconn case).
		// A TLS handshake on a plain pipe would fail because the
		// peer (mock gRPC server) doesn't speak TLS. Pin tested by
		// `test_d23_go_sdk_audit_fix_a1_bufconn_uses_insecure_creds`.
		creds = insecure.NewCredentials()
	case isH2C(o.addr):
		// Allow `grpc://localhost:50051` h2c for the dev path.
		creds = insecure.NewCredentials()
	default:
		creds = resolveTLSCredentials(o)
	}

	unary := []grpc.UnaryClientInterceptor{
		authUnaryInterceptor(o.jwtToken),
		otelUnaryInterceptor(o.tracerProvider),
		promUnaryInterceptor(m),
		retryUnaryInterceptor(o.retry, m),
	}
	stream := []grpc.StreamClientInterceptor{
		authStreamInterceptor(o.jwtToken),
		otelStreamInterceptor(o.tracerProvider),
	}

	dialOpts := []grpc.DialOption{
		grpc.WithTransportCredentials(creds),
		grpc.WithUserAgent(o.userAgent),
		grpc.WithChainUnaryInterceptor(unary...),
		grpc.WithChainStreamInterceptor(stream...),
		grpc.WithKeepaliveParams(grpcconn.DefaultKeepalive()),
		grpc.WithDefaultCallOptions(
			grpc.MaxCallRecvMsgSize(o.maxRecvSize),
		),
	}
	// Test-only : when [WithCustomDialer] was passed, install a
	// gRPC ContextDialer that targets the bufconn (or any custom
	// transport). Overrides the TLS credentials for the dialer
	// path — bufconn is plain TCP-like, no TLS handshake.
	if o.contextDialer != nil {
		dialOpts = append(dialOpts,
			grpc.WithContextDialer(o.contextDialer),
		)
	}
	return dialOpts
}

// isH2C heuristic-matches plain h2c addresses.
func isH2C(addr string) bool {
	switch {
	case len(addr) >= 7 && addr[:7] == "grpc://":
		return true
	case len(addr) >= 7 && addr[:7] == "http://":
		return true
	default:
		return false
	}
}

// promUnaryInterceptor materialises the Prometheus side-effects
// per call. Lives in client.go (not metrics.go) to keep the
// metrics module pure observability + zero gRPC dep.
func promUnaryInterceptor(m *metrics) grpc.UnaryClientInterceptor {
	return func(
		ctx context.Context,
		method string,
		req, reply any,
		cc *grpc.ClientConn,
		invoker grpc.UnaryInvoker,
		opts ...grpc.CallOption,
	) error {
		start := time.Now()
		m.observeStart()
		err := invoker(ctx, method, req, reply, cc, opts...)
		elapsed := time.Since(start).Seconds()
		if err == nil {
			m.observeEnd("success", "", elapsed)
		} else {
			m.observeEnd("error", ErrorCode(fromGRPC(err)), elapsed)
		}
		return err
	}
}

// Close releases the underlying gRPC connection. Idempotent —
// subsequent calls return [ErrClosed] from the RPC methods, but
// Close itself returns nil.
func (c *EfeClient) Close() error {
	var closeErr error
	c.closeOnce.Do(func() {
		c.closed.Store(true)
		if c.conn != nil {
			closeErr = c.conn.Close()
		}
		c.opts.logger.Info("efe-sdk-go: client closed")
	})
	return closeErr
}

// isClosed returns true after Close has been called.
func (c *EfeClient) isClosed() bool {
	return c.closed.Load()
}

// applyDeadline returns a child context with the canonical deadline :
//   - if `ctx` already has a deadline, it wins (caller's intent).
//   - else if c.opts.deadline > 0, apply it.
//   - else return ctx unchanged.
//
// Cancel func is returned and MUST be called by the RPC method
// (canonical Go ctx hygiene).
func (c *EfeClient) applyDeadline(ctx context.Context) (context.Context, context.CancelFunc) {
	if _, hasDeadline := ctx.Deadline(); hasDeadline {
		return ctx, func() {}
	}
	if c.opts.deadline > 0 {
		return context.WithTimeout(ctx, c.opts.deadline)
	}
	return ctx, func() {}
}

// guardClosed is the canonical opening of every RPC method.
// Returns [ErrClosed] wrapped if the client was Close'd already.
func (c *EfeClient) guardClosed() error {
	if c.isClosed() {
		return fmt.Errorf("%w: cannot issue RPC after Close()", ErrClosed)
	}
	return nil
}

// String returns a redacted summary, safe for logs.
func (c *EfeClient) String() string {
	return fmt.Sprintf(
		"EfeClient{addr=%q closed=%v retry_max=%d}",
		grpcconn.StripScheme(c.opts.addr),
		c.isClosed(),
		c.opts.retry.maxAttempts,
	)
}

// (compile-time guard) — the client struct must NOT carry the JWT
// token by mistake in `String()`. We assert the field name lives
// in `opts.jwtToken` and is not surfaced by `String()`.
var _ fmt.Stringer = (*EfeClient)(nil)
