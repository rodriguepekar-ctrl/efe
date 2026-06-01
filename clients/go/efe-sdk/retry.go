// Sprint D23 (Φ-B) — Retry interceptor with exponential backoff.
//
// Idempotence table (see § 12.3 of the architecture doc) :
//   - Classify, Healthcheck → retry-eligible (read-mostly).
//   - RegisterCatalog, EvolveCatalog, DeleteCatalog → NEVER retry.
//
// The retry interceptor consults [IsRetryable] before scheduling
// a retry. Non-retryable errors propagate immediately — saves
// budget AND avoids double-side-effects on writes.
//
// Backoff pattern : full jitter (Marc Brooker / AWS canonical) :
//
//	delay = random(0, min(MaxInterval, InitialInterval * Multiplier^attempt))
//
// Pin tested by `test_d23_go_sdk_retry_exponential_with_jitter`.

package efe

import (
	"context"
	"math"
	"math/rand/v2"
	"time"

	"google.golang.org/grpc"
)

// ─────────────────────────────────────────────────────────────────
// RetryBackoff interface + ExponentialBackoff
// ─────────────────────────────────────────────────────────────────

// RetryBackoff computes the delay before the (attempt+1)-th try.
//
// Implementations MUST be deterministic given the same attempt
// number — except for jitter, which intentionally introduces
// randomness to avoid thundering herd.
type RetryBackoff interface {
	// Next returns the delay before retry attempt `attempt` (1-indexed,
	// so attempt=1 = first retry, AFTER the initial try).
	Next(attempt int) time.Duration
}

// ExponentialBackoff implements [RetryBackoff] with full jitter.
//
// Defaults if any field is zero :
//   - InitialInterval : 100 ms
//   - MaxInterval     : 30 s
//   - Multiplier      : 2.0
type ExponentialBackoff struct {
	InitialInterval time.Duration
	MaxInterval     time.Duration
	Multiplier      float64

	// rng is overridable in tests for deterministic delays.
	// Defaults to math/rand/v2 global source (seeded from time + pid).
	rng *rand.Rand
}

// Next computes the delay for the given attempt with full jitter.
func (e ExponentialBackoff) Next(attempt int) time.Duration {
	initial := e.InitialInterval
	if initial <= 0 {
		initial = 100 * time.Millisecond
	}
	maxv := e.MaxInterval
	if maxv <= 0 {
		maxv = 30 * time.Second
	}
	mult := e.Multiplier
	if mult <= 0 {
		mult = 2.0
	}
	// exponential : base * mult^(attempt-1)
	base := float64(initial) * math.Pow(mult, float64(attempt-1))
	if base > float64(maxv) {
		base = float64(maxv)
	}
	// Full jitter : uniform(0, base).
	var jittered float64
	if e.rng != nil {
		jittered = e.rng.Float64() * base
	} else {
		jittered = rand.Float64() * base //nolint:gosec — non-crypto jitter.
	}
	return time.Duration(jittered)
}

// defaultRetryConfig — 3 attempts, 100ms..30s exponential.
func defaultRetryConfig() retryConfig {
	return retryConfig{
		maxAttempts: 3,
		backoff: ExponentialBackoff{
			InitialInterval: 100 * time.Millisecond,
			MaxInterval:     30 * time.Second,
			Multiplier:      2.0,
		},
	}
}

// retryConfig is the internal materialisation of [WithRetry].
type retryConfig struct {
	maxAttempts int
	backoff     RetryBackoff
}

// ─────────────────────────────────────────────────────────────────
// Unary interceptor — the actual retry logic
// ─────────────────────────────────────────────────────────────────

// retryUnaryInterceptor wraps a gRPC unary call with exponential
// retry on retry-eligible errors.
//
// Concurrency : safe for concurrent calls. Each call has its own
// `attempt` counter — no shared state across calls.
func retryUnaryInterceptor(cfg retryConfig, m *metrics) grpc.UnaryClientInterceptor {
	return func(
		ctx context.Context,
		method string,
		req, reply any,
		cc *grpc.ClientConn,
		invoker grpc.UnaryInvoker,
		opts ...grpc.CallOption,
	) error {
		var lastErr error
		for attempt := 1; attempt <= cfg.maxAttempts; attempt++ {
			lastErr = invoker(ctx, method, req, reply, cc, opts...)
			if lastErr == nil {
				if attempt > 1 && m != nil {
					m.observeRetrySuccess(attempt)
				}
				return nil
			}
			// Translate the raw gRPC error to a sentinel — IsRetryable
			// accepts only sentinels.
			translated := fromGRPC(lastErr)
			if !IsRetryable(translated) {
				return lastErr // propagate the RAW gRPC err so callers can inspect status if they want
			}
			if attempt == cfg.maxAttempts {
				break
			}
			delay := cfg.backoff.Next(attempt)
			if m != nil {
				m.observeRetryAttempt(attempt)
			}
			select {
			case <-ctx.Done():
				return ctx.Err()
			case <-time.After(delay):
				// Continue to the next attempt.
			}
		}
		return lastErr
	}
}
