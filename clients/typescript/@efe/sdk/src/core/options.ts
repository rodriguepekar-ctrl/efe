// Sprint D24 (Φ-B) — Constructor options for EfeClient + EfeWebClient.
//
// Idiom : single options-object passed to the constructor (canonical TS).
// Equivalent to Go SDK functional options or Python kwargs.
//
// All fields are validated EAGERLY at construction time — bad config
// → throw at `new EfeClient(...)`, never at first RPC.
//
// Cf. docs/EFE_TYPESCRIPT_SDK_EXTENSION_ARCHITECTURE.md §10.4.

import type { TracerProvider } from '@opentelemetry/api';
import type { Registry } from 'prom-client';

import { ErrInvalidArgument } from './errors.js';
import { ExponentialBackoff, type RetryBackoff } from './retry.js';
import { Version } from './version.js';

/**
 * Discriminated union of auth strategies.
 *
 *   - `jwt` : Bearer token in `authorization` header (universal).
 *   - `mtls` : client certificate at TLS handshake (Node-only — paths
 *     read at construction time via `fs.readFileSync`).
 *   - `none` : no auth (DEV ONLY — server typically rejects).
 */
export type AuthConfig =
  | { readonly kind: 'jwt'; readonly token: string }
  | {
      readonly kind: 'mtls';
      readonly certPath: string;
      readonly keyPath: string;
      readonly caPath: string;
    }
  | { readonly kind: 'none' };

/** Retry config — Node only ; ignored by EfeWebClient. */
export interface RetryConfig {
  /** ≥ 1. 1 = no retry (single try). 3 = recommended. */
  readonly maxAttempts: number;
  /** Backoff impl — see `ExponentialBackoff`. */
  readonly backoff: RetryBackoff;
}

/**
 * OTel tracer provider config.
 *
 *   - `noop`  : no tracing (default).
 *   - `provider` : delegate to a user-supplied `TracerProvider`
 *     (e.g. `NodeTracerProvider` or `WebTracerProvider`).
 */
export type OTelConfig =
  | { readonly kind: 'noop' }
  | { readonly kind: 'provider'; readonly tracerProvider: TracerProvider };

/**
 * Prometheus registry config — Node only.
 *
 *   - `noop`     : no metrics (default).
 *   - `registry` : delegate to a user-supplied `prom-client` Registry.
 */
export type PrometheusConfig =
  | { readonly kind: 'noop' }
  | { readonly kind: 'registry'; readonly registry: Registry };

/**
 * ClientOptions — the constructor input for `new EfeClient(...)` /
 * `new EfeWebClient(...)`.
 *
 * Required : `endpoint`. All other fields have safe defaults.
 */
export interface ClientOptions {
  /** gRPC endpoint, e.g. 'staging.efe.ai:50051' (Node) or 'https://api.efe.ai' (Browser). */
  readonly endpoint: string;
  /** Per-call deadline in ms. Default 10000. 0 = no SDK-side deadline (rely on caller signal). */
  readonly deadline?: number;
  /** Auth strategy. Default = `{ kind: 'none' }` (DEV ONLY). */
  readonly auth?: AuthConfig;
  /** Retry policy. Default = 3 attempts exponential. (Node only — ignored in browser.) */
  readonly retry?: RetryConfig;
  /** OTel tracer provider. Default = no-op. */
  readonly otel?: OTelConfig;
  /** Prometheus registry. (Node only.) Default = no-op. */
  readonly prometheus?: PrometheusConfig;
  /** User-Agent header. Default = `@efe/sdk/<version>`. */
  readonly userAgent?: string;
  /** Max gRPC message size (bytes). Default 16 MiB. */
  readonly maxReceivedMessageSize?: number;
  /** Custom fetch implementation. (Browser only — useful for MSW / nock-equivalent.) */
  readonly fetch?: typeof globalThis.fetch;
}

/** Resolved options after default fill + validation — all fields present. */
export interface ResolvedClientOptions {
  readonly endpoint: string;
  readonly deadline: number;
  readonly auth: AuthConfig;
  readonly retry: RetryConfig;
  readonly otel: OTelConfig;
  readonly prometheus: PrometheusConfig;
  readonly userAgent: string;
  readonly maxReceivedMessageSize: number;
  readonly fetch: typeof globalThis.fetch | undefined;
}

/** Default retry config : 3 attempts, 100 ms..30 s exponential w/ full jitter. */
export function defaultRetry(): RetryConfig {
  return { maxAttempts: 3, backoff: new ExponentialBackoff() };
}

/**
 * Validate + fill defaults. Throws `ErrInvalidArgument` on bad input.
 *
 * Pin tested by `test_d24_typescript_sdk_options_validation_*`.
 */
export function validateOptions(opts: ClientOptions): ResolvedClientOptions {
  if (!opts.endpoint || typeof opts.endpoint !== 'string') {
    throw new ErrInvalidArgument('endpoint required (string)');
  }
  if (opts.deadline !== undefined && opts.deadline < 0) {
    throw new ErrInvalidArgument('deadline must be ≥ 0');
  }
  if (opts.maxReceivedMessageSize !== undefined && opts.maxReceivedMessageSize <= 0) {
    throw new ErrInvalidArgument('maxReceivedMessageSize must be > 0');
  }
  if (opts.userAgent !== undefined && opts.userAgent === '') {
    throw new ErrInvalidArgument('userAgent must be non-empty (or omit for default)');
  }
  if (opts.retry !== undefined) {
    if (opts.retry.maxAttempts < 1) {
      throw new ErrInvalidArgument('retry.maxAttempts must be ≥ 1');
    }
    if (typeof opts.retry.backoff?.next !== 'function') {
      throw new ErrInvalidArgument('retry.backoff must implement RetryBackoff');
    }
  }
  if (opts.auth !== undefined) {
    validateAuth(opts.auth);
  }
  return {
    endpoint: opts.endpoint,
    deadline: opts.deadline ?? 10_000,
    auth: opts.auth ?? { kind: 'none' },
    retry: opts.retry ?? defaultRetry(),
    otel: opts.otel ?? { kind: 'noop' },
    prometheus: opts.prometheus ?? { kind: 'noop' },
    userAgent: opts.userAgent ?? `@efe/sdk/${Version}`,
    maxReceivedMessageSize: opts.maxReceivedMessageSize ?? 16 * 1024 * 1024,
    fetch: opts.fetch,
  };
}

function validateAuth(auth: AuthConfig): void {
  switch (auth.kind) {
    case 'jwt':
      if (!auth.token) {
        throw new ErrInvalidArgument('auth.token must be non-empty (jwt kind)');
      }
      break;
    case 'mtls':
      if (!auth.certPath || !auth.keyPath || !auth.caPath) {
        throw new ErrInvalidArgument('auth.{certPath,keyPath,caPath} must all be non-empty (mtls kind)');
      }
      break;
    case 'none':
      break;
    default:
      // Exhaustiveness check — TS should never let us reach here.
      throw new ErrInvalidArgument(`unknown auth.kind`);
  }
}
