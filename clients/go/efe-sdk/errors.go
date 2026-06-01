// Sprint D23 (Φ-B) — Public sentinel errors + gRPC code mapping.
//
// Idiom : sentinel `var Err* = errors.New("efe: ...")` wrapped via
// `fmt.Errorf("...: %w", err)`. Consumers use `errors.Is(err,
// efe.ErrXxx)` to discriminate.
//
// Pin tested by `test_d23_go_sdk_errors_taxonomy_stable` (anti-
// regression : adding/removing a sentinel requires a deliberate
// test update).
//
// Mapping table (gRPC code → SDK sentinel) is the SOLE conversion
// path between server-side errors and client-side handling. The
// SDK NEVER surfaces raw `google.golang.org/grpc/status.Status`
// to consumers — they always see a wrapped sentinel.

package efe

import (
	"errors"
	"fmt"

	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

// ─────────────────────────────────────────────────────────────────
// Sentinel errors — public, stable. Bump major to remove or rename.
// ─────────────────────────────────────────────────────────────────

var (
	// ErrUnauthenticated — gRPC code 16 (UNAUTHENTICATED). Token
	// missing, malformed, or rejected by the server.
	ErrUnauthenticated = errors.New("efe: unauthenticated")

	// ErrPermissionDenied — gRPC code 7. Token valid but lacking
	// the required scope (e.g. trying admin RPC with a `data`-only
	// token).
	ErrPermissionDenied = errors.New("efe: permission denied")

	// ErrCatalogNotFound — gRPC code 5 (NOT_FOUND). Catalog key
	// doesn't resolve on the server.
	ErrCatalogNotFound = errors.New("efe: catalog not found")

	// ErrCatalogInvalid — gRPC code 9 (FAILED_PRECONDITION). Catalog
	// exists but is corrupted, version-mismatched, or otherwise
	// unusable.
	ErrCatalogInvalid = errors.New("efe: catalog invalid")

	// ErrTimeout — gRPC code 4 (DEADLINE_EXCEEDED). The deadline
	// was reached before the server responded. Also surfaced when
	// `ctx.Deadline()` triggers locally.
	ErrTimeout = errors.New("efe: deadline exceeded")

	// ErrUnavailable — gRPC code 14 (UNAVAILABLE). Server reachable
	// but in a transient bad state (restart, leader election, …).
	ErrUnavailable = errors.New("efe: server unavailable")

	// ErrRateLimited — gRPC code 8 (RESOURCE_EXHAUSTED). Tenant
	// quota exceeded. Retry-eligible after backoff.
	ErrRateLimited = errors.New("efe: rate limited")

	// ErrInvalidArgument — gRPC code 3. The request shape is bad
	// (empty text, malformed catalog ref, …). Not retryable.
	ErrInvalidArgument = errors.New("efe: invalid argument")

	// ErrInternal — gRPC code 13 (INTERNAL) or 2 (UNKNOWN). Server
	// crashed or is in an unknown state. Not retryable by default.
	ErrInternal = errors.New("efe: server internal error")

	// ErrClosed — local SDK error : the client was already closed
	// when the RPC was attempted. Surfaced by [EfeClient.Classify]
	// after a [EfeClient.Close] call.
	ErrClosed = errors.New("efe: client is closed")
)

// ─────────────────────────────────────────────────────────────────
// fromGRPC : translate a gRPC status into one of the sentinels above.
// ─────────────────────────────────────────────────────────────────

// fromGRPC maps a gRPC error to a wrapped sentinel.
//
// The original status message is preserved as the wrapped error's
// .Error() suffix. Callers can recover the gRPC code via
// `status.FromError(unwrap(err))` if needed (advanced — most
// callers should rely on errors.Is).
//
// nil input → nil output.
func fromGRPC(err error) error {
	if err == nil {
		return nil
	}
	st, ok := status.FromError(err)
	if !ok {
		// Not a gRPC status — pass through unchanged. Could be a
		// local I/O error, a context cancellation, or a panic
		// turned into error somewhere upstream.
		return err
	}
	var sentinel error
	switch st.Code() {
	case codes.OK:
		return nil
	case codes.Canceled:
		// gRPC canonical : Canceled = ctx.Cancel(). Surface as Timeout
		// (closest semantic in our taxonomy).
		sentinel = ErrTimeout
	case codes.Unauthenticated:
		sentinel = ErrUnauthenticated
	case codes.PermissionDenied:
		sentinel = ErrPermissionDenied
	case codes.NotFound:
		sentinel = ErrCatalogNotFound
	case codes.InvalidArgument, codes.OutOfRange:
		sentinel = ErrInvalidArgument
	case codes.FailedPrecondition:
		sentinel = ErrCatalogInvalid
	case codes.DeadlineExceeded:
		sentinel = ErrTimeout
	case codes.Unavailable:
		sentinel = ErrUnavailable
	case codes.ResourceExhausted:
		sentinel = ErrRateLimited
	case codes.Internal, codes.Unknown, codes.DataLoss:
		sentinel = ErrInternal
	default:
		// Unknown code — wrap with a generic message. The sentinel
		// is ErrInternal so retry policy errs on the side of "stop".
		sentinel = ErrInternal
	}
	return fmt.Errorf("%w: %s (code=%s)", sentinel, st.Message(), st.Code())
}

// IsRetryable reports whether `err` belongs to the retry-eligible
// subset.
//
// Retry-eligible :
//   - ErrUnavailable
//   - ErrTimeout (client-side deadline NOT included — caller's
//     responsibility).
//   - ErrRateLimited
//
// Not retry-eligible :
//   - ErrUnauthenticated, ErrPermissionDenied — auth failure.
//   - ErrCatalogNotFound, ErrCatalogInvalid — config error.
//   - ErrInvalidArgument — request shape error.
//   - ErrInternal — server bug ; could be retried but high risk.
//   - ErrClosed — local state, client already closed.
//
// The retry interceptor in retry.go consults this function before
// scheduling a retry.
func IsRetryable(err error) bool {
	switch {
	case err == nil:
		return false
	case errors.Is(err, ErrUnavailable):
		return true
	case errors.Is(err, ErrTimeout):
		return true
	case errors.Is(err, ErrRateLimited):
		return true
	default:
		return false
	}
}

// ErrorCode returns the canonical short code used in logs / metrics.
//
// Stable mapping :
//   - ErrUnauthenticated  → "unauthenticated"
//   - ErrPermissionDenied → "permission_denied"
//   - ErrCatalogNotFound  → "catalog_not_found"
//   - ErrCatalogInvalid   → "catalog_invalid"
//   - ErrTimeout          → "timeout"
//   - ErrUnavailable      → "unavailable"
//   - ErrRateLimited      → "rate_limited"
//   - ErrInvalidArgument  → "invalid_argument"
//   - ErrInternal         → "internal"
//   - ErrClosed           → "closed"
//   - other               → "unknown"
func ErrorCode(err error) string {
	switch {
	case err == nil:
		return ""
	case errors.Is(err, ErrUnauthenticated):
		return "unauthenticated"
	case errors.Is(err, ErrPermissionDenied):
		return "permission_denied"
	case errors.Is(err, ErrCatalogNotFound):
		return "catalog_not_found"
	case errors.Is(err, ErrCatalogInvalid):
		return "catalog_invalid"
	case errors.Is(err, ErrTimeout):
		return "timeout"
	case errors.Is(err, ErrUnavailable):
		return "unavailable"
	case errors.Is(err, ErrRateLimited):
		return "rate_limited"
	case errors.Is(err, ErrInvalidArgument):
		return "invalid_argument"
	case errors.Is(err, ErrInternal):
		return "internal"
	case errors.Is(err, ErrClosed):
		return "closed"
	default:
		return "unknown"
	}
}

// allSentinels is the canonical list — pin tested for stability.
// New sentinel = explicit append + bump CHANGELOG.
func allSentinels() []error {
	return []error{
		ErrUnauthenticated,
		ErrPermissionDenied,
		ErrCatalogNotFound,
		ErrCatalogInvalid,
		ErrTimeout,
		ErrUnavailable,
		ErrRateLimited,
		ErrInvalidArgument,
		ErrInternal,
		ErrClosed,
	}
}
