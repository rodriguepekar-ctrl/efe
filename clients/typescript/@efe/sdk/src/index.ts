// Sprint D24 (Φ-B) — Universal entrypoint (re-exports core types).
//
// In runtime context :
//   - Browser bundlers resolve `@efe/sdk` → `dist/web/index.js`
//     (which itself re-exports from this file).
//   - Node bundlers resolve `@efe/sdk` → `dist/node/index.js`.
//
// This file is the SHARED CORE — it re-exports types + helpers that
// work identically in both runtimes. It does NOT export `EfeClient`
// directly ; that's done by the per-runtime wrapper (`src/node.ts`
// + `src/web.ts`).

// ─────────────────────────────────────────────────────────────────
// Verdict + StreamItem types
// ─────────────────────────────────────────────────────────────────
export type { Verdict, StreamItem } from './core/verdict.js';
export { isTrusted, isWarn, isReject, deriveZone } from './core/verdict.js';

// ─────────────────────────────────────────────────────────────────
// Client options
// ─────────────────────────────────────────────────────────────────
export type {
  ClientOptions,
  ResolvedClientOptions,
  AuthConfig,
  RetryConfig,
  OTelConfig,
  PrometheusConfig,
} from './core/options.js';
export { defaultRetry, validateOptions } from './core/options.js';

// ─────────────────────────────────────────────────────────────────
// Per-call options (CallOptions, ClassifyStreamOptions)
// ─────────────────────────────────────────────────────────────────
export type { CallOptions, ClassifyStreamOptions } from './core/_types.js';

// ─────────────────────────────────────────────────────────────────
// Sentinel error classes
// ─────────────────────────────────────────────────────────────────
export {
  EfeError,
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
  isRetryable,
  errorCode,
  ALL_SENTINELS,
} from './core/errors.js';

// ─────────────────────────────────────────────────────────────────
// Retry
// ─────────────────────────────────────────────────────────────────
export type { RetryBackoff } from './core/retry.js';
export { ExponentialBackoff, sleep } from './core/retry.js';

// ─────────────────────────────────────────────────────────────────
// Auth helper
// ─────────────────────────────────────────────────────────────────
export { buildBearerHeader } from './core/auth.js';

// ─────────────────────────────────────────────────────────────────
// Version
// ─────────────────────────────────────────────────────────────────
export { Version } from './core/version.js';
