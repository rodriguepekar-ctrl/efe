// Sprint D23 (Φ-B) — Functional options pattern.
//
// All ClientOption return values from `WithXxx(...)` form a stable
// public surface. Pin tested by `test_d23_go_sdk_options_canonical`.
//
// Idiom rationale :
//   - Avoids a 15-arg constructor.
//   - Avoids a mutable Config struct (race-prone).
//   - Allows error reporting at construction time (vs runtime).
//   - Makes deprecation cycles smooth (deprecated option = no-op
//     with a warning, not a struct field removal).
//
// All options validate eagerly + return an error from NewClient
// when invalid.

package efe

import (
	"context"
	"crypto/tls"
	"fmt"
	"io"
	"log/slog"
	"net"
	"time"

	"github.com/prometheus/client_golang/prometheus"
	"go.opentelemetry.io/otel/trace"
	"go.opentelemetry.io/otel/trace/noop"
)

// ─────────────────────────────────────────────────────────────────
// ClientOption + internal options struct
// ─────────────────────────────────────────────────────────────────

// ClientOption mutates an [EfeClient]'s configuration at
// construction.
//
// The returned func may return an error to signal a bad option
// (e.g. an empty token, an unreadable cert file). [NewClient]
// halts on the first error and returns it.
type ClientOption func(*options) error

// options is internal — never exposed through the public API.
// All fields have safe defaults set by [defaultOptions].
type options struct {
	addr             string
	dialTimeout      time.Duration
	deadline         time.Duration

	jwtToken         string
	mtls             *mtlsConfig
	tlsServerName    string
	insecureSkipVerify bool
	tlsConfig        *tls.Config

	retry            retryConfig
	userAgent        string
	maxRecvSize      int

	tracerProvider   trace.TracerProvider
	promRegisterer   prometheus.Registerer
	logger           *slog.Logger

	// Test-only : custom dialer. Set by [WithCustomDialer] — used by
	// [mock.NewServer] to wire bufconn. Production code SHOULD NOT
	// touch this.
	contextDialer    func(ctx context.Context, target string) (net.Conn, error)
}

// defaultOptions returns the canonical zero-config baseline.
// Every WithXxx() override surgically mutates one field.
func defaultOptions() options {
	return options{
		dialTimeout:    5 * time.Second,
		deadline:       10 * time.Second,
		userAgent:      "efe-sdk-go/" + Version,
		maxRecvSize:    16 * 1024 * 1024, // 16 MiB
		retry:          defaultRetryConfig(),
		// Soft-deps — no-op providers by default. Opt-in via WithOTel
		// / WithPrometheus.
		tracerProvider: noop.NewTracerProvider(),
		promRegisterer: prometheus.NewRegistry(), // local, NOT global
		logger:         slog.New(slog.NewTextHandler(io.Discard, nil)),
	}
}

// validate runs cross-field checks AFTER all options are applied.
// Returns nil iff the config is internally consistent.
func (o *options) validate() error {
	if o.addr == "" {
		return fmt.Errorf("efe: empty server address")
	}
	if o.deadline < 0 {
		return fmt.Errorf("efe: WithDeadline value must be ≥ 0")
	}
	if o.dialTimeout < 0 {
		return fmt.Errorf("efe: WithDialTimeout value must be ≥ 0")
	}
	if o.maxRecvSize <= 0 {
		return fmt.Errorf("efe: maxRecvSize must be > 0")
	}
	if o.retry.maxAttempts < 1 {
		return fmt.Errorf("efe: retry maxAttempts must be ≥ 1")
	}
	// JWT + mTLS are not mutually exclusive — mTLS proves identity
	// at the transport layer, JWT proves principal at the app layer.
	// Servers may require both (recommended for production).
	return nil
}

// ─────────────────────────────────────────────────────────────────
// WithJWT
// ─────────────────────────────────────────────────────────────────

// WithJWT attaches a Bearer JWT to every outgoing RPC.
//
// The token is sent as `authorization: Bearer <token>` in the
// gRPC metadata of every request. The SDK does NOT validate the
// token client-side — that's the server's job.
//
// Empty token returns an error : passing WithJWT("") is almost
// always a bug (forgetting to read EFE_JWT_TOKEN from env).
func WithJWT(token string) ClientOption {
	return func(o *options) error {
		if token == "" {
			return fmt.Errorf("efe: WithJWT: empty token (did you forget to set EFE_JWT_TOKEN ?)")
		}
		o.jwtToken = token
		return nil
	}
}

