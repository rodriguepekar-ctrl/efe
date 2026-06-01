// Sprint D24 (Φ-B) — 10 sentinel error classes + helpers.
//
// Cross-language pinned : same 10 sentinels as Python M8 + Go M9.
// Each maps to a canonical gRPC status code (see `from-rpc.ts`).
//
// Idiom : TS class hierarchy. Consumers use `err instanceof ErrTimeout`
// to discriminate (idiomatic TS, equivalent to Go `errors.Is(err,
// ErrTimeout)` or Python `isinstance(err, EfeTimeoutError)`).
//
// Cf. docs/EFE_TYPESCRIPT_SDK_EXTENSION_ARCHITECTURE.md §12.

/**
 * Base class for all SDK-generated errors.
 *
 * Use `err instanceof EfeError` to detect any SDK error generically ;
 * use `err instanceof ErrXxx` to discriminate the specific sentinel.
 *
 * The `code` field is a stable string short-label (used for metrics /
 * logs / IDE autocomplete). The `name` field follows the Node.js
 * convention `EfeXxx` for stack traces.
 */
export class EfeError extends Error {
  readonly code: string;
  override readonly name: string;

  constructor(code: string, message: string, options?: { cause?: unknown }) {
    super(message, options);
    this.code = code;
    this.name = `Efe${code}`;
    // Preserve the prototype chain across transpilation (TS target ES5/6
    // truncates `extends Error` ; ES2022 + above is fine).
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

/** gRPC code 16 (UNAUTHENTICATED) — token missing/malformed/rejected. */
export class ErrUnauthenticated extends EfeError {
  constructor(message = 'unauthenticated', options?: { cause?: unknown }) {
    super('Unauthenticated', `efe: ${message}`, options);
  }
}

/** gRPC code 7 (PERMISSION_DENIED) — token valid but lacks required scope. */
export class ErrPermissionDenied extends EfeError {
  constructor(message = 'permission denied', options?: { cause?: unknown }) {
    super('PermissionDenied', `efe: ${message}`, options);
  }
}

/** gRPC code 5 (NOT_FOUND) — catalog key doesn't resolve on the server. */
export class ErrCatalogNotFound extends EfeError {
  constructor(message = 'catalog not found', options?: { cause?: unknown }) {
    super('CatalogNotFound', `efe: ${message}`, options);
  }
}

/** gRPC code 9 (FAILED_PRECONDITION) — catalog corrupted/version-mismatched. */
export class ErrCatalogInvalid extends EfeError {
  constructor(message = 'catalog invalid', options?: { cause?: unknown }) {
    super('CatalogInvalid', `efe: ${message}`, options);
  }
}

/** gRPC code 4 (DEADLINE_EXCEEDED) — deadline reached. Retry-eligible. */
export class ErrTimeout extends EfeError {
  constructor(message = 'deadline exceeded', options?: { cause?: unknown }) {
    super('Timeout', `efe: ${message}`, options);
  }
}

/** gRPC code 14 (UNAVAILABLE) — server reachable but transient bad state. Retry-eligible. */
export class ErrUnavailable extends EfeError {
  constructor(message = 'server unavailable', options?: { cause?: unknown }) {
    super('Unavailable', `efe: ${message}`, options);
  }
}

/** gRPC code 8 (RESOURCE_EXHAUSTED) — tenant quota exceeded. Retry-eligible. */
export class ErrRateLimited extends EfeError {
  constructor(message = 'rate limited', options?: { cause?: unknown }) {
    super('RateLimited', `efe: ${message}`, options);
  }
}

/** gRPC code 3 (INVALID_ARGUMENT) — request shape is bad. Not retryable. */
export class ErrInvalidArgument extends EfeError {
  constructor(message = 'invalid argument', options?: { cause?: unknown }) {
    super('InvalidArgument', `efe: ${message}`, options);
  }
}

/** gRPC code 13 (INTERNAL) or 2 (UNKNOWN) — server crashed/unknown. */
export class ErrInternal extends EfeError {
  constructor(message = 'server internal error', options?: { cause?: unknown }) {
    super('Internal', `efe: ${message}`, options);
  }
}

/**
 * Local SDK error : the client was already closed when the RPC was
 * attempted. Surfaced by `EfeClient.classify` after a `close()` call.
 */
export class ErrClosed extends EfeError {
  constructor(message = 'client is closed', options?: { cause?: unknown }) {
    super('Closed', `efe: ${message}`, options);
  }
}

/**
 * Returns true iff `err` belongs to the retry-eligible subset.
 *
 * Retry-eligible :
 *   - ErrUnavailable   (gRPC 14 — transient unavailability)
 *   - ErrTimeout       (gRPC 4 — server-side deadline)
 *   - ErrRateLimited   (gRPC 8 — quota — caller MUST honor backoff)
 *
 * Not retry-eligible :
 *   - All others (auth failure, config error, request shape error,
 *     internal server bug, local close).
 */
export function isRetryable(err: unknown): boolean {
  if (!(err instanceof EfeError)) return false;
  return (
    err instanceof ErrUnavailable ||
    err instanceof ErrTimeout ||
    err instanceof ErrRateLimited
  );
}

/**
 * Returns the canonical short code label used in logs / metrics.
 *
 * Stable mapping :
 *   - ErrUnauthenticated  → "unauthenticated"
 *   - ErrPermissionDenied → "permission_denied"
 *   - ErrCatalogNotFound  → "catalog_not_found"
 *   - ErrCatalogInvalid   → "catalog_invalid"
 *   - ErrTimeout          → "timeout"
 *   - ErrUnavailable      → "unavailable"
 *   - ErrRateLimited      → "rate_limited"
 *   - ErrInvalidArgument  → "invalid_argument"
 *   - ErrInternal         → "internal"
 *   - ErrClosed           → "closed"
 *   - other               → "unknown"
 *   - null/undefined      → ""
 */
export function errorCode(err: unknown): string {
  if (err === undefined || err === null) return '';
  if (!(err instanceof EfeError)) return 'unknown';
  switch (err.code) {
    case 'Unauthenticated':
      return 'unauthenticated';
    case 'PermissionDenied':
      return 'permission_denied';
    case 'CatalogNotFound':
      return 'catalog_not_found';
    case 'CatalogInvalid':
      return 'catalog_invalid';
    case 'Timeout':
      return 'timeout';
    case 'Unavailable':
      return 'unavailable';
    case 'RateLimited':
      return 'rate_limited';
    case 'InvalidArgument':
      return 'invalid_argument';
    case 'Internal':
      return 'internal';
    case 'Closed':
      return 'closed';
    default:
      return 'unknown';
  }
}

/**
 * Canonical sentinel list — pin tested for stability.
 * New sentinel = explicit append + bump CHANGELOG + bump
 * `CANONICAL_SENTINELS` in `test_d24_typescript_sdk_extension.py`.
 */
export const ALL_SENTINELS = [
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
] as const;
