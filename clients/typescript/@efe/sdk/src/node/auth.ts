// Sprint D24 (Φ-B) — Auth interceptor (Node).
//
// JWT Bearer is attached as the `authorization` request header.
// mTLS auth happens at TLS handshake time (cf. tls.ts), so the
// interceptor is a no-op for mtls.
//
// Cf. docs/EFE_TYPESCRIPT_SDK_EXTENSION_ARCHITECTURE.md §15.

import type { Interceptor } from '@connectrpc/connect';

import { buildBearerHeader } from '../core/auth.js';
import type { AuthConfig } from '../core/options.js';

/**
 * Returns a Connect-RPC interceptor that injects the canonical auth
 * header(s) on every outgoing RPC.
 *
 *   - kind === 'jwt' : sets `authorization: Bearer <token>`.
 *   - kind === 'mtls' / 'none' : no-op (auth already at TLS layer or absent).
 *
 * The token is captured at construction time + closed over ; never
 * logged, never exposed via toString.
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
