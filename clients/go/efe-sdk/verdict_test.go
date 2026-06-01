// Sprint D23 (Φ-B) — unit tests for Verdict struct.

package efe

import (
	"encoding/json"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestVerdict_IsTrustedWarnReject(t *testing.T) {
	t.Parallel()
	cases := []struct {
		zone     string
		reject   bool
		trusted  bool
		warn     bool
		rejected bool
	}{
		{"trusted", false, true, false, false},
		{"warn", false, false, true, false},
		{"reject", false, false, false, true},
		{"trusted", true, false, false, true},
		{"", false, false, false, false},
	}
	for _, c := range cases {
		v := &Verdict{Zone: c.zone, Reject: c.reject}
		assert.Equal(t, c.trusted, v.IsTrusted(),
			"IsTrusted zone=%q reject=%v", c.zone, c.reject)
		assert.Equal(t, c.warn, v.IsWarn(),
			"IsWarn zone=%q reject=%v", c.zone, c.reject)
		assert.Equal(t, c.rejected, v.IsReject(),
			"IsReject zone=%q reject=%v", c.zone, c.reject)
	}
}

func TestVerdict_String_NoTokenLeak(t *testing.T) {
	t.Parallel()
	// Verdict's String() must surface label/confidence/zone but
	// never anything resembling a JWT or auth header.
	v := &Verdict{
		Label:      "billing",
		Confidence: 0.91,
		Zone:       "trusted",
		Rounds:     4,
		ElapsedMs:  42,
		RequestID:  "req-123",
	}
	s := v.String()
	assert.Contains(t, s, "billing")
	assert.Contains(t, s, "trusted")
	assert.NotContains(t, s, "Bearer")
	assert.NotContains(t, s, "Authorization")
}

func TestVerdict_JSONStable(t *testing.T) {
	t.Parallel()
	v := &Verdict{
		Label:      "billing",
		Confidence: 0.85,
		Reject:     false,
		Rounds:     4,
		Zone:       "trusted",
		Catalog:    "acme/billing@v3",
		SwarmID:    "swarm-x",
		ElapsedMs:  42,
	}
	raw, err := json.Marshal(v)
	require.NoError(t, err)
	// Pin the JSON keys (snake_case) — pin tested for downstream
	// HTTP API stability.
	for _, key := range []string{
		`"label"`, `"confidence"`, `"reject"`, `"rounds"`,
		`"alignment_delta"`, `"catalog"`, `"zone"`, `"swarm_id"`,
		`"elapsed_ms"`,
	} {
		assert.Contains(t, string(raw), key, "missing JSON key %s", key)
	}
}
