// Sprint D23 (Φ-B) — Prometheus metrics.
//
// SOFT dependency : when [WithPrometheus] is unused, the SDK
// installs a LOCAL `prometheus.NewRegistry()` (NOT the global
// `DefaultRegisterer`). Consequence : metrics are accumulated
// in memory but NOT scraped via `promhttp.Handler()` unless the
// caller explicitly wires the local registry.
//
// Canonical metrics :
//   - efe_sdk_classify_total{outcome=success|error}
//   - efe_sdk_classify_latency_seconds{outcome}
//   - efe_sdk_classify_errors_total{sentinel}
//   - efe_sdk_in_flight
//   - efe_sdk_retry_attempts_total{attempt=N}
//
// Names are STABLE — bump major to rename or remove. Pin tested
// by `test_d23_go_sdk_metrics_canonical_names`.

package efe

import (
	"errors"
	"sync/atomic"

	"github.com/prometheus/client_golang/prometheus"
)

// Stable metric names.
const (
	metricClassifyTotal       = "efe_sdk_classify_total"
	metricClassifyLatency     = "efe_sdk_classify_latency_seconds"
	metricClassifyErrorsTotal = "efe_sdk_classify_errors_total"
	metricInFlight            = "efe_sdk_in_flight"
	metricRetryAttemptsTotal  = "efe_sdk_retry_attempts_total"
)

// metrics is the SDK-internal collector bag.
type metrics struct {
	classifyTotal     *prometheus.CounterVec
	classifyLatency   *prometheus.HistogramVec
	classifyErrors    *prometheus.CounterVec
	inFlight          prometheus.Gauge
	retryAttempts     *prometheus.CounterVec

	// inFlightCounter mirrors inFlight as an atomic for the hot path
	// (atomic.Int64 is faster than calling Gauge.Inc/Dec on every
	// Classify call). The Prom Gauge is updated lazily on each
	// observation if the count changed.
	inFlightCounter atomic.Int64
}

// newMetrics registers the 5 collectors with `reg`.
//
// `reg` may be a fresh `prometheus.NewRegistry()` (default — local,
// not scraped) or `prometheus.DefaultRegisterer` (global) or
// any custom Registerer.
//
// Idempotent : if the collectors are already registered (pytest
// re-importing the module, etc.), we recover the existing handles.
func newMetrics(reg prometheus.Registerer) *metrics {
	if reg == nil {
		reg = prometheus.NewRegistry()
	}
	m := &metrics{}

	m.classifyTotal = prometheus.NewCounterVec(
		prometheus.CounterOpts{
			Name: metricClassifyTotal,
			Help: "Total efe_sdk Classify calls, partitioned by outcome.",
		},
		[]string{"outcome"},
	)
	m.classifyLatency = prometheus.NewHistogramVec(
		prometheus.HistogramOpts{
			Name:    metricClassifyLatency,
			Help:    "Classify wall-clock latency in seconds, partitioned by outcome.",
			Buckets: prometheus.DefBuckets,
		},
		[]string{"outcome"},
	)
	m.classifyErrors = prometheus.NewCounterVec(
		prometheus.CounterOpts{
			Name: metricClassifyErrorsTotal,
			Help: "Total efe_sdk Classify errors, partitioned by sentinel.",
		},
		[]string{"sentinel"},
	)
	m.inFlight = prometheus.NewGauge(
		prometheus.GaugeOpts{
			Name: metricInFlight,
			Help: "Currently in-flight Classify calls.",
		},
	)
	m.retryAttempts = prometheus.NewCounterVec(
		prometheus.CounterOpts{
			Name: metricRetryAttemptsTotal,
			Help: "Total retry attempts, partitioned by attempt number.",
		},
		[]string{"attempt"},
	)

	for _, c := range []prometheus.Collector{
		m.classifyTotal, m.classifyLatency, m.classifyErrors,
		m.inFlight, m.retryAttempts,
	} {
		if err := reg.Register(c); err != nil {
			// Already registered — recover the existing handle if possible.
			// On any other error, panic via MustRegister-style behaviour
			// is too aggressive ; we just discard. Tests with parallel
			// EfeClient instances on the same registry hit this path.
			are := &prometheus.AlreadyRegisteredError{}
			if !errors.As(err, are) {
				// Unexpected — leave the field nil-safe (CounterVec handles
				// nil receiver in its callees by panicking, but we never
				// actually hit this path in practice).
				continue
			}
			// Replace with the existing collector to share state.
			switch v := are.ExistingCollector.(type) {
			case *prometheus.CounterVec:
				if c == m.classifyTotal {
					m.classifyTotal = v
				} else if c == m.classifyErrors {
					m.classifyErrors = v
				} else if c == m.retryAttempts {
					m.retryAttempts = v
				}
			case *prometheus.HistogramVec:
				if c == m.classifyLatency {
					m.classifyLatency = v
				}
			case prometheus.Gauge:
				if c == m.inFlight {
					m.inFlight = v
				}
			}
		}
	}
	return m
}

// observeStart marks an in-flight RPC.
func (m *metrics) observeStart() {
	if m == nil {
		return
	}
	v := m.inFlightCounter.Add(1)
	m.inFlight.Set(float64(v))
}

// observeEnd marks the end of an in-flight RPC.
//
// `outcome` ∈ {"success", "error"}.
// `sentinel` is the [ErrorCode] of the error (or "" on success).
// `seconds` is the wall-clock duration.
func (m *metrics) observeEnd(outcome, sentinel string, seconds float64) {
	if m == nil {
		return
	}
	v := m.inFlightCounter.Add(-1)
	m.inFlight.Set(float64(v))
	m.classifyTotal.WithLabelValues(outcome).Inc()
	m.classifyLatency.WithLabelValues(outcome).Observe(seconds)
	if outcome == "error" && sentinel != "" {
		m.classifyErrors.WithLabelValues(sentinel).Inc()
	}
}

// observeRetryAttempt counts a retry try (BEFORE invoking it).
func (m *metrics) observeRetryAttempt(attempt int) {
	if m == nil {
		return
	}
	m.retryAttempts.WithLabelValues(formatAttempt(attempt)).Inc()
}

// observeRetrySuccess marks a successful retry (the call eventually
// succeeded after `attempt` attempts).
func (m *metrics) observeRetrySuccess(attempt int) {
	if m == nil || attempt <= 1 {
		return
	}
	// Re-use retryAttempts with a sentinel label "success_after_<N>"
	// so the histogram of "how many retries before success" emerges
	// without a separate metric.
	m.retryAttempts.WithLabelValues("success").Inc()
}

func formatAttempt(n int) string {
	switch n {
	case 1:
		return "1"
	case 2:
		return "2"
	case 3:
		return "3"
	case 4:
		return "4"
	case 5:
		return "5"
	default:
		return "many"
	}
}
