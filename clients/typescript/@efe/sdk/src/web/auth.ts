// Sprint D24 (Φ-B) — Auth interceptor (Browser).
//
// JWT Bearer only — mTLS impossible in browser context (no JS API
// to set client cert on fetch).
//
// Cf. docs/EFE_TYPESCRIPT_SDK_EXTENSION_ARCHITECTURE.md §15.3.

import type { Interceptor } from '@connectrpc/connect';

import { buildBearerHeader } from '../core/auth.js';
import type { AuthConfig } from '../core/options.js';

/**
 * Browser auth interceptor. Only the `jwt` kind has effect ; any
 * other kind is a no-op.
 *
 * The browser's TLS handshake is handled by the OS / fetch's native
 * implementation ; we don't have access to client cert APIs.
 */
export function authInterceptor(cfg: AuthConfig): Interceptor {
  if (cfg.kind !== 'jwt') {
    return (next) => async (req) => next(req);
  }
  const header = buildBearerHeader(cfg.token);
  return (next) => async (req) => {
    req.header.set('authorization', header);
    return next(req);
  };
}

/**
 * Helper to build a fetch wrapper that injects auth + custom CORS-friendly
 * headers. Useful when the consumer wants to test with MSW or similar
 * fetch-mocking libraries.
 *
 * @example
 *   const client = new EfeClient({
 *     endpoint: 'https://api.efe.ai',
 *     fetch: withFetchAuth(jwt),
 *   });
 */
export function withFetchAuth(jwt: string, baseFetch: typeof fetch = globalThis.fetch): typeof fetch {
  if (!jwt) {
    throw new Error('withFetchAuth: empty JWT');
  }
  return async (input, init) => {
    const headers = new Headers(init?.headers);
    headers.set('authorization', `Bearer ${jwt}`);
    return baseFetch(input, { ...init, headers });
  };
}
