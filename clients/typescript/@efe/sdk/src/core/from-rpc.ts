// Sprint D24 (Φ-B) — Connect-RPC error → SDK sentinel mapping.
//
// SOLE conversion path between Connect-RPC errors and SDK-typed
// errors. The SDK NEVER surfaces raw `ConnectError` to consumers —
// they always see one of the 10 sentinel classes.
//
// Cf. docs/EFE_TYPESCRIPT_SDK_EXTENSION_ARCHITECTURE.md §12.2.

import { Code, ConnectError } from '@connectrpc/connect';

import {
  ErrCatalogInvalid,
  ErrCatalogNotFound,
  ErrInternal,
  ErrInvalidArgument,
  ErrPermissionDenied,
  ErrRateLimited,
  ErrTimeout,
  ErrUnauthenticated,
  ErrUnavailable,
} from './errors.js';

/**
 * Translate a Connect-RPC error to a typed SDK sentinel.
 *
 * Mapping :
 *   - Code.Unauthenticated     → ErrUnauthenticated
 *   - Code.PermissionDenied    → ErrPermissionDenied
 *   - Code.NotFound            → ErrCatalogNotFound
 *   - Code.FailedPrecondition  → ErrCatalogInvalid
 *   - Code.DeadlineExceeded    → ErrTimeout
 *   - Code.Canceled            → ErrTimeout (closest semantic)
 *   - Code.Unavailable         → ErrUnavailable
 *   - Code.ResourceExhausted   → ErrRateLimited
 *   - Code.InvalidArgument     → ErrInvalidArgument
 *   - Code.OutOfRange          → ErrInvalidArgument
 *   - Code.Internal            → ErrInternal
 *   - Code.Unknown             → ErrInternal
 *   - Code.DataLoss            → ErrInternal
 *   - other                    → ErrInternal (defensive — unknown code)
 *
 * The original `ConnectError` is preserved as `cause` so consumers
 * who want the raw status can still access `err.cause`.
 *
 * **Audit fix M10-A7** : return type was `EfeError | unknown` which
 * collapses to just `unknown` (top-type union) and is misleading.
 * Now explicitly `unknown` — callers narrow via `instanceof EfeError`.
 *
 * @param err Any thrown error. Non-`ConnectError` errors pass through unchanged.
 */
export function fromConnectError(err: unknown): unknown {
  if (!(err instanceof ConnectError)) {
    return err;
  }
  // The conversion below produces an `EfeError` subclass, but TS sees
  // the wider `unknown` return type so the export signature stays clean.
  // Internal callers narrow with `instanceof EfeError`.
  const message = err.rawMessage || err.message;
  const cause = { cause: err };
  switch (err.code) {
    case Code.Unauthenticated:
      return new ErrUnauthenticated(message, cause);
    case Code.PermissionDenied:
      return new ErrPermissionDenied(message, cause);
    case Code.NotFound:
      return new ErrCatalogNotFound(message, cause);
    case Code.FailedPrecondition:
      return new ErrCatalogInvalid(message, cause);
    case Code.DeadlineExceeded:
    case Code.Canceled:
      return new ErrTimeout(message, cause);
    case Code.Unavailable:
      return new ErrUnavailable(message, cause);
    case Code.ResourceExhausted:
      return new ErrRateLimited(message, cause);
    case Code.InvalidArgument:
    case Code.OutOfRange:
      return new ErrInvalidArgument(message, cause);
    case Code.Internal:
    case Code.Unknown:
    case Code.DataLoss:
      return new ErrInternal(message, cause);
    default:
      return new ErrInternal(message, cause);
  }
}
