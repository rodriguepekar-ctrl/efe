// Sprint D23 (Φ-B) — `MockEfeClient` : a pure-struct *efe.EfeClient
// replacement that requires NO gRPC machinery.
//
// Use this for the simplest unit tests where you :
//   - inject canned verdicts ;
//   - assert the texts your handler classified ;
//   - simulate errors via SetError.
//
// For tests that exercise the gRPC stack (auth, retry, OTel,
// interceptor ordering), use [NewServer] (bufconn-based mock) instead.

package mock

import (
	"context"
	"sync"

	efe "github.com/efe-ai/efe-sdk-go"
)

// MockEfeClient is a stand-in for [efe.EfeClient] in unit tests.
//
// Concurrency : safe for concurrent use.
type MockEfeClient struct {
	mu       sync.Mutex
	verdict  *efe.Verdict
	err      error
	calls    []string
	closed   bool
}

// NewClient returns a fresh, empty MockEfeClient.
//
// Default behaviour : Classify returns a canonical [NewVerdict]
// + nil error.
func NewClient() *MockEfeClient {
	return &MockEfeClient{}
}

// SetVerdict sets the verdict returned by subsequent Classify calls.
// Concurrent-safe.
func (m *MockEfeClient) SetVerdict(v *efe.Verdict) {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.verdict = v
}

// SetError causes subsequent Classify calls to return `err`.
// Pass nil to clear.
func (m *MockEfeClient) SetError(err error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.err = err
}

// Calls returns a snapshot of the texts Classify was called with,
// in invocation order. Useful for assertions like :
//
//	require.Equal(t, []string{"a", "b"}, mock.Calls())
func (m *MockEfeClient) Calls() []string {
	m.mu.Lock()
	defer m.mu.Unlock()
	out := make([]string, len(m.calls))
	copy(out, m.calls)
	return out
}

// CallCount returns the number of recorded calls.
func (m *MockEfeClient) CallCount() int {
	m.mu.Lock()
	defer m.mu.Unlock()
	return len(m.calls)
}

// Reset clears the calls log + verdict + error. Useful between
// sub-tests.
func (m *MockEfeClient) Reset() {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.calls = nil
	m.verdict = nil
	m.err = nil
}

// Classify mirrors [efe.EfeClient.Classify].
//
// Behaviour :
//   - If SetError was called with a non-nil err, returns nil + err.
//   - Else if SetVerdict was called, returns that verdict + nil.
//   - Else returns NewVerdict() + nil.
func (m *MockEfeClient) Classify(_ context.Context, text string) (*efe.Verdict, error) {
	m.mu.Lock()
	if m.closed {
		m.mu.Unlock()
		return nil, efe.ErrClosed
	}
	m.calls = append(m.calls, text)
	verdict := m.verdict
	err := m.err
	m.mu.Unlock()

	if err != nil {
		return nil, err
	}
	if verdict != nil {
		// Return a copy so callers' mutations don't poison the next call.
		copy := *verdict
		return &copy, nil
	}
	return NewVerdict(), nil
}

// Healthcheck mirrors [efe.EfeClient.Healthcheck]. Returns nil
// unless SetError was set.
//
// **Audit fix M9-A4** — pre-fix returned nil on SetError(ErrTimeout)
// (special-cased), making the mock inconsistent : Classify returned
// ErrTimeout but Healthcheck reported healthy. The fix : any non-nil
// SetError'd error fails Healthcheck symmetrically.
func (m *MockEfeClient) Healthcheck(_ context.Context) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.closed {
		return efe.ErrClosed
	}
	if m.err != nil {
		return m.err
	}
	return nil
}

// Close marks the mock client closed. Subsequent Classify /
// Healthcheck calls return [efe.ErrClosed].
func (m *MockEfeClient) Close() error {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.closed = true
	return nil
}
