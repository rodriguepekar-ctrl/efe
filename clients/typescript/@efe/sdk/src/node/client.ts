// Sprint D24 (Φ-B) — `EfeClient` (Node) : the public Node entry point.
//
// Architecture :
//   1. Constructor validates options + builds the interceptor chain
//      [auth, otel, prom, retry] in canonical order.
//   2. Builds the gRPC transport via @connectrpc/connect-node (HTTP/2).
//   3. Creates the typed Connect-RPC client from the EfeService descriptor.
//   4. Lazy dial — first RPC triggers the actual TCP+TLS handshake.
//
// Concurrency : EfeClient instances are SAFE for concurrent use.
//
// Lifecycle :
//   - new EfeClient(...) → ready (no I/O).
//   - classify / healthcheck / classifyStream — concurrent OK.
//   - close() → idempotent ; subsequent calls throw ErrClosed.
//
// Cf. docs/EFE_TYPESCRIPT_SDK_EXTENSION_ARCHITECTURE.md §10.1.

import {
  ConnectError,
  type Interceptor,
  type PromiseClient,
  createPromiseClient,
} from '@connectrpc/connect';

import { EfeError, ErrClosed, ErrInvalidArgument, ErrUnavailable } from '../core/errors.js';
import { fromConnectError } from '../core/from-rpc.js';
import { otelInterceptor } from '../core/otel.js';
import { type ClientOptions, type ResolvedClientOptions, validateOptions } from '../core/options.js';
import type { Verdict } from '../core/verdict.js';
import { deriveZone } from '../core/verdict.js';
import type { CallOptions, ClassifyStreamOptions } from '../core/_types.js';

import {
  EfeService,
  HealthService,
} from '../_generated/efe/v1/efe_connect.js';
import type { ExecuteResponse } from '../_generated/efe/v1/efe_pb.js';
import { HealthStatus } from '../_generated/efe/v1/efe_pb.js';
import { Verdict as VerdictEnum } from '../_generated/efe/v1/types_pb.js';

import { authInterceptor } from './auth.js';
import { retryInterceptor } from './interceptors.js';
import { promInterceptor } from './metrics.js';
import { classifyStreamImpl } from './stream.js';
import { createTransport } from './transport.js';

import type { StreamItem } from '../core/verdict.js';

/**
 * EfeClient — Node entry point of the EFE SDK.
 *
 * @example
 *   const client = new EfeClient({
 *     endpoint: 'staging.efe.ai:50051',
 *     auth: { kind: 'jwt', token: process.env.EFE_JWT_TOKEN! },
 *   });
 *   try {
 *     const verdict = await client.classify('I want a refund');
 *   } finally {
 *     await client.close();
 *   }
 */
export class EfeClient {
  // The `@connectrpc/connect` PromiseClient is generic over the service
  // descriptor — we type it via the literal EfeService type.
  readonly #efeService: PromiseClient<typeof EfeService>;
  readonly #healthService: PromiseClient<typeof HealthService>;
  readonly #opts: ResolvedClientOptions;
  #closed = false;

