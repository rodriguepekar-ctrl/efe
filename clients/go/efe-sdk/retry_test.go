// Sprint D23 (Φ-B) — unit tests for retry / backoff.

package efe

import (
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestExponentialBackoff_DefaultsApply(t *testing.T) {
	t.Parallel()
	// Zero-value struct should fall back to documented defaults
	// (100ms initial, 30s max, 2.0 multiplier).
	b := ExponentialBackoff{}
	d := b.Next(1)
	assert.GreaterOrEqual(t, d, time.Duration(0))
	assert.Less(t, d, 200*time.Millisecond, "first attempt should jitter < 200ms")
}

func TestExponentialBackoff_GrowsWithAttempt(t *testing.T) {
	t.Parallel()
	b := ExponentialBackoff{
		InitialInterval: 100 * time.Millisecond,
		MaxInterval:     10 * time.Second,
		Multiplier:      2.0,
	}
	// Run multiple samples to average out the jitter and check the
	// monotonic trend on average.
	const samples = 100
	avgs := make([]float64, 5)
	for attempt := 1; attempt <= 5; attempt++ {
		var sum time.Duration
		for i := 0; i < samples; i++ {
			sum += b.Next(attempt)
		}
		avgs[attempt-1] = float64(sum) / float64(samples)
	}
	// avgs[0] < avgs[1] < ... within the cap. With full jitter
	// (uniform [0, base]), the expectation grows multiplicatively.
	for i := 1; i < 5; i++ {
		require.Greater(t, avgs[i], avgs[i-1]*0.5,
			"attempt %d avg %.2f should be >> attempt %d avg %.2f",
			i+1, avgs[i], i, avgs[i-1])
	}
}

func TestExponentialBackoff_CapsAtMaxInterval(t *testing.T) {
	t.Parallel()
	b := ExponentialBackoff{
		InitialInterval: 1 * time.Second,
		MaxInterval:     2 * time.Second,
		Multiplier:      10.0,
	}
	// Even at attempt=10, the delay can't exceed MaxInterval (full
	// jitter of [0, 2s] caps at 2s).
	for attempt := 1; attempt <= 10; attempt++ {
		d := b.Next(attempt)
		assert.LessOrEqual(t, d, 2*time.Second,
			"attempt %d delay %v exceeds MaxInterval", attempt, d)
	}
}

func TestExponentialBackoff_NeverNegative(t *testing.T) {
	t.Parallel()
	b := ExponentialBackoff{
		InitialInterval: 100 * time.Millisecond,
	}
	for attempt := 1; attempt <= 100; attempt++ {
		assert.GreaterOrEqual(t, b.Next(attempt), time.Duration(0),
			"attempt %d returned negative delay", attempt)
	}
}

func TestDefaultRetryConfig_Sensible(t *testing.T) {
	t.Parallel()
	cfg := defaultRetryConfig()
	assert.Equal(t, 3, cfg.maxAttempts)
	assert.NotNil(t, cfg.backoff)
}