// ─────────────────────────────────────────────────────────────────
// WithMTLS
// ─────────────────────────────────────────────────────────────────

// WithMTLS configures mutual TLS using paths to PEM files.
//
//   - certPath : client certificate (X.509 PEM).
//   - keyPath  : private key (PEM, unencrypted).
//   - caPath   : root CA pool (PEM bundle).
//
// On error (file missing, malformed, key/cert mismatch), the
// option returns the error and [NewClient] aborts.
//
// Production servers running Lettre E with mTLS enabled (D22)
// REQUIRE this — connections without a valid client cert are
// refused at TLS handshake time.
func WithMTLS(certPath, keyPath, caPath string) ClientOption {
	return func(o *options) error {
		cfg, err := loadMTLS(certPath, keyPath, caPath)
		if err != nil {
			return fmt.Errorf("efe: WithMTLS: %w", err)
		}
		o.mtls = cfg
		return nil
	}
}

// ─────────────────────────────────────────────────────────────────
// WithTLS / WithInsecureSkipVerify (advanced)
// ─────────────────────────────────────────────────────────────────

// WithTLS configures the TLS layer with a custom config.
//
// Use this when WithMTLS doesn't fit (e.g. you need to load certs
// from a Vault secret instead of disk, or use a custom CipherSuites
// list). The SDK does NOT enforce TLS 1.3 here — pass
// `MinVersion: tls.VersionTLS13` in your config.
func WithTLS(cfg *tls.Config) ClientOption {
	return func(o *options) error {
		if cfg == nil {
			return fmt.Errorf("efe: WithTLS: nil config")
		}
		o.tlsConfig = cfg
		return nil
	}
}

// WithInsecureSkipVerify disables TLS server certificate verification.
//
// **DEV ONLY** — emit a UserAgent suffix `(insecure)` so server-side
// logs surface this. Production code that calls this in `if
// stage == "prod"` is asking for a CVE.
func WithInsecureSkipVerify() ClientOption {
	return func(o *options) error {
		o.insecureSkipVerify = true
		return nil
	}
}

// ─────────────────────────────────────────────────────────────────
// WithDeadline / WithDialTimeout
// ─────────────────────────────────────────────────────────────────

// WithDeadline sets the per-RPC deadline applied when the caller's
// `ctx` has no deadline of its own. 0 = no SDK-side deadline (rely
// on caller's ctx).
//
// The actual deadline used for an RPC is the `min(d, ctx.Deadline)`.
func WithDeadline(d time.Duration) ClientOption {
	return func(o *options) error {
		if d < 0 {
			return fmt.Errorf("efe: WithDeadline must be ≥ 0, got %v", d)
		}
		o.deadline = d
		return nil
	}
}

// WithDialTimeout sets how long the gRPC dial may block on the
// initial TCP+TLS handshake. Default 5s.
func WithDialTimeout(d time.Duration) ClientOption {
	return func(o *options) error {
		if d <= 0 {
			return fmt.Errorf("efe: WithDialTimeout must be > 0, got %v", d)
		}
		o.dialTimeout = d
		return nil
	}
}

// ─────────────────────────────────────────────────────────────────
// WithRetry
// ─────────────────────────────────────────────────────────────────

// WithRetry configures the retry policy for retry-eligible errors
// (cf. [IsRetryable]).
//
//   - maxAttempts ≥ 1. 1 = no retry (single try). 3 = recommended.
//   - backoff : algorithm to compute the inter-attempt delay.
//     [ExponentialBackoff] is the canonical choice.
func WithRetry(maxAttempts int, backoff RetryBackoff) ClientOption {
	return func(o *options) error {
		if maxAttempts < 1 {
			return fmt.Errorf("efe: WithRetry maxAttempts must be ≥ 1, got %d", maxAttempts)
		}
		if backoff == nil {
			return fmt.Errorf("efe: WithRetry: nil backoff")
		}
		o.retry = retryConfig{maxAttempts: maxAttempts, backoff: backoff}
		return nil
	}
}

// ─────────────────────────────────────────────────────────────────
// WithOTel / WithPrometheus / WithLogger (soft-dep observability)
// ─────────────────────────────────────────────────────────────────

