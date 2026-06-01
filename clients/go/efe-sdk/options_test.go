// Sprint D23 (Φ-B) — unit tests for functional options.

package efe

import (
	"testing"
	"time"

	"github.com/prometheus/client_golang/prometheus"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"go.opentelemetry.io/otel/trace/noop"
)

func TestWithJWT_Empty(t *testing.T) {
	t.Parallel()
	o := defaultOptions()
	err := WithJWT("")(&o)
	require.Error(t, err)
	assert.Contains(t, err.Error(), "empty token")
}

func TestWithJWT_Valid(t *testing.T) {
	t.Parallel()
	o := defaultOptions()
	require.NoError(t, WithJWT("test-token")(&o))
	assert.Equal(t, "test-token", o.jwtToken)
}

func TestWithDeadline_Negative(t *testing.T) {
	t.Parallel()
	o := defaultOptions()
	err := WithDeadline(-1 * time.Second)(&o)
	require.Error(t, err)
}

func TestWithDeadline_Zero(t *testing.T) {
	t.Parallel()
	// 0 = disable SDK deadline → rely on caller ctx.
	o := defaultOptions()
	require.NoError(t, WithDeadline(0)(&o))
}

func TestWithRetry_BadAttempts(t *testing.T) {
	t.Parallel()
	o := defaultOptions()
	err := WithRetry(0, ExponentialBackoff{})(&o)
	require.Error(t, err)
}

func TestWithRetry_Valid(t *testing.T) {
	t.Parallel()
	o := defaultOptions()
	require.NoError(t, WithRetry(5, ExponentialBackoff{
		InitialInterval: 10 * time.Millisecond,
	})(&o))
	assert.Equal(t, 5, o.retry.maxAttempts)
}

func TestWithOTel_Nil(t *testing.T) {
	t.Parallel()
	o := defaultOptions()
	err := WithOTel(nil)(&o)
	require.Error(t, err)
}

func TestWithOTel_Valid(t *testing.T) {
	t.Parallel()
	o := defaultOptions()
	tp := noop.NewTracerProvider()
	require.NoError(t, WithOTel(tp)(&o))
	assert.Equal(t, tp, o.tracerProvider)
}

func TestWithPrometheus_Nil(t *testing.T) {
	t.Parallel()
	o := defaultOptions()
	err := WithPrometheus(nil)(&o)
	require.Error(t, err)
}

func TestWithPrometheus_Valid(t *testing.T) {
	t.Parallel()
	o := defaultOptions()
	reg := prometheus.NewRegistry()
	require.NoError(t, WithPrometheus(reg)(&o))
	assert.Equal(t, reg, o.promRegisterer)
}

func TestWithUserAgent_Empty(t *testing.T) {
	t.Parallel()
	o := defaultOptions()
	err := WithUserAgent("")(&o)
	require.Error(t, err)
}

func TestWithUserAgent_Valid(t *testing.T) {
	t.Parallel()
	o := defaultOptions()
	require.NoError(t, WithUserAgent("my-app/1.0")(&o))
	assert.Equal(t, "my-app/1.0", o.userAgent)
}

func TestDefaultOptions_SafeBaseline(t *testing.T) {
	t.Parallel()
	o := defaultOptions()
	assert.NotEmpty(t, o.userAgent)
	assert.NotZero(t, o.maxRecvSize)
	assert.NotZero(t, o.deadline)
	assert.NotNil(t, o.tracerProvider, "default OTel must be no-op tracer (not nil)")
	assert.NotNil(t, o.promRegisterer, "default Prom must be local registry (not nil)")
	assert.NotNil(t, o.logger, "default logger must be discard-handler (not nil)")
}
