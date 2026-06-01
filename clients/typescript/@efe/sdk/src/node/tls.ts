// Sprint D24 (Φ-B) — mTLS configuration helper (Node-only).
//
// Reads X.509 cert + private key + CA pool from disk and produces the
// `nodeOptions` shape consumed by `@connectrpc/connect-node` /
// `@grpc/grpc-js`. Enforces TLS 1.3 minimum (matches Lettre E D22).
//
// Trust boundary :
//   - All file reads happen at construction time. Errors surface at
//     `new EfeClient(...)` (vs. at first RPC), so the operator
//     catches bad paths early.
//   - The private key is held in memory in a Buffer ; we don't
//     redact it from logs because the logger never sees the buffer
//     (only the resolved options object, whose toString is overridden).
//
// Cf. docs/EFE_TYPESCRIPT_SDK_EXTENSION_ARCHITECTURE.md §15.2.

import { readFileSync } from 'node:fs';

import { ErrInvalidArgument } from '../core/errors.js';
import type { AuthConfig } from '../core/options.js';

// **Audit fix M10-A9** — pre-fix dragged `import { constants as tlsConstants } from 'node:tls'`
// for a single dead `_TLS_DEFAULT_OPTIONS` export. Removed for cleanliness.

/** Resolved Node mTLS material — passed to grpc-js / connect-node transport. */
export interface NodeTlsCredentials {
  readonly cert?: Buffer;
  readonly key?: Buffer;
  readonly ca?: Buffer;
  readonly minVersion?: 'TLSv1.2' | 'TLSv1.3';
  readonly rejectUnauthorized?: boolean;
}

/**
 * Resolve the TLS credentials for an EfeClient based on its auth config.
 *
 *   - kind === 'mtls' → load cert/key/CA from disk + enforce TLS 1.3.
 *   - kind === 'jwt' or 'none' → no client cert ; system trust pool used.
 *
 * Throws `ErrInvalidArgument` on file-read errors, with the offending
 * path included in the message (no secrets leak — paths only).
 */
export function resolveTLSCredentials(auth: AuthConfig): NodeTlsCredentials {
  if (auth.kind !== 'mtls') {
    return {};
  }
  let cert: Buffer;
  let key: Buffer;
  let ca: Buffer;
  try {
    cert = readFileSync(auth.certPath);
  } catch (err) {
    throw new ErrInvalidArgument(`mTLS: cannot read cert ${auth.certPath}`, {
      cause: err,
    });
  }
  try {
    key = readFileSync(auth.keyPath);
  } catch (err) {
    throw new ErrInvalidArgument(`mTLS: cannot read key ${auth.keyPath}`, {
      cause: err,
    });
  }
  try {
    ca = readFileSync(auth.caPath);
  } catch (err) {
    throw new ErrInvalidArgument(`mTLS: cannot read CA ${auth.caPath}`, {
      cause: err,
    });
  }
  // Sanity check : refuse empty files.
  if (cert.length === 0 || key.length === 0 || ca.length === 0) {
    throw new ErrInvalidArgument('mTLS: one of cert/key/ca is empty');
  }
  return {
    cert,
    key,
    ca,
    minVersion: 'TLSv1.3',
    rejectUnauthorized: true,
  };
}

/**
 * Helper : build a `NodeTlsCredentials` directly from in-memory PEM
 * strings (useful for tests that don't want disk I/O, or for ops who
 * read certs from a Vault secret manager).
 */
export function withMTLS(opts: {
  readonly certPem: string;
  readonly keyPem: string;
  readonly caPem: string;
}): NodeTlsCredentials {
  if (!opts.certPem || !opts.keyPem || !opts.caPem) {
    throw new ErrInvalidArgument('withMTLS: certPem/keyPem/caPem must all be non-empty');
  }
  return {
    cert: Buffer.from(opts.certPem),
    key: Buffer.from(opts.keyPem),
    ca: Buffer.from(opts.caPem),
    minVersion: 'TLSv1.3',
    rejectUnauthorized: true,
  };
}

