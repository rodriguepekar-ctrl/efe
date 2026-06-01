// Package mock ships test fixtures for the efe-sdk-go SDK.
//
// Sprint D23 (Φ-B) — three building blocks :
//   - [NewVerdict] (this file) : factory for canonical [efe.Verdict]
//     instances with sensible defaults + override options.
//   - [NewServer]  ([server.go]) : in-process gRPC mock server
//     (bufconn) + a connected [efe.EfeClient].
//   - [NewClient]  ([client.go]) : pure-struct [efe.EfeClient]
//     replacement, NO gRPC machinery.
//
// Use [NewServer] when your test exercises the FULL gRPC path
// (interceptors, retry, OTel) ; use [NewClient] when you only
// want to inject canned verdicts into a handler.
//
// All public surface is import-stable across SDK minor versions.
package mock

import (
	efe "github.com/efe-ai/efe-sdk-go"
)

// VerdictOption mutates a *efe.Verdict during [NewVerdict] construction.
type VerdictOption func(*efe.Verdict)

// WithLabel sets v.Label.
func WithLabel(label string) VerdictOption {
	return func(v *efe.Verdict) { v.Label = label }
}

// WithConfidence sets v.Confidence.
func WithConfidence(confidence float64) VerdictOption {
	return func(v *efe.Verdict) { v.Confidence = confidence }
}

// WithReject sets v.Reject (and updates v.Zone to "reject").
func WithReject() VerdictOption {
	return func(v *efe.Verdict) {
		v.Reject = true
		v.Zone = "reject"
	}
}

// WithRounds sets v.Rounds.
func WithRounds(rounds int) VerdictOption {
	return func(v *efe.Verdict) { v.Rounds = rounds }
}

// WithCatalog sets v.Catalog.
func WithCatalog(catalog string) VerdictOption {
	return func(v *efe.Verdict) { v.Catalog = catalog }
}

// WithZone sets v.Zone explicitly. Override the auto-derived zone
// (which derives from confidence by default).
func WithZone(zone string) VerdictOption {
	return func(v *efe.Verdict) { v.Zone = zone }
}

// WithSwarmID sets v.SwarmID.
func WithSwarmID(id string) VerdictOption {
	return func(v *efe.Verdict) { v.SwarmID = id }
}

// WithRequestID sets v.RequestID.
func WithRequestID(id string) VerdictOption {
	return func(v *efe.Verdict) { v.RequestID = id }
}

// WithElapsedMs sets v.ElapsedMs.
func WithElapsedMs(ms int64) VerdictOption {
	return func(v *efe.Verdict) { v.ElapsedMs = ms }
}

// NewVerdict builds a canonical *efe.Verdict for tests.
//
// Default :
//
//	{Label: "alpha", Confidence: 0.85, Rounds: 4, Catalog: "test/cat@v1",
//	 Zone: "trusted", SwarmID: "mock-swarm", ElapsedMs: 42}
//
// Override any field via the variadic VerdictOption args. The Zone
// auto-derives from Confidence ONLY IF [WithZone] is NOT passed
// (≥ 0.80 → trusted, ≥ 0.50 → warn, else reject).
func NewVerdict(opts ...VerdictOption) *efe.Verdict {
	v := &efe.Verdict{
		Label:          "alpha",
		Confidence:     0.85,
		Reject:         false,
		Rounds:         4,
		AlignmentDelta: 0.10,
		Catalog:        "test/cat@v1",
		SwarmID:        "mock-swarm",
		ElapsedMs:      42,
	}
	for _, opt := range opts {
		opt(v)
	}
	// Auto-derive zone if it wasn't explicitly set.
	if v.Zone == "" && !v.Reject {
		switch {
		case v.Confidence >= 0.80:
			v.Zone = "trusted"
		case v.Confidence >= 0.50:
			v.Zone = "warn"
		default:
			v.Zone = "reject"
		}
	}
	return v
}
