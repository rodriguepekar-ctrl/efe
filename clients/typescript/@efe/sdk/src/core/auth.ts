// Sprint D24 (Φ-B) — Auth header builder (universal).
//
// JWT Bearer is universal (works in Node + Browser). mTLS is Node-only
// (browser TLS Client Cert API has been deprecated ; the OS / user
// profile holds the cert).
//
// The builder is a pure function so it's trivially testable.

import { ErrInvalidArgument } from './errors.js';

/**
 * Build the canonical `Authorization: Bearer <jwt>` header value.
 *
 * Throws `ErrInvalidArgument` on empty token (almost always a bug —
 * forgot to read EFE_JWT_TOKEN from env).
 */
export function buildBearerHeader(jwt: string): string {
  if (!jwt) {
    throw new ErrInvalidArgument('empty JWT — did you forget EFE_JWT_TOKEN env var?');
  }
  return `Bearer ${jwt}`;
}
