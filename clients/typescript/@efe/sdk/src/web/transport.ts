// Sprint D24 (Φ-B) — Browser transport (gRPC-Web).
//
// Wraps `createGrpcWebTransport` from `@connectrpc/connect-web`.
// Browser uses HTTP/1.1 (no HTTP/2 client API in browsers) — the
// EFE server must expose port :50052 with a gRPC-Web translator
// (e.g. Envoy filter `envoy.filters.http.grpc_web`).

import type { Interceptor, Transport } from '@connectrpc/connect';
import { createGrpcWebTransport } from '@connectrpc/connect-web';

import type { ResolvedClientOptions } from '../core/options.js';

/**
 * Build the browser Connect-RPC transport.
 *
 *   - gRPC-Web protocol over HTTP/1.1 (CORS-aware).
 *   - Custom fetch override (if provided) — used for MSW-style mocking.
 *   - Interceptor chain : [auth, otel] in that order. NO retry / NO
 *     prom in browser (caller responsibility for retries via TanStack
 *     Query / SWR ; metrics via OTel only).
 */
export function createTransport(
  opts: ResolvedClientOptions,
  interceptors: Interceptor[],
): Transport {
  return createGrpcWebTransport({
    baseUrl: normalizeEndpoint(opts.endpoint),
    interceptors,
    fetch: opts.fetch ?? globalThis.fetch.bind(globalThis),
  });
}

/**
 * Normalize endpoint for browser use.
 *
 *   - already-prefixed http(s) : passthrough
 *   - else                     : assume https://
 *
 * Browser doesn't support h2c without HTTPS, so `grpc://` and `http://`
 * are forwarded only for tests against localhost.
 */
export function normalizeEndpoint(endpoint: string): string {
  if (endpoint.startsWith('https://') || endpoint.startsWith('http://')) {
    return endpoint;
  }
  return `https://${endpoint}`;
}