  constructor(opts: ClientOptions) {
    this.#opts = validateOptions(opts);

    // Canonical interceptor chain order :
    //   1. auth   — token attached BEFORE any downstream layer
    //   2. otel   — span captures auth-decorated request
    //   3. prom   — metrics observe POST-OTel timing (includes overhead)
    //   4. retry  — retries are counted as separate observations
    //                + each retry runs through auth+otel independently
    const interceptors: Interceptor[] = [
      authInterceptor(this.#opts.auth),
      otelInterceptor(this.#opts.otel),
      promInterceptor(this.#opts.prometheus),
      retryInterceptor(this.#opts.retry),
    ];

    const transport = createTransport(this.#opts, interceptors);
    // **Audit fix M10-A1** — the placeholder `EfeService` / `HealthService`
    // consts in `_generated/efe/v1/efe_connect.ts` are HAND-CODED stubs
    // for D24 day 1. The real `buf generate` output via `@bufbuild/protobuf`
    // produces full `ServiceType` objects ; until then we cast at the
    // `createPromiseClient` boundary so the SDK compiles end-to-end.
    // The cast becomes a no-op identity once `pnpm gen:proto` runs.
    // Pin tested by `test_d24_typescript_sdk_audit_fix_a1_create_promise_client_cast`.
    /* eslint-disable @typescript-eslint/no-explicit-any */
    this.#efeService = createPromiseClient(EfeService as any, transport) as PromiseClient<typeof EfeService>;
    this.#healthService = createPromiseClient(HealthService as any, transport) as PromiseClient<typeof HealthService>;
    /* eslint-enable @typescript-eslint/no-explicit-any */
  }

  /**
   * Send one classification request to the EFE server.
   *
   * @throws ErrInvalidArgument | ErrUnauthenticated | ErrTimeout |
   *         ErrUnavailable | ErrRateLimited | ErrCatalogInvalid |
   *         ErrInternal | ErrClosed
   *
   * @example
   *   const verdict = await client.classify('refund please', {
   *     timeout: 5_000,
   *   });
   *   if (verdict.zone === 'trusted') { ... }
   */
  async classify(text: string, opts: CallOptions = {}): Promise<Verdict> {
    if (this.#closed) {
      throw new ErrClosed('cannot issue RPC after close()');
    }
    if (text.trim().length === 0) {
      throw new ErrInvalidArgument('empty text');
    }
    const start = Date.now();
    try {
      // **Audit fix M10-A2** — `exactOptionalPropertyTypes: true` makes
      // `{ signal: opts.signal }` invalid when `opts.signal` is `undefined`
      // (the property type `signal?: AbortSignal` rejects explicit
      // undefined). Use conditional spread to preserve absence semantics.
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

  /**
   * Probe the server's gRPC standard Health endpoint.
   *
   * Returns void (resolves) when the server reports SERVING.
   * Throws on NOT_SERVING / unreachable / timeout.
   *
   * @example
   *   try {
   *     await client.healthcheck();
   *     console.log('OK');
   *   } catch (err) {
   *     console.log('Unhealthy:', err);
   *   }
   */
  async healthcheck(opts: CallOptions = {}): Promise<void> {
    if (this.#closed) {
      throw new ErrClosed('cannot issue RPC after close()');
    }
    try {
      // **Audit fix M10-A2** — see classify() for rationale.
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

  /**
   * Concurrent batch classification with strict input-order index
   * preservation.
   *
   * Architecture : single dispatcher reads from `texts` and assigns
   * monotonic indices ; a worker pool of `concurrency` (default 8)
   * goroutines pulls from a fan-out channel and emits StreamItems
   * on the output channel. The dispatcher pattern guarantees that
   * `index` reflects the position of the text in the input stream.
   *
   * @example
   *   for await (const item of client.classifyStream(['a', 'b', 'c'])) {
   *     if (item.error) console.error(item.index, item.error);
   *     else console.log(item.index, item.verdict?.label);
   *   }
   */
  classifyStream(
    texts: AsyncIterable<string> | Iterable<string>,
    opts: ClassifyStreamOptions = {},
  ): AsyncIterable<StreamItem> {
    return classifyStreamImpl(this, texts, opts);
  }

  /**
   * Release the underlying gRPC connection. Idempotent — subsequent
   * calls are no-ops, but classify/healthcheck calls throw ErrClosed.
   */
  async close(): Promise<void> {
    this.#closed = true;
    // Connect-Node currently doesn't expose a transport.dispose() ;
    // the underlying `@grpc/grpc-js` channel is closed implicitly
    // when no more references to the PromiseClient exist.
  }

  /** True after close() has been called. */
  get closed(): boolean {
    return this.#closed;
  }

  /** Redacted summary, safe for logs (NEVER includes JWT / mTLS material). */
  toString(): string {
    return `EfeClient{endpoint=${this.#opts.endpoint} closed=${this.#closed} deadline=${this.#opts.deadline}ms}`;
  }
}

/**
 * Project a wire-shape `ExecuteResponse` into the public `Verdict`.
 *
 * Field mapping :
 *   - label          ← output.text (proto-side label string)
 *   - confidence     ← confidence
 *   - rounds         ← roundsExecuted
 *   - alignmentDelta ← alignmentDelta
 *   - swarmId        ← swarmId
 *   - requestId      ← requestId
 *   - reject         ← derived from Verdict enum (DIVERGENCE / MODE_COLLAPSE)
 *   - zone           ← derived from Verdict enum + confidence threshold
 *   - catalog        ← server-side metadata (empty for D24 baseline)
 *   - elapsedMs      ← computed client-side
 */
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
      // Server timed out without converging — surface as warn.
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
