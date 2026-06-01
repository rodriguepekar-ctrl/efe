// Sprint D23 (Φ-B) — Public `Verdict` type.
//
// Mirrors the Python SDK's `Verdict` (M8) with Go idioms :
//   - struct with public fields (vs Python `@dataclass(frozen=True)`)
//   - immutability is *conventional* — callers MUST NOT mutate
//     after Classify() returns. The pin test
//     `test_d23_go_sdk_verdict_fields_stable` anchors the field set.
//
// Cf. `LETTER_D_ARCHITECTURE.md` §5.1 for the cross-language
// canonical shape.

package efe

import (
	"fmt"
	"strings"
)

// Verdict is the public verdict type returned by [EfeClient.Classify].
//
// Conventionally immutable — the SDK never mutates a Verdict
// after returning it, and callers SHOULD NOT either.
//
// All fields are exported so consumers can `json.Marshal(verdict)`
// directly (canonical HTTP JSON response shape for the 3 framework
// examples).
type Verdict struct {
	// Label is the predicted class name (e.g. "fraud", "billing").
	// Empty string when Reject == true.
	Label string `json:"label"`

	// Confidence ∈ [0, 1]. Higher = more confident the label is right.
	Confidence float64 `json:"confidence"`

	// Reject is true iff the swarm decided the input is OUT-of-cone
	// (alignment violation) — refuse the classification.
	Reject bool `json:"reject"`

	// Rounds is the number of fusion rounds the swarm executed
	// before converging.
	Rounds int `json:"rounds"`

	// AlignmentDelta ∈ [0, 1] — distance from the alignment cone
	// boundary. 0 = perfectly aligned ; > MaxProjectionDistance =
	// reject.
	AlignmentDelta float64 `json:"alignment_delta"`

	// Catalog is the canonical key of the catalog used (e.g.
	// "acme/billing@v3").
	Catalog string `json:"catalog"`

	// Zone ∈ {"trusted", "warn", "reject"} — derived from
	// Confidence + ThresholdPolicy of the catalog.
	Zone string `json:"zone"`

	// SwarmID is the server-assigned id of the ephemeral swarm
	// instance that produced this verdict.
	SwarmID string `json:"swarm_id"`

	// RequestID is the gRPC request id (echoed by the server).
	// Empty when caller didn't supply one + server didn't generate.
	RequestID string `json:"request_id,omitempty"`

	// ElapsedMs is the SDK-side wall-clock latency from RPC start
	// to RPC return. Includes serialisation + network + server
	// + deserialisation.
	ElapsedMs int64 `json:"elapsed_ms"`
}

// IsTrusted reports whether the verdict is in the "trusted" zone
// (high confidence + accepted alignment).
func (v *Verdict) IsTrusted() bool {
	return !v.Reject && v.Zone == "trusted"
}

// IsWarn reports whether the verdict is borderline — accepted
// but with a confidence below the high threshold.
func (v *Verdict) IsWarn() bool {
	return !v.Reject && v.Zone == "warn"
}

// IsReject reports whether the verdict is rejected — either
// explicitly (Reject=true) or via Zone=="reject".
func (v *Verdict) IsReject() bool {
	return v.Reject || v.Zone == "reject"
}

// String returns a human-friendly one-line summary.
func (v *Verdict) String() string {
	var b strings.Builder
	fmt.Fprintf(&b, "Verdict{label=%q confidence=%.3f zone=%s rounds=%d",
		v.Label, v.Confidence, v.Zone, v.Rounds)
	if v.Reject {
		b.WriteString(" reject=true")
	}
	if v.RequestID != "" {
		fmt.Fprintf(&b, " req=%s", v.RequestID)
	}
	fmt.Fprintf(&b, " elapsed=%dms}", v.ElapsedMs)
	return b.String()
}

// StreamItem is one verdict + index pair from [EfeClient.ClassifyStream].
//
// Index preserves the input order even when the server returns
// out-of-order. Err is non-nil iff this particular item failed ;
// the stream as a whole continues unless ctx is cancelled.
type StreamItem struct {
	Index   int
	Verdict *Verdict
	Err     error
}