// WithOTel installs an OpenTelemetry tracer provider.
//
// The SDK creates one span per RPC named `efe.classify` /
// `efe.healthcheck`, with the canonical `rpc.system=grpc` +
// `rpc.service=efe.v1.SwarmService` attributes (auto-set by
// `otelgrpc`).
//
// Default (when this option is unused) : no-op tracer (zero
// overhead).
func WithOTel(tp trace.TracerProvider) ClientOption {
	return func(o *options) error {
		if tp == nil {
			return fmt.Errorf("efe: WithOTel: nil tracer provider")
		}
		o.tracerProvider = tp
		return nil
	}
}

// WithPrometheus installs a Prometheus registerer where the SDK
// publishes its 5 collectors (efe_sdk_classify_total,
// efe_sdk_classify_latency_seconds, efe_sdk_classify_errors_total,
// efe_sdk_in_flight, efe_sdk_retry_attempts_total).
//
// Pass `prometheus.DefaultRegisterer` to publish on the global
// registry, OR a fresh `prometheus.NewRegistry()` to publish on
// an isolated one (useful for tests).
//
// Default (when this option is unused) : a LOCAL registry that's
// NOT scraped by `promhttp.Handler()` — i.e. the metrics are
// effectively no-op.
func WithPrometheus(reg prometheus.Registerer) ClientOption {
	return func(o *options) error {
		if reg == nil {
			return fmt.Errorf("efe: WithPrometheus: nil registerer")
		}
		o.promRegisterer = reg
		return nil
	}
}

// WithLogger installs a structured logger ([slog.Logger]).
//
// The SDK logs at INFO when an RPC succeeds, WARN on retryable
// errors, ERROR on non-retryable ones. JWT tokens + mTLS material
// are NEVER logged (the auth interceptor uses opaque structs to
// hold them).
//
// Default : a logger that writes to `io.Discard` (silent).
func WithLogger(logger *slog.Logger) ClientOption {
	return func(o *options) error {
		if logger == nil {
			return fmt.Errorf("efe: WithLogger: nil logger")
		}
		o.logger = logger
		return nil
	}
}

// ─────────────────────────────────────────────────────────────────
// WithUserAgent
// ─────────────────────────────────────────────────────────────────

// WithUserAgent overrides the default `efe-sdk-go/<version>`
// user agent. The SDK appends a sub-version "(insecure)" automatically
// when [WithInsecureSkipVerify] is also set, regardless of the
// caller's value.
//
// Operators MAY use this to identify their app on server-side logs :
//
//	efe.WithUserAgent("billing-router/1.4.0")
func WithUserAgent(ua string) ClientOption {
	return func(o *options) error {
		if ua == "" {
			return fmt.Errorf("efe: WithUserAgent: empty value")
		}
		o.userAgent = ua
		return nil
	}
}

// ─────────────────────────────────────────────────────────────────
// WithMaxRecvSize (advanced)
// ─────────────────────────────────────────────────────────────────

// WithMaxRecvSize overrides the gRPC max incoming message size
// (bytes). Default 16 MiB. Bump if your server returns large
// streaming verdicts.
func WithMaxRecvSize(bytes int) ClientOption {
	return func(o *options) error {
		if bytes <= 0 {
			return fmt.Errorf("efe: WithMaxRecvSize must be > 0, got %d", bytes)
		}
		o.maxRecvSize = bytes
		return nil
	}
}

// ─────────────────────────────────────────────────────────────────
// WithCustomDialer (test-only — kept public so the mock/ package
// can use it ; production callers should NOT need this).
// ─────────────────────────────────────────────────────────────────

// WithCustomDialer installs a custom gRPC dial function.
//
// **TEST-ONLY** — used by the [mock.NewServer] helper to wire an
// EfeClient to a `bufconn.Listener` in-process. Production code
// SHOULD NOT use this option : the default dialer (system network)
// is the right answer.
//
// The signature matches `grpc.WithContextDialer`'s expectation
// (`func(context.Context, string) (net.Conn, error)`).
func WithCustomDialer(
	dialer func(ctx context.Context, target string) (net.Conn, error),
) ClientOption {
	return func(o *options) error {
		if dialer == nil {
			return fmt.Errorf("efe: WithCustomDialer: nil dialer")
		}
		o.contextDialer = dialer
		return nil
	}
}
