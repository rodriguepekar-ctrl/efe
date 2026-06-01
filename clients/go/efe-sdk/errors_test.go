// Sprint D23 (Φ-B) — unit tests for the sentinel errors taxonomy.

package efe

import (
	"errors"
	"fmt"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestSentinels_AreDistinct(t *testing.T) {
	t.Parallel()
	all := allSentinels()
	require.Len(t, all, 10, "10 stable sentinels (pin tested)")
	for i, a := range all {
		for j, b := range all {
			if i == j {
				continue
			}
			assert.False(t, errors.Is(a, b),
				"sentinel %d (%v) MUST NOT match sentinel %d (%v)",
				i, a, j, b)
		}
	}
}

func TestErrorCode_KnownSentinels(t *testing.T) {
	t.Parallel()
	cases := map[error]string{
		ErrUnauthenticated:  "unauthenticated",
		ErrPermissionDenied: "permission_denied",
		ErrCatalogNotFound:  "catalog_not_found",
		ErrCatalogInvalid:   "catalog_invalid",
		ErrTimeout:          "timeout",
		ErrUnavailable:      "unavailable",
		ErrRateLimited:      "rate_limited",
		ErrInvalidArgument:  "invalid_argument",
		ErrInternal:         "internal",
		ErrClosed:           "closed",
	}
	for sentinel, expectedCode := range cases {
		got := ErrorCode(sentinel)
		assert.Equal(t, expectedCode, got, "ErrorCode(%v)", sentinel)

		// Wrapped via fmt.Errorf — should still produce the code.
		wrapped := fmt.Errorf("ctx: %w", sentinel)
		assert.Equal(t, expectedCode, ErrorCode(wrapped),
			"ErrorCode(wrapped %v)", sentinel)
	}
}

func TestErrorCode_UnknownReturnsUnknown(t *testing.T) {
	t.Parallel()
	assert.Equal(t, "unknown", ErrorCode(errors.New("random")))
}

func TestErrorCode_NilReturnsEmpty(t *testing.T) {
	t.Parallel()
	assert.Equal(t, "", ErrorCode(nil))
}

func TestIsRetryable_RetryableSet(t *testing.T) {
	t.Parallel()
	for _, retryable := range []error{
		ErrUnavailable,
		ErrTimeout,
		ErrRateLimited,
		fmt.Errorf("ctx: %w", ErrUnavailable),
	} {
		assert.True(t, IsRetryable(retryable),
			"%v should be retryable", retryable)
	}
}

func TestIsRetryable_NonRetryableSet(t *testing.T) {
	t.Parallel()
	for _, nonRetryable := range []error{
		ErrUnauthenticated,
		ErrPermissionDenied,
		ErrCatalogNotFound,
		ErrCatalogInvalid,
		ErrInvalidArgument,
		ErrInternal,
		ErrClosed,
		errors.New("random"),
		nil,
	} {
		assert.False(t, IsRetryable(nonRetryable),
			"%v should NOT be retryable", nonRetryable)
	}
}
