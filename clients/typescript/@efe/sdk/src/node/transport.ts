// Sprint D24 (Φ-B) — Node transport wrapper.
//
// Wraps `createGrpcTransport` from `@connectrpc/connect-node` with
// the canonical defaults (HTTP/2, mTLS resolution, max-recv-size,
// user-agent).
//
// Note : `@connectrpc/connect-node` builds on `@grpc/grpc-js` under
// the hood — that's our HTTP/2 client.

import type { Interceptor, Transport } from '@connectrpc/connect';
import { createGrpcTransport } from '@connectrpc/connect-node';

import type { ResolvedClientOptions } from '../core/options.js';
import { resolveTLSCredentials } from './tls.js';

/**
 * Build the Node Connect-RPC transport from resolved client options.
 *
 *   - HTTP/2 (gRPC native).
 *   - mTLS material loaded from disk if auth.kind === 'mtls'.
 *   - max-recv-size = `opts.maxReceivedMessageSize`.
 *   - User-Agent set via custom interceptor (connect-node currently
 *     ignores it on transport options, so we inject as header).
 *   - Interceptor chain : [auth, otel, prom, retry] in that order.
 */
export function createTransport(
  opts: ResolvedClientOptions,
  interceptors: Interceptor[],
): Transport {
  const tls = resolveTLSCredentials(opts.auth);
  return createGrpcTransport({
    baseUrl: normalizeEndpoint(opts.endpoint),
    httpVersion: '2',
    interceptors,
    nodeOptions: tls.cert
      ? {
          cert: tls.cert,
          key: tls.key,
          ca: tls.ca,
          minVersion: tls.minVersion,
          rejectUnauthorized: tls.rejectUnauthorized,
        }
      : undefined,
    readMaxBytes: opts.maxReceivedMessageSize,
  });
}

/**
 * Normalize endpoint to the URL form expected by Connect-RPC.
 *
 *   - "host:port"            → "https://host:port"  (assume TLS)
 *   - "grpc://host:port"     → "http://host:port"   (h2c — DEV ONLY)
 *   - "https://..."          → unchanged
 *   - "http://..."           → unchanged (h2c)
 *   - "passthrough:bufconn"  → "http://passthrough.invalid"  (mock test)
 */
export function normalizeEndpoint(endpoint: string): string {
  if (endpoint.startsWith('https://') || endpoint.startsWith('http://')) {
    return endpoint;
  }
  if (endpoint.startsWith('grpc://')) {
    return `http://${endpoint.slice('grpc://'.length)}`;
  }
  if (endpoint.startsWith('passthrough:')) {
    // Used by the in-memory mock server. Connect-node will route via
    // a custom dispatcher injected at transport-options level (when
    // we wire bufconn-equivalent in tests). For the URL parser, any
    // valid HTTP origin works.
    return 'http://127.0.0.1:0';
  }
  return `https://${endpoint}`;
}
