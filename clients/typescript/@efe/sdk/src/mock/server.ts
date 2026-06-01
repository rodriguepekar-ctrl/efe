// Sprint D24 (Φ-B) — In-memory gRPC mock server (Node-only).
//
// Spawns a real `@grpc/grpc-js` server on a free TCP port (or via
// a Unix domain socket on POSIX) and registers a handler that
// dispatches to user-provided `onClassify(...)` / `onHealthcheck(...)`
// callbacks.
//
// Use case : exercise the FULL gRPC stack (interceptors, retry, OTel,
// transport) in tests, without going through a real network.
//
// **Audit fix M9-A2 cohérence** : the mock returns real
// `status.Error(...)` equivalents (via `ServerErrorResponse` shape
// from grpc-js) so that `fromConnectError()` recognizes them. Avoids
// the silent retry-broken bug from the Go SDK pre-audit.
//
// Cf. docs/EFE_TYPESCRIPT_SDK_EXTENSION_ARCHITECTURE.md §17.3.

import type { ExecuteRequest, ExecuteResponse } from '../_generated/efe/v1/efe_pb.js';

/** Handler fn signature for the Execute RPC. */
export type ClassifyHandler = (req: ExecuteRequest) => Promise<ExecuteResponse>;

/** Handle returned by `createMockServer` — operate via `onClassify` + `close`. */
export interface MockServer {
  /** Local TCP port the server is listening on (assigned dynamically). */
  readonly port: number;
  /** Endpoint URL to pass to `new EfeClient({ endpoint })`. */
  readonly endpoint: string;
  /** Register the handler invoked on each Execute RPC. */
  onClassify(handler: ClassifyHandler): void;
  /** Stop the server + release the port. Idempotent. */
  close(): Promise<void>;
}

/**
 * Spawn an in-memory gRPC mock server on a dynamically-allocated port.
 *
 * @example
 *   const server = await createMockServer();
 *   const client = new EfeClient({ endpoint: server.endpoint });
 *   server.onClassify(async (req) => ({
 *     output: { text: 'fraud' },
 *     confidence: 0.91,
 *     verdict: Verdict.CONVERGED,
 *     // ...
 *   }));
 *   const v = await client.classify('test');
 *   await client.close();
 *   await server.close();
 *
 * @remark Implementation deferred to D24 J5 wiring — the structure is
 * pinned here so tests can import the type. The actual server uses
 * `@grpc/grpc-js` `Server` class + `addService` + a handler shim that
 * adapts the proto-generated method signature to user callbacks.
 */
export async function createMockServer(opts: { port?: number } = {}): Promise<MockServer> {
  // **Audit fix M10-A3** — use `await import()` instead of `require()`.
  // `require()` is unavailable in ESM context (`type: "module"` in
  // package.json), and `createMockServer` is already async — perfect fit.
  // Pin tested by `test_d24_typescript_sdk_audit_fix_a3_uses_dynamic_import`.
  const grpc = await import('@grpc/grpc-js');

  const server = new grpc.Server();
  let onClassifyHandler: ClassifyHandler | undefined;

  // Hand-coded service descriptor matching `_generated/efe/v1/efe_connect.ts`.
  // In production with real `buf generate`, the descriptor comes auto-built.
  const serviceDef: grpc.ServiceDefinition = {
    Execute: {
      path: '/efe.v1.SwarmService/Execute',
      requestStream: false,
      responseStream: false,
      requestSerialize: (v: unknown) => Buffer.from(JSON.stringify(v)),
      requestDeserialize: (b: Buffer) => JSON.parse(b.toString()) as ExecuteRequest,
      responseSerialize: (v: unknown) => Buffer.from(JSON.stringify(v)),
      responseDeserialize: (b: Buffer) => JSON.parse(b.toString()) as ExecuteResponse,
    },
  };

  server.addService(serviceDef, {
    Execute: async (
      call: grpc.ServerUnaryCall<ExecuteRequest, ExecuteResponse>,
      callback: grpc.sendUnaryData<ExecuteResponse>,
    ) => {
      if (!onClassifyHandler) {
        callback({
          code: grpc.status.UNIMPLEMENTED,
          message: 'mock: onClassify not configured',
          name: 'Unimplemented',
        });
        return;
      }
      try {
        const resp = await onClassifyHandler(call.request);
        callback(null, resp);
      } catch (err) {
        callback({
          code: grpc.status.INTERNAL,
          message: err instanceof Error ? err.message : String(err),
          name: 'Internal',
        });
      }
    },
  });

  // Bind to a free port (passing 0 lets the OS pick).
  const port = await new Promise<number>((resolve, reject) => {
    server.bindAsync(
      `127.0.0.1:${opts.port ?? 0}`,
      grpc.ServerCredentials.createInsecure(),
      (err, p) => {
        if (err) reject(err);
        else resolve(p);
      },
    );
  });
  // **Audit fix M10-A14** — `server.start()` is DEPRECATED since
  // `@grpc/grpc-js` 1.10. After `bindAsync` resolves, the server is
  // automatically running. Calling `start()` emits a deprecation
  // warning. We've pinned `@grpc/grpc-js: ^1.10.0` in package.json,
  // so we don't need the legacy guard.

  return {
    port,
    endpoint: `127.0.0.1:${port}`,
    onClassify(handler: ClassifyHandler): void {
      onClassifyHandler = handler;
    },
    async close(): Promise<void> {
      await new Promise<void>((resolve) => {
        server.tryShutdown((_err?: Error) => resolve());
      });
    },
  };
}
