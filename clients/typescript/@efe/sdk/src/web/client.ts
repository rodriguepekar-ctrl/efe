// Sprint D24 (Φ-B) — `EfeWebClient` : the browser entry point.
//
// **SUBSET API** : exposes only IDEMPOTENT RPCs (Classify + Healthcheck).
// Write RPCs (RegisterCatalog, EvolveCatalog, DeleteCatalog) require
// admin scope and SHOULD be performed server-side via a BFF — not
// directly from a browser context. The class doesn't even define those
// methods, so TypeScript catches misuse at compile time.
//
// Architecture :
//   1. Constructor validates options + builds the interceptor chain
//      [auth, otel] (NO retry / NO prom in browser).
//   2. Builds the gRPC-Web transport via @connectrpc/connect-web.
//   3. Creates the typed Connect-RPC client.
//
// Browser-specific limitations enforced by design :
//   - No mTLS (browser TLS Client Cert API deprecated).
//   - No streaming RPCs (gRPC-Web protocol limit).
//   - No retry by default (caller's responsibility).
//   - No Prometheus metrics (no scrape endpoint).
//
// Cf. docs/EFE_TYPESCRIPT_SDK_EXTENSION_ARCHITECTURE.md §10.2.

import {
  ConnectError,
  type Interceptor,
  type PromiseClient,
  createPromiseClient,
} from '@connectrpc/connect';

import { EfeError, ErrClosed, ErrInvalidArgument, ErrUnavailable } from '../core/errors.js';
import { fromConnectError } from '../core/from-rpc.js';
import { otelInterceptor } from '../core/otel.js';
import {
  type ClientOptions,
  type ResolvedClientOptions,
  validateOptions,
} from '../core/options.js';
import type { Verdict } from '../core/verdict.js';
import { deriveZone } from '../core/verdict.js';
import type { CallOptions } from '../core/_types.js';

import {
  EfeService,
  HealthService,
} from '../_generated/efe/v1/efe_connect.js';
import type { ExecuteResponse } from '../_generated/efe/v1/efe_pb.js';
import { HealthStatus } from '../_generated/efe/v1/efe_pb.js';
import { Verdict as VerdictEnum } from '../_generated/efe/v1/types_pb.js';

import { authInterceptor } from './auth.js';
import { createTransport } from './transport.js';

/**
 * EfeWebClient — browser entry point of the EFE SDK.
 *
 * @example
 *   const client = new EfeWebClient({
 *     endpoint: 'https://api.efe.ai',
 *     auth: { kind: 'jwt', token: localStorage.getItem('jwt')! },
 *   });
 *   const verdict = await client.classify('refund please');
 *
 * @remark mTLS is NOT supported (browser API limitation) — passing
 * `auth: { kind: 'mtls', ... }` will validate at construction but the
 * cert paths are ignored by the browser fetch.
 */
export class EfeWebClient {
  readonly #efeService: PromiseClient<typeof EfeService>;
  readonly #healthService: PromiseClient<typeof HealthService>;
  readonly #opts: ResolvedClientOptions;
  #closed = false;

  constructor(opts: ClientOptions) {
    this.#opts = validateOptions(opts);

    // No retry / no prom in browser — only auth + otel.
    const interceptors: Interceptor[] = [
      authInterceptor(this.#opts.auth),
      otelInterceptor(this.#opts.otel),
    ];

    const transport = createTransport(this.#opts, interceptors);
    // **Audit fix M10-A1** — see node/client.ts for rationale.
    /* eslint-disable @typescript-eslint/no-explicit-any */
    this.#efeService = createPromiseClient(EfeService as any, transport) as PromiseClient<typeof EfeService>;
    this.#healthService = createPromiseClient(HealthService as any, transport) as PromiseClient<typeof HealthService>;
    /* eslint-enable @typescript-eslint/no-explicit-any */
  }

  /** Send one classification request. See `EfeClient.classify` for details. */
  async classify(text: string, opts: CallOptions = {}): Promise<Verdict> {
    if (this.#closed) {
      throw new ErrClosed('cannot issue RPC after close()');
    }
    if (text.trim().length === 0) {
      throw new ErrInvalidArgument('empty text');
    }
    const start = Date.now();
    try {
      // **Audit fix M10-A2** — conditional spread for exactOptionalPropertyTypes.
      const resp = await this.#efeService.execute(
        { rawPrompt: text },
        {
          ...(opts.signal !== undefined ? { signal: opts.signal } : {}),
          timeoutMs: opts.timeout ?? this.#opts.deadline,
        },
      );
      return verdictFromExecuteResponse(resp, Date.now() - start);
    } catch (err) {
      if (err instanceof ConnectError) {
        const translated = fromConnectError(err);
        throw translated instanceof EfeError ? translated : err;
      }
      throw err;
    }
  }

  /** Probe the server's gRPC standard Health endpoint. */
  async healthcheck(opts: CallOptions = {}): Promise<void> {
    if (this.#closed) {
      throw new ErrClosed('cannot issue RPC after close()');
    }
    try {
      // **Audit fix M10-A2** — conditional spread for exactOptionalPropertyTypes.
      const resp = await this.#healthService.check(
        { service: '' },
        {
          ...(opts.signal !== undefined ? { signal: opts.signal } : {}),
          timeoutMs: opts.timeout ?? this.#opts.deadline,
        },
      );
      if (resp.status !== HealthStatus.SERVING) {
        throw new ErrUnavailable(`server reports status=${resp.status}`);
      }
    } catch (err) {
      if (err instanceof ConnectError) {
        const translated = fromConnectError(err);
        throw translated instanceof EfeError ? translated : err;
      }
      throw err;
    }
  }

  /** Mark the client as closed. Browser fetch transport has no persistent connection. */
  async close(): Promise<void> {
    this.#closed = true;
  }

  get closed(): boolean {
    return this.#closed;
  }

  toString(): string {
    return `EfeWebClient{endpoint=${this.#opts.endpoint} closed=${this.#closed}}`;
  }
}

function verdictFromExecuteResponse(resp: ExecuteResponse, elapsedMs: number): Verdict {
  let reject = false;
  let zone: string;
  switch (resp.verdict) {
    case VerdictEnum.DIVERGENCE:
    case VerdictEnum.MODE_COLLAPSE:
      reject = true;
      zone = 'reject';
      break;
    case VerdictEnum.CONVERGED:
      zone = deriveZone(resp.confidence);
      break;
    case VerdictEnum.CONTINUE:
      zone = 'warn';
      break;
    default:
      zone = 'warn';
  }
  return {
    label: resp.output?.text ?? '',
    confidence: resp.confidence,
    reject,
    rounds: resp.roundsExecuted,
    alignmentDelta: resp.alignmentDelta,
    catalog: '',
    zone,
    swarmId: resp.swarmId,
    requestId: resp.requestId,
    elapsedMs,
  };
}
